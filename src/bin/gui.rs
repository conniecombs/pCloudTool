use iced::advanced::subscription::{self, Event, Hasher, Recipe};
use iced::futures::stream::{self, BoxStream, StreamExt};
use iced::keyboard::{self, Key, Modifiers};
use iced::time::Instant;
use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, mouse_area, opaque,
    progress_bar, row, scrollable, slider, stack, text, text_input, vertical_rule, Space,
};
use iced::{alignment, Alignment, Background, Color, Element, Length, Subscription, Task, Theme};

use pcloud_rust::{FileItem, PCloudClient, Region};
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Context menu state for right-click operations
#[derive(Debug, Clone)]
struct ContextMenu {
    item: Option<FileItem>,
}

/// Double-click detection
const DOUBLE_CLICK_THRESHOLD: Duration = Duration::from_millis(400);

pub fn main() -> iced::Result {
    iced::application("pCloud Fast Transfer", PCloudGui::update, PCloudGui::view)
        .theme(PCloudGui::theme)
        .subscription(PCloudGui::subscription)
        .run_with(PCloudGui::new)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppState {
    Login,
    Dashboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SortBy {
    #[default]
    Name,
    Size,
    Date,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

#[derive(Debug, Clone)]
struct TransferProgress {
    total_files: usize,
    finished_files: usize,
    total_bytes: u64,
    transferred_bytes: u64,
    start_time: Instant,
    current_speed: f64,
    current_file: Option<String>,
    current_file_size: u64,
    current_file_progress: u64,
}

#[derive(Debug, Clone)]
enum Status {
    Idle,
    ReadyToUpload(usize, u64),
    Working(String),
    Transferring(TransferProgress),
    Success(String),
    Error(String),
}

struct PCloudGui {
    state: AppState,
    status: Status,
    username: String,
    password: String,
    client: PCloudClient,
    current_path: String,
    // FIX: Wrapped in Arc to prevent expensive clones
    file_list: Arc<Vec<FileItem>>,
    selected_item: Option<FileItem>,
    concurrency_setting: usize,
    active_concurrency: usize,
    staged_transfer: Option<TransferType>,
    active_transfer: Option<TransferType>,
    bytes_progress: Arc<AtomicU64>,
    sort_by: SortBy,
    sort_order: SortOrder,
    search_filter: String,
    // Usability improvements
    context_menu: Option<ContextMenu>,
    last_click_time: Option<std::time::Instant>,
    last_clicked_item: Option<String>,
    create_folder_state: CreateFolderState,
}

#[derive(Debug, Clone)]
enum TransferType {
    Upload(u64, Vec<(PathBuf, String)>, u64),
    Download(u64, Vec<(String, String)>, u64),
}

struct TransferRecipe {
    id: u64,
    mode: TransferMode,
    client: PCloudClient,
    concurrency: usize,
    total_files: usize,
    total_bytes: u64,
    bytes_progress: Arc<AtomicU64>,
}

#[derive(Clone)]
enum TransferMode {
    Upload(Vec<(PathBuf, String)>),
    Download(Vec<(String, String)>),
}

impl Recipe for TransferRecipe {
    type Output = Message;

    fn hash(&self, state: &mut Hasher) {
        use std::any::TypeId;
        TypeId::of::<Self>().hash(state);
        self.id.hash(state);
        self.concurrency.hash(state);
    }

    fn stream(self: Box<Self>, _input: BoxStream<Event>) -> BoxStream<Message> {
        let client = self.client.clone();
        let mode = self.mode.clone();
        let concurrency = self.concurrency;
        let t_files = self.total_files;
        let t_bytes = self.total_bytes;
        let bytes_progress = self.bytes_progress.clone();

        match mode {
            TransferMode::Upload(tasks) => {
                // Channel to receive progress updates and file completions
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

                let transfer_stream = async_stream::stream! {
                    yield Message::TransferStarted(t_files, t_bytes);

                    // Spawn the actual transfer work
                    let tx_clone = tx.clone();
                    let bytes_progress_clone = bytes_progress.clone();

                    let transfer_handle = tokio::spawn(async move {
                        let uploads = stream::iter(tasks)
                            .map(|(local, remote)| {
                                let c = client.clone();
                                let bp = bytes_progress_clone.clone();
                                let tx_inner = tx_clone.clone();
                                async move {
                                    let size = std::fs::metadata(&local).map(|m| m.len()).unwrap_or(0);
                                    let filename = local.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("unknown")
                                        .to_string();

                                    // Notify file start
                                    let _ = tx_inner.send(Message::TransferFileStarted(filename, size));

                                    let result = c
                                        .upload_file_with_progress(
                                            local.to_str().unwrap_or_default(),
                                            &remote,
                                            move |bytes| {
                                                bp.fetch_add(bytes as u64, Ordering::Relaxed);
                                            }
                                        )
                                        .await;
                                    let _ = tx_inner.send(Message::TransferItemFinished(size, result.is_ok()));
                                }
                            })
                            .buffer_unordered(concurrency);

                        uploads.collect::<Vec<_>>().await;
                    });

                    // Emit progress updates every 100ms while transfer is running
                    let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
                    let mut files_done = 0usize;

                    loop {
                        tokio::select! {
                            biased;  // Prioritize interval for consistent progress updates
                            _ = interval.tick() => {
                                let bytes = bytes_progress.load(Ordering::Relaxed);
                                yield Message::TransferBytesProgress(bytes);
                            }
                            msg = rx.recv() => {
                                match msg {
                                    Some(Message::TransferFileStarted(name, size)) => {
                                        yield Message::TransferFileStarted(name, size);
                                    }
                                    Some(Message::TransferItemFinished(size, ok)) => {
                                        files_done += 1;
                                        // Emit progress update with file completion
                                        let bytes = bytes_progress.load(Ordering::Relaxed);
                                        yield Message::TransferBytesProgress(bytes);
                                        yield Message::TransferItemFinished(size, ok);
                                        if files_done >= t_files {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    let _ = transfer_handle.await;
                    // Final progress update before completion
                    let final_bytes = bytes_progress.load(Ordering::Relaxed);
                    yield Message::TransferBytesProgress(final_bytes);
                    yield Message::TransferCompleted;
                };

                Box::pin(transfer_stream)
            }
            TransferMode::Download(tasks) => {
                // Channel to receive progress updates and file completions
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

                let transfer_stream = async_stream::stream! {
                    yield Message::TransferStarted(t_files, t_bytes);

                    let tx_clone = tx.clone();
                    let bytes_progress_clone = bytes_progress.clone();

                    let transfer_handle = tokio::spawn(async move {
                        let downloads = stream::iter(tasks)
                            .map(|(remote, local)| {
                                let c = client.clone();
                                let bp = bytes_progress_clone.clone();
                                let tx_inner = tx_clone.clone();
                                async move {
                                    let filename = remote.split('/').next_back().unwrap_or("unknown").to_string();

                                    // Notify file start (size unknown for downloads until complete)
                                    let _ = tx_inner.send(Message::TransferFileStarted(filename, 0));

                                    let result = c.download_file(&remote, &local).await;
                                    let size = if result.is_ok() {
                                        let s = std::fs::metadata(&local).map(|m| m.len()).unwrap_or(0);
                                        bp.fetch_add(s, Ordering::Relaxed);
                                        s
                                    } else {
                                        0
                                    };
                                    let _ = tx_inner.send(Message::TransferItemFinished(size, result.is_ok()));
                                }
                            })
                            .buffer_unordered(concurrency);

                        downloads.collect::<Vec<_>>().await;
                    });

                    // Emit progress updates every 100ms while transfer is running
                    let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
                    let mut files_done = 0usize;

                    loop {
                        tokio::select! {
                            biased;  // Prioritize interval for consistent progress updates
                            _ = interval.tick() => {
                                let bytes = bytes_progress.load(Ordering::Relaxed);
                                yield Message::TransferBytesProgress(bytes);
                            }
                            msg = rx.recv() => {
                                match msg {
                                    Some(Message::TransferFileStarted(name, size)) => {
                                        yield Message::TransferFileStarted(name, size);
                                    }
                                    Some(Message::TransferItemFinished(size, ok)) => {
                                        files_done += 1;
                                        // Emit progress update with file completion
                                        let bytes = bytes_progress.load(Ordering::Relaxed);
                                        yield Message::TransferBytesProgress(bytes);
                                        yield Message::TransferItemFinished(size, ok);
                                        if files_done >= t_files {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    let _ = transfer_handle.await;
                    // Final progress update before completion
                    let final_bytes = bytes_progress.load(Ordering::Relaxed);
                    yield Message::TransferBytesProgress(final_bytes);
                    yield Message::TransferCompleted;
                };

                Box::pin(transfer_stream)
            }
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    UsernameChanged(String),
    PasswordChanged(String),
    LoginPressed,
    LoginResult(Result<String, String>),
    LogoutPressed,
    RefreshList,
    // FIX: Using Arc<Vec>
    ListResult(Result<Arc<Vec<FileItem>>, String>),
    NavigateTo(String),
    NavigateUp,
    NavigateToPath(String),
    SortByChanged(SortBy),
    SearchFilterChanged(String),
    ClearSearchFilter,
    ItemClicked(FileItem), // For single-click selection and double-click navigation
    ConcurrencyChanged(f64),
    UploadFilePressed,
    UploadFolderPressed,
    UploadSelected(Option<Vec<PathBuf>>),
    UploadFolderSelected(Option<PathBuf>),
    StartTransferPressed,
    CancelTransferPressed,
    DownloadPressed,
    DownloadDestSelected(Option<PathBuf>),
    DeletePressed,
    DeleteConfirmed,
    DeleteResult(Result<(), String>),
    StageTransfer(TransferType),
    TransferStarted(usize, u64),
    TransferBytesProgress(u64),
    TransferFileStarted(String, u64),
    TransferItemFinished(u64, bool),
    TransferCompleted,
    OperationFailed(String),
    // Context menu messages
    ShowContextMenu(Option<FileItem>),
    HideContextMenu,
    ContextMenuOpen,
    ContextMenuDownload,
    ContextMenuDelete,
    ContextMenuNewFolder,
    // Keyboard shortcut messages
    KeyboardEvent(Key, Modifiers),
    // Create folder
    CreateFolderPressed,
    CreateFolderNameChanged(String),
    CreateFolderConfirmed,
    CreateFolderResult(Result<(), String>),
    CancelCreateFolder,
}

/// State for creating a new folder
#[derive(Debug, Clone, Default)]
struct CreateFolderState {
    active: bool,
    name: String,
}

impl PCloudGui {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                state: AppState::Login,
                status: Status::Idle,
                username: String::new(),
                password: String::new(),
                client: PCloudClient::new(None, Region::US, 20),
                current_path: "/".to_string(),
                file_list: Arc::new(Vec::new()),
                selected_item: None,
                concurrency_setting: 5,
                active_concurrency: 5,
                staged_transfer: None,
                active_transfer: None,
                bytes_progress: Arc::new(AtomicU64::new(0)),
                sort_by: SortBy::default(),
                sort_order: SortOrder::default(),
                search_filter: String::new(),
                context_menu: None,
                last_click_time: None,
                last_clicked_item: None,
                create_folder_state: CreateFolderState::default(),
            },
            Task::none(),
        )
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn is_busy(&self) -> bool {
        matches!(self.status, Status::Working(_) | Status::Transferring(_))
    }

    fn subscription(&self) -> Subscription<Message> {
        let keyboard_sub = keyboard::on_key_press(|key, modifiers| {
            Some(Message::KeyboardEvent(key, modifiers))
        });

        let transfer_sub = if let Some(transfer_type) = &self.active_transfer {
            match transfer_type {
                TransferType::Upload(id, tasks, bytes) => {
                    subscription::from_recipe(TransferRecipe {
                        id: *id,
                        mode: TransferMode::Upload(tasks.clone()),
                        client: self.client.clone(),
                        concurrency: self.active_concurrency,
                        total_files: tasks.len(),
                        total_bytes: *bytes,
                        bytes_progress: self.bytes_progress.clone(),
                    })
                }
                TransferType::Download(id, tasks, bytes) => {
                    subscription::from_recipe(TransferRecipe {
                        id: *id,
                        mode: TransferMode::Download(tasks.clone()),
                        client: self.client.clone(),
                        concurrency: self.active_concurrency,
                        total_files: tasks.len(),
                        total_bytes: *bytes,
                        bytes_progress: self.bytes_progress.clone(),
                    })
                }
            }
        } else {
            Subscription::none()
        };

        Subscription::batch([keyboard_sub, transfer_sub])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::UsernameChanged(v) => {
                self.username = v;
                Task::none()
            }
            Message::PasswordChanged(v) => {
                self.password = v;
                Task::none()
            }
            Message::LoginPressed => {
                if self.username.is_empty() || self.password.is_empty() {
                    self.status = Status::Error("Enter username/password".into());
                    return Task::none();
                }
                self.status = Status::Working("Authenticating...".into());
                let mut client = self.client.clone();
                let (u, p) = (self.username.clone(), self.password.clone());
                Task::perform(
                    async move { client.login(&u, &p).await.map_err(|e| e.to_string()) },
                    Message::LoginResult,
                )
            }
            Message::LoginResult(res) => match res {
                Ok(token) => {
                    self.state = AppState::Dashboard;
                    self.client.set_token(token);
                    self.status = Status::Success("Logged in".into());
                    self.update(Message::RefreshList)
                }
                Err(e) => {
                    self.status = Status::Error(e);
                    Task::none()
                }
            },
            Message::LogoutPressed => {
                self.state = AppState::Login;
                self.password.clear();
                self.active_transfer = None;
                self.staged_transfer = None;
                self.status = Status::Idle;
                Task::none()
            }
            Message::ConcurrencyChanged(val) => {
                self.concurrency_setting = val as usize;
                Task::none()
            }
            Message::RefreshList => {
                self.status = Status::Working("Listing...".into());
                let client = self.client.clone();
                let path = self.current_path.clone();
                Task::perform(
                    async move {
                        let list = client.list_folder(&path).await.map_err(|e| e.to_string())?;
                        Ok(Arc::new(list))
                    },
                    Message::ListResult,
                )
            }
            Message::ListResult(res) => {
                match res {
                    Ok(list) => {
                        self.file_list = list;
                        self.status = Status::Idle;
                    }
                    Err(e) => {
                        self.status = Status::Error(e);
                    }
                }
                Task::none()
            }
            Message::NavigateTo(folder) => {
                self.current_path = if self.current_path == "/" {
                    format!("/{}", folder)
                } else {
                    format!("{}/{}", self.current_path, folder)
                };
                self.selected_item = None;
                self.update(Message::RefreshList)
            }
            Message::NavigateUp => {
                if self.current_path != "/" {
                    let mut parts: Vec<&str> = self.current_path.split('/').collect();
                    parts.pop();
                    let new = parts.join("/");
                    self.current_path = if new.is_empty() { "/".to_string() } else { new };
                    self.update(Message::RefreshList)
                } else {
                    Task::none()
                }
            }
            Message::NavigateToPath(path) => {
                self.current_path = path;
                self.selected_item = None;
                self.update(Message::RefreshList)
            }
            Message::SortByChanged(sort_by) => {
                if self.sort_by == sort_by {
                    self.sort_order = match self.sort_order {
                        SortOrder::Ascending => SortOrder::Descending,
                        SortOrder::Descending => SortOrder::Ascending,
                    };
                } else {
                    self.sort_by = sort_by;
                    self.sort_order = SortOrder::Ascending;
                }
                Task::none()
            }
            Message::SearchFilterChanged(filter) => {
                self.search_filter = filter;
                Task::none()
            }
            Message::ClearSearchFilter => {
                self.search_filter.clear();
                Task::none()
            }
            Message::UploadFilePressed => {
                self.status = Status::Working("Selecting files...".into());
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .pick_files()
                            .await
                            .map(|h| h.into_iter().map(|f| f.path().to_path_buf()).collect())
                    },
                    Message::UploadSelected,
                )
            }
            Message::UploadSelected(opt) => {
                if let Some(paths) = opt {
                    let remote = self.current_path.clone();
                    let tasks: Vec<(PathBuf, String)> =
                        paths.into_iter().map(|p| (p, remote.clone())).collect();

                    let total_bytes: u64 = tasks
                        .iter()
                        .map(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
                        .sum();

                    let id = gen_id();
                    self.update(Message::StageTransfer(TransferType::Upload(
                        id,
                        tasks,
                        total_bytes,
                    )))
                } else {
                    self.status = Status::Idle;
                    Task::none()
                }
            }
            Message::StageTransfer(tt) => {
                let (count, bytes) = match &tt {
                    TransferType::Upload(_, t, b) => (t.len(), *b),
                    TransferType::Download(_, t, b) => (t.len(), *b),
                };
                self.staged_transfer = Some(tt);
                self.status = Status::ReadyToUpload(count, bytes);
                Task::none()
            }
            Message::UploadFolderPressed => {
                self.status = Status::Working("Selecting folder...".into());
                Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .pick_folder()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    Message::UploadFolderSelected,
                )
            }
            Message::UploadFolderSelected(opt) => {
                if let Some(path) = opt {
                    self.status = Status::Working("Scanning folder...".into());
                    let client = self.client.clone();
                    let local = path.to_string_lossy().to_string();
                    let remote = self.current_path.clone();
                    Task::perform(
                        async move {
                            let tasks = client.upload_folder_tree(local, remote).await.ok()?;
                            let pb_tasks: Vec<(PathBuf, String)> = tasks
                                .into_iter()
                                .map(|(l, r)| (PathBuf::from(l), r))
                                .collect();
                            let bytes: u64 = pb_tasks
                                .iter()
                                .map(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
                                .sum();
                            Some((pb_tasks, bytes))
                        },
                        |res| {
                            if let Some((tasks, bytes)) = res {
                                Message::StageTransfer(TransferType::Upload(gen_id(), tasks, bytes))
                            } else {
                                Message::OperationFailed("Scan failed".into())
                            }
                        },
                    )
                } else {
                    self.status = Status::Idle;
                    Task::none()
                }
            }
            Message::DownloadPressed => {
                if self.selected_item.is_some() {
                    self.status = Status::Working("Pick destination...".into());
                    Task::perform(
                        async {
                            rfd::AsyncFileDialog::new()
                                .pick_folder()
                                .await
                                .map(|h| h.path().to_path_buf())
                        },
                        Message::DownloadDestSelected,
                    )
                } else {
                    self.status = Status::Error("Select file first".into());
                    Task::none()
                }
            }
            Message::DownloadDestSelected(opt) => {
                if let Some(local_path) = opt {
                    let Some(item) = self.selected_item.clone() else {
                        self.status = Status::Error("No item selected".into());
                        return Task::none();
                    };
                    let local_base = local_path.to_string_lossy().to_string();
                    let remote = if self.current_path == "/" {
                        format!("/{}", item.name)
                    } else {
                        format!("{}/{}", self.current_path, item.name)
                    };

                    if item.isfolder {
                        self.status = Status::Working("Scanning remote...".into());
                        let client = self.client.clone();
                        Task::perform(
                            async move {
                                let tasks =
                                    client.download_folder_tree(remote, local_base).await.ok()?;
                                Some(tasks)
                            },
                            |res| {
                                if let Some(tasks) = res {
                                    Message::StageTransfer(TransferType::Download(
                                        gen_id(),
                                        tasks,
                                        0,
                                    ))
                                } else {
                                    Message::OperationFailed("Scan failed".into())
                                }
                            },
                        )
                    } else {
                        self.update(Message::StageTransfer(TransferType::Download(
                            gen_id(),
                            vec![(remote, local_base)],
                            item.size,
                        )))
                    }
                } else {
                    self.status = Status::Idle;
                    Task::none()
                }
            }
            Message::DeletePressed => {
                if let Some(item) = &self.selected_item {
                    let item_type = if item.isfolder { "folder" } else { "file" };
                    self.status = Status::Error(format!(
                        "Delete {}? Press Delete again to confirm",
                        item_type
                    ));
                    Task::none()
                } else {
                    self.status = Status::Error("Select item to delete".into());
                    Task::none()
                }
            }
            Message::DeleteConfirmed => {
                if let Some(item) = self.selected_item.clone() {
                    self.status = Status::Working("Deleting...".into());
                    let client = self.client.clone();
                    let path = if self.current_path == "/" {
                        format!("/{}", item.name)
                    } else {
                        format!("{}/{}", self.current_path, item.name)
                    };
                    let is_folder = item.isfolder;

                    Task::perform(
                        async move {
                            if is_folder {
                                client.delete_folder(&path).await
                            } else {
                                client.delete_file(&path).await
                            }
                            .map_err(|e| e.to_string())
                        },
                        Message::DeleteResult,
                    )
                } else {
                    Task::none()
                }
            }
            Message::DeleteResult(result) => match result {
                Ok(_) => {
                    self.status = Status::Success("Deleted successfully".into());
                    self.selected_item = None;
                    self.update(Message::RefreshList)
                }
                Err(e) => {
                    self.status = Status::Error(format!("Delete failed: {}", e));
                    Task::none()
                }
            },
            Message::StartTransferPressed => {
                if let Some(tt) = self.staged_transfer.take() {
                    self.active_concurrency = self.concurrency_setting;
                    self.bytes_progress.store(0, Ordering::Relaxed);
                    self.active_transfer = Some(tt);
                    self.status = Status::Working("Starting transfer...".into());
                }
                Task::none()
            }
            Message::CancelTransferPressed => {
                self.staged_transfer = None;
                self.active_transfer = None;
                self.status = Status::Idle;
                Task::none()
            }
            Message::TransferStarted(files, bytes) => {
                self.status = Status::Transferring(TransferProgress {
                    total_files: files,
                    finished_files: 0,
                    total_bytes: bytes,
                    transferred_bytes: 0,
                    start_time: Instant::now(),
                    current_speed: 0.0,
                    current_file: None,
                    current_file_size: 0,
                    current_file_progress: 0,
                });
                Task::none()
            }
            Message::TransferFileStarted(filename, size) => {
                if let Status::Transferring(p) = &mut self.status {
                    p.current_file = Some(filename);
                    p.current_file_size = size;
                    p.current_file_progress = 0;
                }
                Task::none()
            }
            Message::TransferBytesProgress(bytes) => {
                if let Status::Transferring(p) = &mut self.status {
                    p.transferred_bytes = bytes;
                    let elapsed = p.start_time.elapsed().as_secs_f64();
                    if elapsed > 0.1 {
                        p.current_speed = bytes as f64 / elapsed;
                    }
                }
                Task::none()
            }
            Message::TransferItemFinished(_bytes, _) => {
                if let Status::Transferring(p) = &mut self.status {
                    p.finished_files += 1;
                    // Bytes are now tracked via TransferBytesProgress
                }
                Task::none()
            }
            Message::TransferCompleted => {
                self.status = Status::Success("Transfer Complete!".into());
                self.active_transfer = None;
                self.update(Message::RefreshList)
            }
            Message::OperationFailed(s) => {
                self.status = Status::Error(s);
                Task::none()
            }
            // Item clicked - handles single/double click detection
            Message::ItemClicked(item) => {
                let now = std::time::Instant::now();
                let is_double_click = self
                    .last_click_time
                    .map(|t| now.duration_since(t) < DOUBLE_CLICK_THRESHOLD)
                    .unwrap_or(false)
                    && self
                        .last_clicked_item
                        .as_ref()
                        .map(|n| n == &item.name)
                        .unwrap_or(false);

                self.last_click_time = Some(now);
                self.last_clicked_item = Some(item.name.clone());

                if is_double_click && item.isfolder {
                    // Double-click on folder: navigate into it
                    self.update(Message::NavigateTo(item.name))
                } else {
                    // Single click: select item (works for both files and folders)
                    self.selected_item = Some(item);
                    Task::none()
                }
            }
            // Context menu messages
            Message::ShowContextMenu(item) => {
                // Also select the item when showing context menu
                if let Some(ref i) = item {
                    self.selected_item = Some(i.clone());
                }
                self.context_menu = Some(ContextMenu { item });
                Task::none()
            }
            Message::HideContextMenu => {
                self.context_menu = None;
                Task::none()
            }
            Message::ContextMenuOpen => {
                self.context_menu = None;
                if let Some(item) = &self.selected_item {
                    if item.isfolder {
                        return self.update(Message::NavigateTo(item.name.clone()));
                    }
                }
                Task::none()
            }
            Message::ContextMenuDownload => {
                self.context_menu = None;
                self.update(Message::DownloadPressed)
            }
            Message::ContextMenuDelete => {
                self.context_menu = None;
                self.update(Message::DeletePressed)
            }
            Message::ContextMenuNewFolder => {
                self.context_menu = None;
                self.update(Message::CreateFolderPressed)
            }
            // Keyboard shortcuts
            Message::KeyboardEvent(key, modifiers) => {
                // Don't process shortcuts during transfers or when typing in inputs
                if self.state != AppState::Dashboard {
                    return Task::none();
                }
                // Don't process if creating folder (typing)
                if self.create_folder_state.active {
                    if matches!(key, Key::Named(keyboard::key::Named::Escape)) {
                        return self.update(Message::CancelCreateFolder);
                    }
                    if matches!(key, Key::Named(keyboard::key::Named::Enter)) {
                        return self.update(Message::CreateFolderConfirmed);
                    }
                    return Task::none();
                }

                match key {
                    // Ctrl+R or F5: Refresh
                    Key::Character(c) if c.as_str() == "r" && modifiers.control() => {
                        self.update(Message::RefreshList)
                    }
                    Key::Named(keyboard::key::Named::F5) => self.update(Message::RefreshList),

                    // Ctrl+U: Upload files
                    Key::Character(c) if c.as_str() == "u" && modifiers.control() => {
                        if !self.is_busy() {
                            self.update(Message::UploadFilePressed)
                        } else {
                            Task::none()
                        }
                    }

                    // Ctrl+Shift+U: Upload folder
                    Key::Character(c)
                        if c.as_str() == "U" && modifiers.control() && modifiers.shift() =>
                    {
                        if !self.is_busy() {
                            self.update(Message::UploadFolderPressed)
                        } else {
                            Task::none()
                        }
                    }

                    // Ctrl+D: Download selected
                    Key::Character(c) if c.as_str() == "d" && modifiers.control() => {
                        if !self.is_busy() && self.selected_item.is_some() {
                            self.update(Message::DownloadPressed)
                        } else {
                            Task::none()
                        }
                    }

                    // Delete or Backspace: Delete selected (with confirmation)
                    Key::Named(keyboard::key::Named::Delete) => {
                        if !self.is_busy() && self.selected_item.is_some() {
                            let is_confirming =
                                matches!(&self.status, Status::Error(s) if s.contains("Delete"));
                            if is_confirming {
                                self.update(Message::DeleteConfirmed)
                            } else {
                                self.update(Message::DeletePressed)
                            }
                        } else {
                            Task::none()
                        }
                    }

                    // Enter: Open folder / start transfer if staged
                    Key::Named(keyboard::key::Named::Enter) => {
                        if matches!(self.status, Status::ReadyToUpload(_, _)) {
                            self.update(Message::StartTransferPressed)
                        } else if let Some(item) = &self.selected_item {
                            if item.isfolder {
                                let name = item.name.clone();
                                self.update(Message::NavigateTo(name))
                            } else {
                                Task::none()
                            }
                        } else {
                            Task::none()
                        }
                    }

                    // Backspace: Go up one directory
                    Key::Named(keyboard::key::Named::Backspace) => {
                        if !self.is_busy() {
                            self.update(Message::NavigateUp)
                        } else {
                            Task::none()
                        }
                    }

                    // Escape: Cancel staged transfer / clear selection / close context menu
                    Key::Named(keyboard::key::Named::Escape) => {
                        if self.context_menu.is_some() {
                            self.context_menu = None;
                            Task::none()
                        } else if self.staged_transfer.is_some() {
                            self.update(Message::CancelTransferPressed)
                        } else {
                            self.selected_item = None;
                            Task::none()
                        }
                    }

                    // Ctrl+N: New folder
                    Key::Character(c) if c.as_str() == "n" && modifiers.control() => {
                        if !self.is_busy() {
                            self.update(Message::CreateFolderPressed)
                        } else {
                            Task::none()
                        }
                    }

                    // Home: Go to root
                    Key::Named(keyboard::key::Named::Home) if modifiers.control() => {
                        self.update(Message::NavigateToPath("/".to_string()))
                    }

                    _ => Task::none(),
                }
            }
            // Create folder messages
            Message::CreateFolderPressed => {
                self.create_folder_state = CreateFolderState {
                    active: true,
                    name: String::new(),
                };
                Task::none()
            }
            Message::CreateFolderNameChanged(name) => {
                self.create_folder_state.name = name;
                Task::none()
            }
            Message::CreateFolderConfirmed => {
                let name = self.create_folder_state.name.trim().to_string();
                if name.is_empty() {
                    self.status = Status::Error("Folder name cannot be empty".into());
                    return Task::none();
                }
                self.create_folder_state.active = false;
                self.status = Status::Working("Creating folder...".into());
                let client = self.client.clone();
                let path = if self.current_path == "/" {
                    format!("/{}", name)
                } else {
                    format!("{}/{}", self.current_path, name)
                };
                Task::perform(
                    async move { client.create_folder(&path).await.map_err(|e| e.to_string()) },
                    Message::CreateFolderResult,
                )
            }
            Message::CreateFolderResult(result) => {
                self.create_folder_state = CreateFolderState::default();
                match result {
                    Ok(_) => {
                        self.status = Status::Success("Folder created".into());
                        self.update(Message::RefreshList)
                    }
                    Err(e) => {
                        self.status = Status::Error(format!("Failed to create folder: {}", e));
                        Task::none()
                    }
                }
            }
            Message::CancelCreateFolder => {
                self.create_folder_state = CreateFolderState::default();
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if self.state == AppState::Login {
            return self.view_login();
        }
        let sidebar = self.view_sidebar();
        let content = self.view_file_list();
        let status = self.view_status_bar();

        // Base layout
        let base = column![
            self.view_header(),
            horizontal_rule(1),
            row![sidebar, vertical_rule(1), content].height(Length::Fill),
            horizontal_rule(1),
            status
        ];

        // Overlay with context menu if active, or create folder dialog
        if self.create_folder_state.active {
            let dialog = self.view_create_folder_dialog();
            stack![
                base,
                mouse_area(
                    container(Space::new(Length::Fill, Length::Fill))
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .style(|_| container::Style {
                            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
                            ..Default::default()
                        })
                )
                .on_press(Message::CancelCreateFolder),
                container(opaque(dialog))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
            ]
            .into()
        } else if let Some(menu) = &self.context_menu {
            let menu_widget = self.view_context_menu(menu);
            stack![
                base,
                mouse_area(
                    container(Space::new(Length::Fill, Length::Fill))
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .style(|_| container::Style {
                            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.3).into()),
                            ..Default::default()
                        })
                )
                .on_press(Message::HideContextMenu),
                container(opaque(menu_widget))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
            ]
            .into()
        } else {
            base.into()
        }
    }

    fn view_create_folder_dialog(&self) -> Element<'_, Message> {
        container(
            column![
                text("Create New Folder").size(16),
                Space::with_height(15),
                text_input("Folder name", &self.create_folder_state.name)
                    .on_input(Message::CreateFolderNameChanged)
                    .on_submit(Message::CreateFolderConfirmed)
                    .padding(10)
                    .style(style_input),
                Space::with_height(15),
                row![
                    button(text("Cancel").align_x(alignment::Horizontal::Center))
                        .padding([8, 20])
                        .style(style_secondary)
                        .on_press(Message::CancelCreateFolder),
                    Space::with_width(10),
                    button(text("Create").align_x(alignment::Horizontal::Center))
                        .padding([8, 20])
                        .style(style_primary)
                        .on_press(Message::CreateFolderConfirmed),
                ]
            ]
            .padding(20)
            .width(300),
        )
        .style(|_| container::Style {
            background: Some(Color::from_rgb(0.15, 0.15, 0.15).into()),
            border: iced::Border {
                color: Color::from_rgb(0.3, 0.3, 0.3),
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        })
        .into()
    }

    fn view_context_menu(&self, menu: &ContextMenu) -> Element<'_, Message> {
        let mut menu_items: Vec<Element<'_, Message>> = Vec::new();

        // Show item name if selected
        if let Some(item) = &menu.item {
            let icon = if item.isfolder { "📁" } else { "📄" };
            let name = if item.name.len() > 25 {
                format!("{}...", &item.name[..22])
            } else {
                item.name.clone()
            };
            menu_items.push(
                text(format!("{} {}", icon, name))
                    .size(12)
                    .color(Color::from_rgb(0.7, 0.7, 0.7))
                    .into(),
            );
            menu_items.push(Space::with_height(8).into());

            if item.isfolder {
                menu_items.push(
                    button(text("📂 Open Folder").size(12))
                        .width(Length::Fill)
                        .padding([8, 15])
                        .style(style_context_menu_item)
                        .on_press(Message::ContextMenuOpen)
                        .into(),
                );
            }
            menu_items.push(
                button(text("⬇️ Download").size(12))
                    .width(Length::Fill)
                    .padding([8, 15])
                    .style(style_context_menu_item)
                    .on_press(Message::ContextMenuDownload)
                    .into(),
            );
            menu_items.push(
                button(text("🗑️ Delete").size(12))
                    .width(Length::Fill)
                    .padding([8, 15])
                    .style(style_context_menu_item)
                    .on_press(Message::ContextMenuDelete)
                    .into(),
            );
            menu_items.push(horizontal_rule(1).into());
        }

        menu_items.push(
            button(text("📁 New Folder").size(12))
                .width(Length::Fill)
                .padding([8, 15])
                .style(style_context_menu_item)
                .on_press(Message::ContextMenuNewFolder)
                .into(),
        );

        menu_items.push(Space::with_height(8).into());
        menu_items.push(
            button(text("Cancel").size(12).align_x(alignment::Horizontal::Center))
                .width(Length::Fill)
                .padding([8, 15])
                .style(style_secondary)
                .on_press(Message::HideContextMenu)
                .into(),
        );

        container(column(menu_items).width(200).padding(10))
            .style(|_| container::Style {
                background: Some(Color::from_rgb(0.15, 0.15, 0.15).into()),
                border: iced::Border {
                    color: Color::from_rgb(0.35, 0.35, 0.35),
                    width: 1.0,
                    radius: 8.0.into(),
                },
                shadow: iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                    offset: iced::Vector::new(2.0, 2.0),
                    blur_radius: 8.0,
                },
                ..Default::default()
            })
            .into()
    }

    fn view_login(&self) -> Element<'_, Message> {
        container(
            column![
                text("☁️ pCloud Fast Transfer")
                    .size(30)
                    .color(Color::from_rgb(0.2, 0.6, 1.0)),
                Space::with_height(20),
                text_input("Email", &self.username)
                    .on_input(Message::UsernameChanged)
                    .padding(10)
                    .style(style_input),
                text_input("Password", &self.password)
                    .on_input(Message::PasswordChanged)
                    .padding(10)
                    .secure(true)
                    .style(style_input),
                Space::with_height(20),
                button(text("Login").align_x(alignment::Horizontal::Center))
                    .on_press(Message::LoginPressed)
                    .width(Length::Fill)
                    .padding(10)
                    .style(style_primary)
            ]
            .width(300)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_| container::Style {
            background: Some(Color::from_rgb(0.1, 0.1, 0.1).into()),
            ..Default::default()
        })
        .into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let is_busy = self.is_busy();
        let btn = |l, m| {
            let b = button(text(l).align_x(alignment::Horizontal::Center))
                .width(Length::Fill)
                .padding(10)
                .style(style_primary);
            if !is_busy {
                b.on_press(m)
            } else {
                b
            }
        };

        // Keyboard shortcuts help text
        let shortcuts_help = column![
            text("Keyboard Shortcuts")
                .size(11)
                .color(Color::from_rgb(0.5, 0.5, 0.5)),
            Space::with_height(5),
            text("Ctrl+U  Upload files")
                .size(10)
                .color(Color::from_rgb(0.45, 0.45, 0.45)),
            text("Ctrl+D  Download")
                .size(10)
                .color(Color::from_rgb(0.45, 0.45, 0.45)),
            text("Ctrl+N  New folder")
                .size(10)
                .color(Color::from_rgb(0.45, 0.45, 0.45)),
            text("Ctrl+R / F5  Refresh")
                .size(10)
                .color(Color::from_rgb(0.45, 0.45, 0.45)),
            text("Enter  Open folder")
                .size(10)
                .color(Color::from_rgb(0.45, 0.45, 0.45)),
            text("Backspace  Go up")
                .size(10)
                .color(Color::from_rgb(0.45, 0.45, 0.45)),
            text("Delete  Delete item")
                .size(10)
                .color(Color::from_rgb(0.45, 0.45, 0.45)),
            text("Escape  Cancel/Clear")
                .size(10)
                .color(Color::from_rgb(0.45, 0.45, 0.45)),
            Space::with_height(5),
            text("Right-click for menu")
                .size(10)
                .color(Color::from_rgb(0.45, 0.45, 0.45)),
        ]
        .spacing(2);

        container(
            column![
                text("Actions")
                    .size(12)
                    .color(Color::from_rgb(0.5, 0.5, 0.5)),
                Space::with_height(10),
                btn("⬆️ Upload Files", Message::UploadFilePressed),
                Space::with_height(5),
                btn("⬆️ Upload Folder", Message::UploadFolderPressed),
                Space::with_height(20),
                btn("⬇️ Download", Message::DownloadPressed),
                Space::with_height(5),
                {
                    let is_confirming =
                        matches!(&self.status, Status::Error(s) if s.contains("Delete"));
                    let label = if is_confirming {
                        "🗑️ Confirm Delete"
                    } else {
                        "🗑️ Delete"
                    };
                    let msg = if is_confirming {
                        Message::DeleteConfirmed
                    } else {
                        Message::DeletePressed
                    };
                    let b = button(text(label).align_x(alignment::Horizontal::Center))
                        .width(Length::Fill)
                        .padding(10)
                        .style(if is_confirming {
                            style_danger
                        } else {
                            style_secondary
                        });
                    if !is_busy {
                        b.on_press(msg)
                    } else {
                        b
                    }
                },
                Space::with_height(5),
                {
                    let b = button(text("📁 New Folder").align_x(alignment::Horizontal::Center))
                        .width(Length::Fill)
                        .padding(10)
                        .style(style_secondary);
                    if !is_busy {
                        b.on_press(Message::CreateFolderPressed)
                    } else {
                        b
                    }
                },
                Space::with_height(30),
                text(format!("Concurrency: {}", self.concurrency_setting))
                    .size(12)
                    .color(Color::from_rgb(0.7, 0.7, 0.7)),
                slider(
                    1.0..=20.0,
                    self.concurrency_setting as f64,
                    Message::ConcurrencyChanged
                )
                .step(1.0),
                Space::with_height(30),
                text("Navigation")
                    .size(12)
                    .color(Color::from_rgb(0.5, 0.5, 0.5)),
                Space::with_height(10),
                button(text("⬅ Go Up"))
                    .width(Length::Fill)
                    .padding(8)
                    .style(style_secondary)
                    .on_press(Message::NavigateUp),
                Space::with_height(5),
                button(text("🔄 Refresh"))
                    .width(Length::Fill)
                    .padding(8)
                    .style(style_secondary)
                    .on_press(Message::RefreshList),
                Space::with_height(30),
                shortcuts_help,
            ]
            .width(200),
        )
        .padding(20)
        .style(|_| container::Style {
            background: Some(Color::from_rgb(0.12, 0.12, 0.12).into()),
            ..Default::default()
        })
        .height(Length::Fill)
        .into()
    }

    fn view_file_list(&self) -> Element<'_, Message> {
        let filter_lower = self.search_filter.to_lowercase();
        // Deref Arc
        let filtered_items: Vec<FileItem> = if self.search_filter.is_empty() {
            (*self.file_list).clone()
        } else {
            self.file_list
                .iter()
                .filter(|item| item.name.to_lowercase().contains(&filter_lower))
                .cloned()
                .collect()
        };

        let mut sorted_items = filtered_items;
        sorted_items.sort_by(|a, b| match (a.isfolder, b.isfolder) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => {
                let cmp = match self.sort_by {
                    SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                    SortBy::Size => a.size.cmp(&b.size),
                    SortBy::Date => a.modified.cmp(&b.modified),
                };
                match self.sort_order {
                    SortOrder::Ascending => cmp,
                    SortOrder::Descending => cmp.reverse(),
                }
            }
        });

        let list = column(
            sorted_items
                .into_iter()
                .map(|item| {
                    let is_sel = self
                        .selected_item
                        .as_ref()
                        .map(|i| i.name == item.name)
                        .unwrap_or(false);
                    let icon = if item.isfolder { "📁" } else { "📄" };
                    let size = item.size;
                    let item_clone = item.clone();
                    let item_for_context = item.clone();
                    let row_c = row![
                        text(icon),
                        Space::with_width(10),
                        text(item.name.clone()),
                        horizontal_space(),
                        text(format_bytes(size))
                            .size(12)
                            .color(Color::from_rgb(0.7, 0.7, 0.7))
                    ]
                    .align_y(Alignment::Center)
                    .padding(10);

                    // Wrap button in mouse_area for right-click detection
                    let btn = button(row_c)
                        .width(Length::Fill)
                        .style(move |_, s| {
                            let bg = if is_sel {
                                Color::from_rgb(0.2, 0.3, 0.5)
                            } else if s == button::Status::Hovered {
                                Color::from_rgb(0.2, 0.2, 0.2)
                            } else {
                                Color::TRANSPARENT
                            };
                            button::Style {
                                background: Some(bg.into()),
                                text_color: Color::WHITE,
                                ..Default::default()
                            }
                        })
                        // Single click selects, double-click handled by ItemClicked
                        .on_press(Message::ItemClicked(item_clone));

                    mouse_area(btn)
                        .on_right_press(Message::ShowContextMenu(Some(item_for_context)))
                        .into()
                })
                .collect::<Vec<_>>(),
        )
        .spacing(2);

        // Wrap the scrollable in a mouse_area for right-click on empty space
        let scrollable_list = scrollable(list).height(Length::Fill);
        mouse_area(scrollable_list)
            .on_right_press(Message::ShowContextMenu(None))
            .into()
    }

    fn view_header(&self) -> Element<'_, Message> {
        let breadcrumbs = self.view_breadcrumbs();
        let sort_controls = self.view_sort_controls();
        column![
            row![
                breadcrumbs,
                horizontal_space(),
                text(format!("👤 {}", self.username)).size(14),
                Space::with_width(20),
                button(text("Logout").size(12))
                    .style(style_secondary)
                    .on_press(Message::LogoutPressed)
                    .padding([5, 10])
            ]
            .padding(10)
            .align_y(Alignment::Center)
            .width(Length::Fill),
            sort_controls
        ]
        .into()
    }

    fn view_breadcrumbs(&self) -> Element<'_, Message> {
        let mut breadcrumb_row = row![].spacing(2).align_y(Alignment::Center);
        breadcrumb_row = breadcrumb_row.push(
            button(text("🏠").size(14))
                .style(style_breadcrumb)
                .padding([2, 6])
                .on_press(Message::NavigateToPath("/".to_string())),
        );

        if self.current_path != "/" {
            let parts: Vec<&str> = self
                .current_path
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();

            let mut accumulated_path = String::new();
            for (i, part) in parts.iter().enumerate() {
                accumulated_path = format!("{}/{}", accumulated_path, part);
                let path_clone = accumulated_path.clone();
                breadcrumb_row =
                    breadcrumb_row.push(text("/").size(14).color(Color::from_rgb(0.5, 0.5, 0.5)));

                if i == parts.len() - 1 {
                    breadcrumb_row = breadcrumb_row
                        .push(text(*part).size(14).color(Color::from_rgb(0.8, 0.8, 0.8)));
                } else {
                    breadcrumb_row = breadcrumb_row.push(
                        button(text(*part).size(14))
                            .style(style_breadcrumb)
                            .padding([2, 6])
                            .on_press(Message::NavigateToPath(path_clone)),
                    );
                }
            }
        }
        breadcrumb_row.into()
    }

    fn view_sort_controls(&self) -> Element<'_, Message> {
        let sort_indicator = match self.sort_order {
            SortOrder::Ascending => "▲",
            SortOrder::Descending => "▼",
        };
        let sort_btn = |label: &str, sort_by: SortBy| {
            let is_active = self.sort_by == sort_by;
            let display = if is_active {
                format!("{} {}", label, sort_indicator)
            } else {
                label.to_string()
            };
            button(text(display).size(11))
                .style(if is_active {
                    style_sort_active
                } else {
                    style_sort_inactive
                })
                .padding([3, 8])
                .on_press(Message::SortByChanged(sort_by))
        };

        let search_input = row![
            text("🔍").size(12),
            Space::with_width(4),
            text_input("Filter files...", &self.search_filter)
                .on_input(Message::SearchFilterChanged)
                .padding(4)
                .size(12)
                .width(Length::Fixed(150.0))
                .style(style_search_input),
            if !self.search_filter.is_empty() {
                button(text("✕").size(10))
                    .style(style_clear_btn)
                    .padding([2, 6])
                    .on_press(Message::ClearSearchFilter)
            } else {
                button(text("").size(10))
                    .style(style_clear_btn)
                    .padding([2, 6])
            }
        ]
        .align_y(Alignment::Center);

        row![
            text("Sort:").size(11).color(Color::from_rgb(0.5, 0.5, 0.5)),
            Space::with_width(8),
            sort_btn("Name", SortBy::Name),
            Space::with_width(4),
            sort_btn("Size", SortBy::Size),
            Space::with_width(4),
            sort_btn("Date", SortBy::Date),
            horizontal_space(),
            search_input,
        ]
        .padding([3, 10])
        .align_y(Alignment::Center)
        .into()
    }

    fn view_status_bar(&self) -> Element<'_, Message> {
        let content = match &self.status {
            Status::Idle => row![text("Ready").size(12)],
            Status::Working(s) => row![text(s).size(12).color(Color::from_rgb(0.4, 0.8, 1.0))],
            Status::Success(s) => row![text(s).size(12).color(Color::from_rgb(0.4, 1.0, 0.4))],
            Status::Error(s) => row![text(format!("Error: {}", s))
                .size(12)
                .color(Color::from_rgb(1.0, 0.4, 0.4))],
            Status::ReadyToUpload(count, bytes) => row![
                text(format!(
                    "Selected {} files ({})",
                    count,
                    format_bytes(*bytes)
                ))
                .size(12),
                horizontal_space(),
                button(text("Start Transfer").size(12))
                    .padding([5, 15])
                    .style(style_primary)
                    .on_press(Message::StartTransferPressed),
                Space::with_width(10),
                button(text("Cancel").size(12))
                    .padding([5, 10])
                    .style(style_secondary)
                    .on_press(Message::CancelTransferPressed),
            ]
            .align_y(Alignment::Center),
            Status::Transferring(p) => {
                // Use byte-level progress for smoother updates
                let pct = if p.total_bytes > 0 {
                    (p.transferred_bytes as f32 / p.total_bytes as f32) * 100.0
                } else if p.total_files > 0 {
                    // Fallback to file-based progress if total_bytes is unknown
                    (p.finished_files as f32 / p.total_files as f32) * 100.0
                } else {
                    0.0
                };

                // Truncate filename if too long
                let current_file_display = p.current_file.as_ref().map(|f| {
                    if f.len() > 25 {
                        format!("{}...", &f[..22])
                    } else {
                        f.clone()
                    }
                }).unwrap_or_default();

                row![
                    column![
                        row![
                            progress_bar(0.0..=100.0, pct)
                                .height(8)
                                .width(Length::Fixed(200.0))
                                .style(style_bar),
                            Space::with_width(10),
                            text(format!(
                                "{}/{} files • {:.1}%",
                                p.finished_files,
                                p.total_files,
                                pct
                            ))
                            .size(11)
                        ]
                        .align_y(Alignment::Center),
                        row![
                            text(format!(
                                "📄 {} • {} / {} • {:.1} MB/s",
                                current_file_display,
                                format_bytes(p.transferred_bytes),
                                format_bytes(p.total_bytes),
                                p.current_speed / 1_000_000.0
                            ))
                            .size(10)
                            .color(Color::from_rgb(0.6, 0.6, 0.6))
                        ]
                    ]
                    .spacing(2)
                ]
                .align_y(Alignment::Center)
            }
        };
        container(content)
            .padding(10)
            .style(|_| container::Style {
                background: Some(Color::from_rgb(0.08, 0.08, 0.08).into()),
                ..Default::default()
            })
            .width(Length::Fill)
            .into()
    }
}

fn gen_id() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
fn format_bytes(b: u64) -> String {
    if b == 0 {
        return "0 B".to_string();
    }
    if b < 1024 {
        return format!("{} B", b);
    }

    const UNITS: &[&str] = &["KB", "MB", "GB", "TB", "PB", "EB"];
    let exp = ((b as f64).ln() / 1024f64.ln()).floor() as usize;
    let exp = exp.min(UNITS.len()); // Clamp to available units
    let unit_index = exp.saturating_sub(1);
    let divisor = 1024u64.pow(exp as u32) as f64;
    let value = b as f64 / divisor;

    format!("{:.1} {}", value, UNITS[unit_index])
}
fn style_input(_: &Theme, _: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::from_rgb(0.15, 0.15, 0.15)),
        border: iced::Border {
            color: Color::from_rgb(0.3, 0.3, 0.3),
            width: 1.0,
            radius: 4.0.into(),
        },
        icon: Color::WHITE,
        placeholder: Color::from_rgb(0.4, 0.4, 0.4),
        value: Color::WHITE,
        selection: Color::from_rgb(0.2, 0.4, 0.8),
    }
}
fn style_primary(_: &Theme, s: button::Status) -> button::Style {
    let b = button::Style {
        background: Some(Color::from_rgb(0.0, 0.47, 0.95).into()),
        text_color: Color::WHITE,
        border: iced::Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    match s {
        button::Status::Hovered => button::Style {
            background: Some(Color::from_rgb(0.1, 0.55, 1.0).into()),
            ..b
        },
        button::Status::Pressed => button::Style {
            background: Some(Color::from_rgb(0.0, 0.4, 0.8).into()),
            ..b
        },
        _ => b,
    }
}
fn style_secondary(_: &Theme, s: button::Status) -> button::Style {
    let b = button::Style {
        background: Some(Color::from_rgb(0.2, 0.2, 0.2).into()),
        text_color: Color::WHITE,
        border: iced::Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    match s {
        button::Status::Hovered => button::Style {
            background: Some(Color::from_rgb(0.25, 0.25, 0.25).into()),
            ..b
        },
        _ => b,
    }
}
fn style_danger(_: &Theme, s: button::Status) -> button::Style {
    let b = button::Style {
        background: Some(Color::from_rgb(0.7, 0.2, 0.2).into()),
        text_color: Color::WHITE,
        border: iced::Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    match s {
        button::Status::Hovered => button::Style {
            background: Some(Color::from_rgb(0.85, 0.25, 0.25).into()),
            ..b
        },
        button::Status::Pressed => button::Style {
            background: Some(Color::from_rgb(0.6, 0.15, 0.15).into()),
            ..b
        },
        _ => b,
    }
}
fn style_bar(_: &Theme) -> progress_bar::Style {
    progress_bar::Style {
        background: Background::Color(Color::from_rgb(0.2, 0.2, 0.2)),
        bar: Background::Color(Color::from_rgb(0.0, 0.47, 0.95)),
        border: iced::Border {
            radius: 2.0.into(),
            ..Default::default()
        },
    }
}
fn style_breadcrumb(_: &Theme, s: button::Status) -> button::Style {
    let b = button::Style {
        background: Some(Color::TRANSPARENT.into()),
        text_color: Color::from_rgb(0.4, 0.7, 1.0),
        border: iced::Border::default(),
        ..Default::default()
    };
    match s {
        button::Status::Hovered => button::Style {
            background: Some(Color::from_rgb(0.2, 0.2, 0.25).into()),
            text_color: Color::from_rgb(0.5, 0.8, 1.0),
            border: iced::Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..b
        },
        _ => b,
    }
}
fn style_sort_active(_: &Theme, s: button::Status) -> button::Style {
    let b = button::Style {
        background: Some(Color::from_rgb(0.2, 0.35, 0.5).into()),
        text_color: Color::WHITE,
        border: iced::Border {
            radius: 3.0.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    match s {
        button::Status::Hovered => button::Style {
            background: Some(Color::from_rgb(0.25, 0.4, 0.55).into()),
            ..b
        },
        _ => b,
    }
}
fn style_sort_inactive(_: &Theme, s: button::Status) -> button::Style {
    let b = button::Style {
        background: Some(Color::from_rgb(0.15, 0.15, 0.15).into()),
        text_color: Color::from_rgb(0.6, 0.6, 0.6),
        border: iced::Border {
            radius: 3.0.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    match s {
        button::Status::Hovered => button::Style {
            background: Some(Color::from_rgb(0.2, 0.2, 0.2).into()),
            text_color: Color::from_rgb(0.8, 0.8, 0.8),
            ..b
        },
        _ => b,
    }
}
fn style_search_input(_: &Theme, _: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::from_rgb(0.12, 0.12, 0.12)),
        border: iced::Border {
            color: Color::from_rgb(0.25, 0.25, 0.25),
            width: 1.0,
            radius: 3.0.into(),
        },
        icon: Color::WHITE,
        placeholder: Color::from_rgb(0.4, 0.4, 0.4),
        value: Color::WHITE,
        selection: Color::from_rgb(0.2, 0.4, 0.8),
    }
}
fn style_clear_btn(_: &Theme, s: button::Status) -> button::Style {
    let b = button::Style {
        background: Some(Color::TRANSPARENT.into()),
        text_color: Color::from_rgb(0.5, 0.5, 0.5),
        border: iced::Border::default(),
        ..Default::default()
    };
    match s {
        button::Status::Hovered => button::Style {
            text_color: Color::from_rgb(0.8, 0.3, 0.3),
            ..b
        },
        _ => b,
    }
}

fn style_context_menu_item(_: &Theme, s: button::Status) -> button::Style {
    let b = button::Style {
        background: Some(Color::TRANSPARENT.into()),
        text_color: Color::from_rgb(0.9, 0.9, 0.9),
        border: iced::Border::default(),
        ..Default::default()
    };
    match s {
        button::Status::Hovered => button::Style {
            background: Some(Color::from_rgb(0.25, 0.4, 0.6).into()),
            text_color: Color::WHITE,
            ..b
        },
        button::Status::Pressed => button::Style {
            background: Some(Color::from_rgb(0.2, 0.35, 0.5).into()),
            text_color: Color::WHITE,
            ..b
        },
        _ => b,
    }
}
