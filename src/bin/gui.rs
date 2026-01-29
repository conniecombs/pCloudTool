use iced::advanced::subscription::{self, Event, Hasher, Recipe};
use iced::futures::stream::{self, BoxStream, StreamExt};
use iced::keyboard::{self, Key, Modifiers};
use iced::time::Instant;
use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, mouse_area, opaque, progress_bar,
    row, scrollable, slider, stack, text, text_input, vertical_rule, Space,
};
use iced::{alignment, Alignment, Background, Color, Element, Length, Subscription, Task, Theme};

use pcloud_rust::{AccountInfo, DuplicateMode, FileItem, PCloudClient, Region};
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Theme mode for light/dark appearance
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ThemeMode {
    Light,
    #[default]
    Dark,
}

/// Windows Fluent-inspired color palette
#[derive(Debug, Clone, Copy)]
struct ThemeColors {
    // Backgrounds
    bg_base: Color,
    bg_surface: Color,
    bg_elevated: Color,
    bg_hover: Color,
    bg_selected: Color,

    // Text
    text_primary: Color,
    text_secondary: Color,
    text_disabled: Color,

    // Accent (Windows blue)
    accent: Color,
    accent_hover: Color,
    accent_pressed: Color,

    // Borders
    border: Color,
    border_strong: Color,

    // Status colors
    success: Color,
    warning: Color,
    error: Color,

    // Dividers
    divider: Color,
}

impl ThemeColors {
    fn dark() -> Self {
        Self {
            // Dark mode - Windows 11 inspired
            bg_base: Color::from_rgb(0.12, 0.12, 0.12), // #1f1f1f
            bg_surface: Color::from_rgb(0.16, 0.16, 0.16), // #292929
            bg_elevated: Color::from_rgb(0.20, 0.20, 0.20), // #333333
            bg_hover: Color::from_rgb(0.24, 0.24, 0.24), // #3d3d3d
            bg_selected: Color::from_rgb(0.20, 0.30, 0.45), // Accent tinted

            text_primary: Color::from_rgb(1.0, 1.0, 1.0),
            text_secondary: Color::from_rgb(0.70, 0.70, 0.70),
            text_disabled: Color::from_rgb(0.45, 0.45, 0.45),

            accent: Color::from_rgb(0.38, 0.56, 0.89), // #60a0e4 Windows blue
            accent_hover: Color::from_rgb(0.45, 0.63, 0.95),
            accent_pressed: Color::from_rgb(0.30, 0.48, 0.78),

            border: Color::from_rgb(0.28, 0.28, 0.28),
            border_strong: Color::from_rgb(0.40, 0.40, 0.40),

            success: Color::from_rgb(0.35, 0.75, 0.45),
            warning: Color::from_rgb(0.95, 0.75, 0.30),
            error: Color::from_rgb(0.90, 0.35, 0.35),

            divider: Color::from_rgb(0.22, 0.22, 0.22),
        }
    }

    fn light() -> Self {
        Self {
            // Light mode - Windows 11 inspired
            bg_base: Color::from_rgb(0.98, 0.98, 0.98), // #fafafa
            bg_surface: Color::from_rgb(1.0, 1.0, 1.0), // #ffffff
            bg_elevated: Color::from_rgb(0.96, 0.96, 0.96), // #f5f5f5
            bg_hover: Color::from_rgb(0.92, 0.92, 0.92), // #ebebeb
            bg_selected: Color::from_rgb(0.85, 0.91, 0.98), // Accent tinted

            text_primary: Color::from_rgb(0.10, 0.10, 0.10),
            text_secondary: Color::from_rgb(0.40, 0.40, 0.40),
            text_disabled: Color::from_rgb(0.60, 0.60, 0.60),

            accent: Color::from_rgb(0.00, 0.47, 0.84), // #0078d4 Windows blue
            accent_hover: Color::from_rgb(0.10, 0.53, 0.90),
            accent_pressed: Color::from_rgb(0.00, 0.40, 0.72),

            border: Color::from_rgb(0.85, 0.85, 0.85),
            border_strong: Color::from_rgb(0.70, 0.70, 0.70),

            success: Color::from_rgb(0.15, 0.60, 0.30),
            warning: Color::from_rgb(0.80, 0.60, 0.10),
            error: Color::from_rgb(0.80, 0.20, 0.20),

            divider: Color::from_rgb(0.90, 0.90, 0.90),
        }
    }

    fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => Self::light(),
            ThemeMode::Dark => Self::dark(),
        }
    }
}

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
    use_adaptive_concurrency: bool,
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
    // Account info for quota display
    account_info: Option<AccountInfo>,
    // Duplicate mode setting
    duplicate_mode: DuplicateMode,
    // Theme mode (light/dark)
    theme_mode: ThemeMode,
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
    // Account info
    FetchAccountInfo,
    AccountInfoResult(Result<AccountInfo, String>),
    // Settings
    ToggleAdaptiveConcurrency(bool),
    DuplicateModeChanged(DuplicateMode),
    ToggleTheme,
}

/// State for creating a new folder
#[derive(Debug, Clone, Default)]
struct CreateFolderState {
    active: bool,
    name: String,
}

impl PCloudGui {
    fn new() -> (Self, Task<Message>) {
        // Use adaptive worker count by default
        let adaptive_workers = PCloudClient::calculate_adaptive_workers();
        (
            Self {
                state: AppState::Login,
                status: Status::Idle,
                username: String::new(),
                password: String::new(),
                client: PCloudClient::new(None, Region::US, adaptive_workers),
                current_path: "/".to_string(),
                file_list: Arc::new(Vec::new()),
                selected_item: None,
                concurrency_setting: adaptive_workers,
                active_concurrency: adaptive_workers,
                use_adaptive_concurrency: true,
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
                account_info: None,
                duplicate_mode: DuplicateMode::Rename,
                theme_mode: ThemeMode::Dark,
            },
            Task::none(),
        )
    }

    fn theme(&self) -> Theme {
        match self.theme_mode {
            ThemeMode::Light => Theme::Light,
            ThemeMode::Dark => Theme::Dark,
        }
    }

    /// Get current theme colors
    fn colors(&self) -> ThemeColors {
        ThemeColors::for_mode(self.theme_mode)
    }

    fn is_busy(&self) -> bool {
        matches!(self.status, Status::Working(_) | Status::Transferring(_))
    }

    fn subscription(&self) -> Subscription<Message> {
        let keyboard_sub =
            keyboard::on_key_press(|key, modifiers| Some(Message::KeyboardEvent(key, modifiers)));

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
                    self.client.set_duplicate_mode(self.duplicate_mode);
                    self.status = Status::Success("Logged in".into());
                    // Fetch account info and refresh list in parallel
                    Task::batch([
                        self.update(Message::RefreshList),
                        self.update(Message::FetchAccountInfo),
                    ])
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
                self.account_info = None;
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
            // Account info handlers
            Message::FetchAccountInfo => {
                let client = self.client.clone();
                Task::perform(
                    async move { client.get_account_info().await.map_err(|e| e.to_string()) },
                    Message::AccountInfoResult,
                )
            }
            Message::AccountInfoResult(res) => {
                match res {
                    Ok(info) => {
                        self.account_info = Some(info);
                    }
                    Err(e) => {
                        // Silently log error, don't show to user as it's not critical
                        eprintln!("Failed to fetch account info: {}", e);
                    }
                }
                Task::none()
            }
            // Settings handlers
            Message::ToggleAdaptiveConcurrency(enabled) => {
                self.use_adaptive_concurrency = enabled;
                if enabled {
                    let adaptive = PCloudClient::calculate_adaptive_workers();
                    self.concurrency_setting = adaptive;
                }
                Task::none()
            }
            Message::DuplicateModeChanged(mode) => {
                self.duplicate_mode = mode;
                self.client.set_duplicate_mode(mode);
                Task::none()
            }
            Message::ToggleTheme => {
                self.theme_mode = match self.theme_mode {
                    ThemeMode::Light => ThemeMode::Dark,
                    ThemeMode::Dark => ThemeMode::Light,
                };
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
        let colors = self.colors();
        container(
            column![
                text("Create New Folder")
                    .size(16)
                    .color(colors.text_primary),
                Space::with_height(15),
                text_input("Folder name", &self.create_folder_state.name)
                    .on_input(Message::CreateFolderNameChanged)
                    .on_submit(Message::CreateFolderConfirmed)
                    .padding(10)
                    .style(make_input_style(colors)),
                Space::with_height(15),
                row![
                    button(text("Cancel").align_x(alignment::Horizontal::Center))
                        .padding([8, 20])
                        .style(make_secondary_style(colors))
                        .on_press(Message::CancelCreateFolder),
                    Space::with_width(10),
                    button(text("Create").align_x(alignment::Horizontal::Center))
                        .padding([8, 20])
                        .style(make_primary_style(colors))
                        .on_press(Message::CreateFolderConfirmed),
                ]
            ]
            .padding(20)
            .width(300),
        )
        .style(move |_| container::Style {
            background: Some(colors.bg_elevated.into()),
            border: iced::Border {
                color: colors.border,
                width: 1.0,
                radius: 8.0.into(),
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.3),
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 12.0,
            },
            ..Default::default()
        })
        .into()
    }

    fn view_context_menu(&self, menu: &ContextMenu) -> Element<'_, Message> {
        let colors = self.colors();
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
                    .color(colors.text_secondary)
                    .into(),
            );
            menu_items.push(Space::with_height(8).into());

            if item.isfolder {
                menu_items.push(
                    button(text("📂 Open Folder").size(12))
                        .width(Length::Fill)
                        .padding([8, 15])
                        .style(make_context_menu_item_style(colors))
                        .on_press(Message::ContextMenuOpen)
                        .into(),
                );
            }
            menu_items.push(
                button(text("⬇️ Download").size(12))
                    .width(Length::Fill)
                    .padding([8, 15])
                    .style(make_context_menu_item_style(colors))
                    .on_press(Message::ContextMenuDownload)
                    .into(),
            );
            menu_items.push(
                button(text("🗑️ Delete").size(12))
                    .width(Length::Fill)
                    .padding([8, 15])
                    .style(make_context_menu_item_style(colors))
                    .on_press(Message::ContextMenuDelete)
                    .into(),
            );
            menu_items.push(horizontal_rule(1).into());
        }

        menu_items.push(
            button(text("📁 New Folder").size(12))
                .width(Length::Fill)
                .padding([8, 15])
                .style(make_context_menu_item_style(colors))
                .on_press(Message::ContextMenuNewFolder)
                .into(),
        );

        menu_items.push(Space::with_height(8).into());
        menu_items.push(
            button(
                text("Cancel")
                    .size(12)
                    .align_x(alignment::Horizontal::Center),
            )
            .width(Length::Fill)
            .padding([8, 15])
            .style(make_secondary_style(colors))
            .on_press(Message::HideContextMenu)
            .into(),
        );

        container(column(menu_items).width(200).padding(10))
            .style(move |_| container::Style {
                background: Some(colors.bg_elevated.into()),
                border: iced::Border {
                    color: colors.border_strong,
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
        let colors = self.colors();
        let theme_icon = if self.theme_mode == ThemeMode::Dark {
            "🌙"
        } else {
            "☀️"
        };

        container(
            column![
                // Theme toggle in top-right
                row![
                    horizontal_space(),
                    button(text(theme_icon).size(16))
                        .padding([6, 10])
                        .style(make_theme_toggle_style(colors))
                        .on_press(Message::ToggleTheme),
                ]
                .width(Length::Fill),
                Space::with_height(40),
                text("☁️ pCloud Fast Transfer")
                    .size(30)
                    .color(colors.accent),
                Space::with_height(20),
                text_input("Email", &self.username)
                    .on_input(Message::UsernameChanged)
                    .padding(10)
                    .style(make_input_style(colors)),
                text_input("Password", &self.password)
                    .on_input(Message::PasswordChanged)
                    .padding(10)
                    .secure(true)
                    .style(make_input_style(colors)),
                Space::with_height(20),
                button(text("Login").align_x(alignment::Horizontal::Center))
                    .on_press(Message::LoginPressed)
                    .width(Length::Fill)
                    .padding(10)
                    .style(make_primary_style(colors))
            ]
            .width(320)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_| container::Style {
            background: Some(colors.bg_base.into()),
            ..Default::default()
        })
        .into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let colors = self.colors();
        let is_busy = self.is_busy();

        let primary_style = make_primary_style(colors);

        // Theme toggle
        let theme_icon = if self.theme_mode == ThemeMode::Dark {
            "🌙"
        } else {
            "☀️"
        };

        // Account quota display
        let quota_display: Element<'_, Message> = if let Some(info) = &self.account_info {
            let used_gb = info.used_quota as f64 / (1024.0 * 1024.0 * 1024.0);
            let total_gb = info.quota as f64 / (1024.0 * 1024.0 * 1024.0);
            let pct = info.usage_percent();
            let bar_color = if pct > 90.0 {
                colors.error
            } else if pct > 75.0 {
                colors.warning
            } else {
                colors.success
            };
            column![
                text("Storage").size(12).color(colors.text_secondary),
                Space::with_height(5),
                progress_bar(0.0..=100.0, pct as f32)
                    .height(6)
                    .style(move |_| progress_bar::Style {
                        background: Background::Color(colors.bg_elevated),
                        bar: Background::Color(bar_color),
                        border: iced::Border {
                            radius: 3.0.into(),
                            ..Default::default()
                        },
                    }),
                Space::with_height(3),
                text(format!("{:.1} / {:.1} GB ({:.0}%)", used_gb, total_gb, pct))
                    .size(10)
                    .color(colors.text_disabled),
                Space::with_height(15),
            ]
            .into()
        } else {
            Space::with_height(0).into()
        };

        // Duplicate mode buttons
        let dup_mode_btns = row![
            button(text("Skip").size(10))
                .padding([4, 8])
                .style(make_toggle_btn_style(
                    colors,
                    self.duplicate_mode == DuplicateMode::Skip
                ))
                .on_press(Message::DuplicateModeChanged(DuplicateMode::Skip)),
            Space::with_width(4),
            button(text("Rename").size(10))
                .padding([4, 8])
                .style(make_toggle_btn_style(
                    colors,
                    self.duplicate_mode == DuplicateMode::Rename
                ))
                .on_press(Message::DuplicateModeChanged(DuplicateMode::Rename)),
            Space::with_width(4),
            button(text("Overwrite").size(10))
                .padding([4, 8])
                .style(make_toggle_btn_style(
                    colors,
                    self.duplicate_mode == DuplicateMode::Overwrite
                ))
                .on_press(Message::DuplicateModeChanged(DuplicateMode::Overwrite)),
        ];

        // Adaptive concurrency toggle
        let adaptive_toggle = row![button(
            text(if self.use_adaptive_concurrency {
                "Auto"
            } else {
                "Manual"
            })
            .size(10)
        )
        .padding([4, 8])
        .style(make_toggle_btn_style(colors, self.use_adaptive_concurrency))
        .on_press(Message::ToggleAdaptiveConcurrency(
            !self.use_adaptive_concurrency
        )),];

        // Keyboard shortcuts help text
        let shortcuts_help = column![
            text("Keyboard Shortcuts")
                .size(11)
                .color(colors.text_secondary),
            Space::with_height(5),
            text("Ctrl+U  Upload files")
                .size(10)
                .color(colors.text_disabled),
            text("Ctrl+D  Download")
                .size(10)
                .color(colors.text_disabled),
            text("Ctrl+N  New folder")
                .size(10)
                .color(colors.text_disabled),
            text("Ctrl+R / F5  Refresh")
                .size(10)
                .color(colors.text_disabled),
            text("Enter  Open folder")
                .size(10)
                .color(colors.text_disabled),
            text("Backspace  Go up")
                .size(10)
                .color(colors.text_disabled),
            text("Delete  Delete item")
                .size(10)
                .color(colors.text_disabled),
            text("Escape  Cancel/Clear")
                .size(10)
                .color(colors.text_disabled),
            Space::with_height(5),
            text("Right-click for menu")
                .size(10)
                .color(colors.text_disabled),
        ]
        .spacing(2);

        let sidebar_content = scrollable(
            column![
                // Theme toggle at top
                row![
                    text("Theme").size(12).color(colors.text_secondary),
                    horizontal_space(),
                    button(text(theme_icon).size(14))
                        .padding([4, 8])
                        .style(make_theme_toggle_style(colors))
                        .on_press(Message::ToggleTheme),
                ]
                .align_y(Alignment::Center),
                Space::with_height(15),
                quota_display,
                text("Actions").size(12).color(colors.text_secondary),
                Space::with_height(10),
                {
                    let b = button(text("⬆️ Upload Files").align_x(alignment::Horizontal::Center))
                        .width(Length::Fill)
                        .padding(10)
                        .style(primary_style);
                    if !is_busy {
                        b.on_press(Message::UploadFilePressed)
                    } else {
                        b
                    }
                },
                Space::with_height(5),
                {
                    let b = button(text("⬆️ Upload Folder").align_x(alignment::Horizontal::Center))
                        .width(Length::Fill)
                        .padding(10)
                        .style(make_primary_style(colors));
                    if !is_busy {
                        b.on_press(Message::UploadFolderPressed)
                    } else {
                        b
                    }
                },
                Space::with_height(20),
                {
                    let b = button(text("⬇️ Download").align_x(alignment::Horizontal::Center))
                        .width(Length::Fill)
                        .padding(10)
                        .style(make_primary_style(colors));
                    if !is_busy {
                        b.on_press(Message::DownloadPressed)
                    } else {
                        b
                    }
                },
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
                    let btn_style = make_delete_btn_style(colors, is_confirming);
                    let b = button(text(label).align_x(alignment::Horizontal::Center))
                        .width(Length::Fill)
                        .padding(10)
                        .style(btn_style);
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
                        .style(make_secondary_style(colors));
                    if !is_busy {
                        b.on_press(Message::CreateFolderPressed)
                    } else {
                        b
                    }
                },
                Space::with_height(20),
                text("Duplicates").size(12).color(colors.text_secondary),
                Space::with_height(5),
                dup_mode_btns,
                Space::with_height(20),
                row![
                    text(format!("Threads: {}", self.concurrency_setting))
                        .size(12)
                        .color(colors.text_secondary),
                    horizontal_space(),
                    adaptive_toggle,
                ]
                .align_y(Alignment::Center),
                if !self.use_adaptive_concurrency {
                    Element::from(
                        slider(
                            1.0..=32.0,
                            self.concurrency_setting as f64,
                            Message::ConcurrencyChanged,
                        )
                        .step(1.0),
                    )
                } else {
                    Element::from(Space::with_height(0))
                },
                Space::with_height(20),
                text("Navigation").size(12).color(colors.text_secondary),
                Space::with_height(10),
                button(text("⬅ Go Up"))
                    .width(Length::Fill)
                    .padding(8)
                    .style(make_secondary_style(colors))
                    .on_press(Message::NavigateUp),
                Space::with_height(5),
                button(text("🔄 Refresh"))
                    .width(Length::Fill)
                    .padding(8)
                    .style(make_secondary_style(colors))
                    .on_press(Message::RefreshList),
                Space::with_height(20),
                shortcuts_help,
            ]
            .width(200),
        );

        container(sidebar_content)
            .padding(20)
            .style(move |_| container::Style {
                background: Some(colors.bg_surface.into()),
                ..Default::default()
            })
            .height(Length::Fill)
            .into()
    }

    fn view_file_list(&self) -> Element<'_, Message> {
        let colors = self.colors();
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
                        text(item.name.clone()).color(colors.text_primary),
                        horizontal_space(),
                        text(format_bytes(size))
                            .size(12)
                            .color(colors.text_secondary)
                    ]
                    .align_y(Alignment::Center)
                    .padding(10);

                    // Wrap button in mouse_area for right-click detection
                    let btn = button(row_c)
                        .width(Length::Fill)
                        .style(make_file_item_style(colors, is_sel))
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
        let colors = self.colors();
        let breadcrumbs = self.view_breadcrumbs();
        let sort_controls = self.view_sort_controls();
        column![
            row![
                breadcrumbs,
                horizontal_space(),
                text(format!("👤 {}", self.username))
                    .size(14)
                    .color(colors.text_primary),
                Space::with_width(20),
                button(text("Logout").size(12))
                    .style(make_secondary_style(colors))
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
        let colors = self.colors();
        let mut breadcrumb_row = row![].spacing(2).align_y(Alignment::Center);
        breadcrumb_row = breadcrumb_row.push(
            button(text("🏠").size(14))
                .style(make_breadcrumb_style(colors))
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
                    breadcrumb_row.push(text("/").size(14).color(colors.text_disabled));

                if i == parts.len() - 1 {
                    breadcrumb_row =
                        breadcrumb_row.push(text(*part).size(14).color(colors.text_primary));
                } else {
                    breadcrumb_row = breadcrumb_row.push(
                        button(text(*part).size(14))
                            .style(make_breadcrumb_style(colors))
                            .padding([2, 6])
                            .on_press(Message::NavigateToPath(path_clone)),
                    );
                }
            }
        }
        breadcrumb_row.into()
    }

    fn view_sort_controls(&self) -> Element<'_, Message> {
        let colors = self.colors();
        let sort_indicator = match self.sort_order {
            SortOrder::Ascending => "▲",
            SortOrder::Descending => "▼",
        };
        let sort_btn = |label: &str, sort_by: SortBy, colors: ThemeColors, current: SortBy| {
            let is_active = current == sort_by;
            let display = if is_active {
                format!("{} {}", label, sort_indicator)
            } else {
                label.to_string()
            };
            button(text(display).size(11))
                .style(make_toggle_btn_style(colors, is_active))
                .padding([3, 8])
                .on_press(Message::SortByChanged(sort_by))
        };

        let search_input = row![
            text("🔍").size(12).color(colors.text_secondary),
            Space::with_width(4),
            text_input("Filter files...", &self.search_filter)
                .on_input(Message::SearchFilterChanged)
                .padding(4)
                .size(12)
                .width(Length::Fixed(150.0))
                .style(make_search_input_style(colors)),
            if !self.search_filter.is_empty() {
                button(text("✕").size(10))
                    .style(make_clear_btn_style(colors))
                    .padding([2, 6])
                    .on_press(Message::ClearSearchFilter)
            } else {
                button(text("").size(10))
                    .style(make_clear_btn_style(colors))
                    .padding([2, 6])
            }
        ]
        .align_y(Alignment::Center);

        let current_sort = self.sort_by;
        row![
            text("Sort:").size(11).color(colors.text_secondary),
            Space::with_width(8),
            sort_btn("Name", SortBy::Name, colors, current_sort),
            Space::with_width(4),
            sort_btn("Size", SortBy::Size, colors, current_sort),
            Space::with_width(4),
            sort_btn("Date", SortBy::Date, colors, current_sort),
            horizontal_space(),
            search_input,
        ]
        .padding([3, 10])
        .align_y(Alignment::Center)
        .into()
    }

    fn view_status_bar(&self) -> Element<'_, Message> {
        let colors = self.colors();
        let content = match &self.status {
            Status::Idle => row![text("Ready").size(12).color(colors.text_secondary)],
            Status::Working(s) => row![text(s).size(12).color(colors.accent)],
            Status::Success(s) => row![text(s).size(12).color(colors.success)],
            Status::Error(s) => {
                row![text(format!("Error: {}", s)).size(12).color(colors.error)]
            }
            Status::ReadyToUpload(count, bytes) => row![
                text(format!(
                    "Selected {} files ({})",
                    count,
                    format_bytes(*bytes)
                ))
                .size(12)
                .color(colors.text_primary),
                horizontal_space(),
                button(text("Start Transfer").size(12))
                    .padding([5, 15])
                    .style(make_primary_style(colors))
                    .on_press(Message::StartTransferPressed),
                Space::with_width(10),
                button(text("Cancel").size(12))
                    .padding([5, 10])
                    .style(make_secondary_style(colors))
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
                let current_file_display = p
                    .current_file
                    .as_ref()
                    .map(|f| {
                        if f.len() > 25 {
                            format!("{}...", &f[..22])
                        } else {
                            f.clone()
                        }
                    })
                    .unwrap_or_default();

                row![column![
                    row![
                        progress_bar(0.0..=100.0, pct)
                            .height(8)
                            .width(Length::Fixed(200.0))
                            .style(make_bar_style(colors)),
                        Space::with_width(10),
                        text(format!(
                            "{}/{} files • {:.1}%",
                            p.finished_files, p.total_files, pct
                        ))
                        .size(11)
                        .color(colors.text_primary)
                    ]
                    .align_y(Alignment::Center),
                    row![text(format!(
                        "📄 {} • {} / {} • {:.1} MB/s",
                        current_file_display,
                        format_bytes(p.transferred_bytes),
                        format_bytes(p.total_bytes),
                        p.current_speed / 1_000_000.0
                    ))
                    .size(10)
                    .color(colors.text_secondary)]
                ]
                .spacing(2)]
                .align_y(Alignment::Center)
            }
        };
        container(content)
            .padding(10)
            .style(move |_| container::Style {
                background: Some(colors.bg_base.into()),
                border: iced::Border {
                    color: colors.divider,
                    width: 1.0,
                    ..Default::default()
                },
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
// ============================================================================
// Theme-aware style functions - Windows Fluent Design inspired
// ============================================================================

/// Creates a style function for text inputs
fn make_input_style(
    colors: ThemeColors,
) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_, _| text_input::Style {
        background: Background::Color(colors.bg_surface),
        border: iced::Border {
            color: colors.border,
            width: 1.0,
            radius: 4.0.into(),
        },
        icon: colors.text_primary,
        placeholder: colors.text_disabled,
        value: colors.text_primary,
        selection: colors.accent,
    }
}

/// Creates a style function for primary buttons (accent colored)
fn make_primary_style(colors: ThemeColors) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| {
        let base = button::Style {
            background: Some(colors.accent.into()),
            text_color: Color::WHITE,
            border: iced::Border {
                radius: 4.0.into(),
                ..Default::default()
            },
            ..Default::default()
        };
        match s {
            button::Status::Hovered => button::Style {
                background: Some(colors.accent_hover.into()),
                ..base
            },
            button::Status::Pressed => button::Style {
                background: Some(colors.accent_pressed.into()),
                ..base
            },
            _ => base,
        }
    }
}

/// Creates a style function for secondary buttons
fn make_secondary_style(colors: ThemeColors) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| {
        let base = button::Style {
            background: Some(colors.bg_elevated.into()),
            text_color: colors.text_primary,
            border: iced::Border {
                color: colors.border,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        };
        match s {
            button::Status::Hovered => button::Style {
                background: Some(colors.bg_hover.into()),
                ..base
            },
            button::Status::Pressed => button::Style {
                background: Some(colors.bg_selected.into()),
                ..base
            },
            _ => base,
        }
    }
}

/// Creates a style function for delete button (danger or secondary based on confirmation state)
fn make_delete_btn_style(
    colors: ThemeColors,
    is_confirming: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| {
        if is_confirming {
            // Danger style
            let base = button::Style {
                background: Some(colors.error.into()),
                text_color: Color::WHITE,
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            };
            let error_hover = Color::from_rgba(
                (colors.error.r * 1.15).min(1.0),
                (colors.error.g * 1.15).min(1.0),
                (colors.error.b * 1.15).min(1.0),
                1.0,
            );
            let error_pressed = Color::from_rgba(
                colors.error.r * 0.85,
                colors.error.g * 0.85,
                colors.error.b * 0.85,
                1.0,
            );
            match s {
                button::Status::Hovered => button::Style {
                    background: Some(error_hover.into()),
                    ..base
                },
                button::Status::Pressed => button::Style {
                    background: Some(error_pressed.into()),
                    ..base
                },
                _ => base,
            }
        } else {
            // Secondary style
            let base = button::Style {
                background: Some(colors.bg_elevated.into()),
                text_color: colors.text_primary,
                border: iced::Border {
                    color: colors.border,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            };
            match s {
                button::Status::Hovered => button::Style {
                    background: Some(colors.bg_hover.into()),
                    ..base
                },
                button::Status::Pressed => button::Style {
                    background: Some(colors.bg_selected.into()),
                    ..base
                },
                _ => base,
            }
        }
    }
}

/// Creates a style function for progress bars
fn make_bar_style(colors: ThemeColors) -> impl Fn(&Theme) -> progress_bar::Style {
    move |_| progress_bar::Style {
        background: Background::Color(colors.bg_elevated),
        bar: Background::Color(colors.accent),
        border: iced::Border {
            radius: 2.0.into(),
            ..Default::default()
        },
    }
}

/// Creates a style function for breadcrumb buttons
fn make_breadcrumb_style(colors: ThemeColors) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| {
        let base = button::Style {
            background: Some(Color::TRANSPARENT.into()),
            text_color: colors.accent,
            border: iced::Border::default(),
            ..Default::default()
        };
        match s {
            button::Status::Hovered => button::Style {
                background: Some(colors.bg_hover.into()),
                text_color: colors.accent_hover,
                border: iced::Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                ..base
            },
            _ => base,
        }
    }
}

/// Creates a style function for sort/toggle buttons (active or inactive)
fn make_toggle_btn_style(
    colors: ThemeColors,
    is_active: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| {
        if is_active {
            let base = button::Style {
                background: Some(colors.bg_selected.into()),
                text_color: colors.text_primary,
                border: iced::Border {
                    color: colors.accent,
                    width: 1.0,
                    radius: 3.0.into(),
                },
                ..Default::default()
            };
            match s {
                button::Status::Hovered => button::Style {
                    background: Some(colors.bg_hover.into()),
                    ..base
                },
                _ => base,
            }
        } else {
            let base = button::Style {
                background: Some(colors.bg_elevated.into()),
                text_color: colors.text_secondary,
                border: iced::Border {
                    color: colors.border,
                    width: 1.0,
                    radius: 3.0.into(),
                },
                ..Default::default()
            };
            match s {
                button::Status::Hovered => button::Style {
                    background: Some(colors.bg_hover.into()),
                    text_color: colors.text_primary,
                    ..base
                },
                _ => base,
            }
        }
    }
}

/// Creates a style function for search input
fn make_search_input_style(
    colors: ThemeColors,
) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |_, _| text_input::Style {
        background: Background::Color(colors.bg_base),
        border: iced::Border {
            color: colors.border,
            width: 1.0,
            radius: 3.0.into(),
        },
        icon: colors.text_primary,
        placeholder: colors.text_disabled,
        value: colors.text_primary,
        selection: colors.accent,
    }
}

/// Creates a style function for clear button
fn make_clear_btn_style(colors: ThemeColors) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| {
        let base = button::Style {
            background: Some(Color::TRANSPARENT.into()),
            text_color: colors.text_disabled,
            border: iced::Border::default(),
            ..Default::default()
        };
        match s {
            button::Status::Hovered => button::Style {
                text_color: colors.error,
                ..base
            },
            _ => base,
        }
    }
}

/// Creates a style function for context menu items
fn make_context_menu_item_style(
    colors: ThemeColors,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| {
        let base = button::Style {
            background: Some(Color::TRANSPARENT.into()),
            text_color: colors.text_primary,
            border: iced::Border::default(),
            ..Default::default()
        };
        match s {
            button::Status::Hovered => button::Style {
                background: Some(colors.accent.into()),
                text_color: Color::WHITE,
                ..base
            },
            button::Status::Pressed => button::Style {
                background: Some(colors.accent_pressed.into()),
                text_color: Color::WHITE,
                ..base
            },
            _ => base,
        }
    }
}

/// Creates a style function for file list items
fn make_file_item_style(
    colors: ThemeColors,
    is_selected: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| {
        let bg = if is_selected {
            colors.bg_selected
        } else if s == button::Status::Hovered {
            colors.bg_hover
        } else {
            Color::TRANSPARENT
        };
        button::Style {
            background: Some(bg.into()),
            text_color: colors.text_primary,
            ..Default::default()
        }
    }
}

/// Creates a style function for theme toggle button
fn make_theme_toggle_style(
    colors: ThemeColors,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, s| {
        let base = button::Style {
            background: Some(colors.bg_elevated.into()),
            text_color: colors.text_primary,
            border: iced::Border {
                color: colors.border,
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        };
        match s {
            button::Status::Hovered => button::Style {
                background: Some(colors.bg_hover.into()),
                border: iced::Border {
                    color: colors.accent,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..base
            },
            _ => base,
        }
    }
}
