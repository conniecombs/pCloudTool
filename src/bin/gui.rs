use iced::time::Instant;
use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, progress_bar, row, scrollable,
    slider, text, text_input, vertical_rule, Space,
};
use iced::{
    alignment, Alignment, Background, Color, Element, Length, Subscription, Task,
    Theme,
};
use iced::advanced::subscription::{self, Recipe, Event, Hasher};
use iced::futures::stream::{self, BoxStream, StreamExt};

use pcloud_rust::{FileItem, PCloudClient, Region};
use std::path::PathBuf;
use std::hash::Hash;

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

#[derive(Debug, Clone)]
struct TransferProgress {
    total_files: usize,
    finished_files: usize,
    total_bytes: u64,
    transferred_bytes: u64,
    start_time: Instant,
    current_speed: f64, // bytes per second
}

#[derive(Debug, Clone)]
enum Status {
    Idle,
    ReadyToUpload(usize, u64), // Files selected, count/bytes
    Working(String),
    Transferring(TransferProgress),
    Success(String),
    Error(String),
}

struct PCloudGui {
    state: AppState,
    status: Status,

    // Auth Data
    username: String,
    password: String,

    // App Data
    client: PCloudClient,
    current_path: String,
    file_list: Vec<FileItem>,
    selected_item: Option<FileItem>,

    // Settings
    concurrency_setting: usize,      // The value on the slider
    active_concurrency: usize,       // The value locked in for the current/active transfer

    // Staged Transfer (waiting for user confirmation)
    staged_transfer: Option<TransferType>,
    
    // Active Transfer (currently running)
    active_transfer: Option<TransferType>,
}

#[derive(Debug, Clone)]
enum TransferType {
    // ID, Local Path, Remote Path, TotalBytes (calculated beforehand)
    Upload(u64, Vec<(PathBuf, String)>, u64), 
    // ID, Remote Path, Local Path, TotalBytes
    Download(u64, Vec<(String, String)>, u64), 
}

// --- RECIPE IMPLEMENTATION ---
struct TransferRecipe {
    id: u64,
    mode: TransferMode,
    client: PCloudClient,
    concurrency: usize,
    total_files: usize,
    total_bytes: u64,
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

    fn stream(
        self: Box<Self>,
        _input: BoxStream<Event>,
    ) -> BoxStream<Message> {
        let client = self.client.clone();
        let mode = self.mode.clone();
        let concurrency = self.concurrency;
        let t_files = self.total_files;
        let t_bytes = self.total_bytes;

        match mode {
            TransferMode::Upload(tasks) => {
                stream::once(async move {
                    Message::TransferStarted(t_files, t_bytes)
                })
                .chain(
                    stream::iter(tasks)
                        .map(move |(local, remote)| {
                            let c = client.clone();
                            async move {
                                let size = std::fs::metadata(&local).map(|m| m.len()).unwrap_or(0);
                                let result = c.upload_file(local.to_str().unwrap_or_default(), &remote).await;
                                Message::TransferItemFinished(size, result.is_ok())
                            }
                        })
                        .buffer_unordered(concurrency)
                )
                .chain(stream::once(async { Message::TransferCompleted }))
                .boxed()
            }
            TransferMode::Download(tasks) => {
                stream::once(async move {
                    Message::TransferStarted(t_files, t_bytes)
                })
                .chain(
                    stream::iter(tasks)
                        .map(move |(remote, local)| {
                            let c = client.clone();
                            async move {
                                let result = c.download_file(&remote, &local).await;
                                let size = if result.is_ok() {
                                    std::fs::metadata(&local).map(|m| m.len()).unwrap_or(0)
                                } else { 0 };
                                Message::TransferItemFinished(size, result.is_ok())
                            }
                        })
                        .buffer_unordered(concurrency)
                )
                .chain(stream::once(async { Message::TransferCompleted }))
                .boxed()
            }
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    // Auth
    UsernameChanged(String),
    PasswordChanged(String),
    LoginPressed,
    LoginResult(Result<String, String>),
    LogoutPressed,

    // Navigation
    RefreshList,
    ListResult(Result<Vec<FileItem>, String>),
    NavigateTo(String),
    NavigateUp,

    // Selection
    SelectItem(FileItem),

    // Settings
    ConcurrencyChanged(f64),

    // Actions
    UploadFilePressed,
    UploadFolderPressed,
    UploadSelected(Option<Vec<PathBuf>>),
    UploadFolderSelected(Option<PathBuf>),
    
    // Explicit Start
    StartTransferPressed,
    CancelTransferPressed,

    DownloadPressed,
    DownloadDestSelected(Option<PathBuf>),

    // Transfer Logic
    StageTransfer(TransferType),
    TransferStarted(usize, u64),
    TransferItemFinished(u64, bool),
    TransferCompleted,
    
    OperationFailed(String),
}

impl PCloudGui {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                state: AppState::Login,
                status: Status::Idle,
                username: String::new(),
                password: String::new(),
                // CHANGED: Workers increased to 20 to support full concurrency slider range
                client: PCloudClient::new(None, Region::US, 20),
                current_path: "/".to_string(),
                file_list: Vec::new(),
                selected_item: None,
                concurrency_setting: 5,
                active_concurrency: 5,
                staged_transfer: None,
                active_transfer: None,
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
        if let Some(transfer_type) = &self.active_transfer {
            match transfer_type {
                TransferType::Upload(id, tasks, bytes) => {
                    subscription::from_recipe(TransferRecipe {
                        id: *id,
                        mode: TransferMode::Upload(tasks.clone()),
                        client: self.client.clone(),
                        concurrency: self.active_concurrency,
                        total_files: tasks.len(),
                        total_bytes: *bytes,
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
                    })
                }
            }
        } else {
            Subscription::none()
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::UsernameChanged(v) => { self.username = v; Task::none() }
            Message::PasswordChanged(v) => { self.password = v; Task::none() }
            Message::LoginPressed => {
                if self.username.is_empty() || self.password.is_empty() {
                    self.status = Status::Error("Enter username/password".into());
                    return Task::none();
                }
                self.status = Status::Working("Authenticating...".into());
                let mut client = self.client.clone();
                let (u, p) = (self.username.clone(), self.password.clone());
                Task::perform(async move { client.login(&u, &p).await.map_err(|e| e.to_string()) }, Message::LoginResult)
            }
            Message::LoginResult(res) => match res {
                Ok(token) => {
                    self.state = AppState::Dashboard;
                    self.client.set_token(token);
                    self.status = Status::Success("Logged in".into());
                    self.update(Message::RefreshList)
                }
                Err(e) => { self.status = Status::Error(e); Task::none() }
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
                Task::perform(async move { client.list_folder(&path).await.map_err(|e| e.to_string()) }, Message::ListResult)
            }
            Message::ListResult(res) => {
                match res {
                    Ok(list) => { self.file_list = list; self.status = Status::Idle; }
                    Err(e) => { self.status = Status::Error(e); }
                }
                Task::none()
            }
            Message::NavigateTo(folder) => {
                self.current_path = if self.current_path == "/" { format!("/{}", folder) } else { format!("{}/{}", self.current_path, folder) };
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
                } else { Task::none() }
            }
            Message::SelectItem(item) => { self.selected_item = Some(item); Task::none() }
            
            // --- UPLOAD FLOW ---
            Message::UploadFilePressed => {
                self.status = Status::Working("Selecting files...".into());
                Task::perform(async { rfd::AsyncFileDialog::new().pick_files().await.map(|h| h.into_iter().map(|f| f.path().to_path_buf()).collect()) }, Message::UploadSelected)
            }
            Message::UploadSelected(opt) => {
                if let Some(paths) = opt {
                    let remote = self.current_path.clone();
                    let tasks: Vec<(PathBuf, String)> = paths.into_iter().map(|p| (p, remote.clone())).collect();
                    
                    let total_bytes: u64 = tasks.iter().map(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
                    
                    let id = gen_id();
                    // Recursively call update to handle state transition immediately
                    self.update(Message::StageTransfer(TransferType::Upload(id, tasks, total_bytes)))
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
                Task::perform(async { rfd::AsyncFileDialog::new().pick_folder().await.map(|h| h.path().to_path_buf()) }, Message::UploadFolderSelected)
            }
            Message::UploadFolderSelected(opt) => {
                if let Some(path) = opt {
                    self.status = Status::Working("Scanning folder...".into());
                    let client = self.client.clone();
                    let local = path.to_string_lossy().to_string();
                    let remote = self.current_path.clone();
                    Task::perform(async move {
                        let tasks = client.upload_folder_tree(local, remote).await.ok()?;
                        let pb_tasks: Vec<(PathBuf, String)> = tasks.into_iter().map(|(l,r)| (PathBuf::from(l), r)).collect();
                        let bytes: u64 = pb_tasks.iter().map(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
                        Some((pb_tasks, bytes))
                    }, |res| if let Some((tasks, bytes)) = res { 
                        Message::StageTransfer(TransferType::Upload(gen_id(), tasks, bytes)) 
                    } else { 
                        Message::OperationFailed("Scan failed".into()) 
                    })
                } else {
                    self.status = Status::Idle;
                    Task::none()
                }
            }

            // --- DOWNLOAD FLOW ---
            Message::DownloadPressed => {
                if self.selected_item.is_some() {
                    self.status = Status::Working("Pick destination...".into());
                    Task::perform(async { rfd::AsyncFileDialog::new().pick_folder().await.map(|h| h.path().to_path_buf()) }, Message::DownloadDestSelected)
                } else {
                    self.status = Status::Error("Select file first".into());
                    Task::none()
                }
            }
            Message::DownloadDestSelected(opt) => {
                if let Some(local_path) = opt {
                    let item = self.selected_item.clone().unwrap();
                    let local_base = local_path.to_string_lossy().to_string();
                    let remote = if self.current_path == "/" { format!("/{}", item.name) } else { format!("{}/{}", self.current_path, item.name) };

                    if item.isfolder {
                        self.status = Status::Working("Scanning remote...".into());
                        let client = self.client.clone();
                        Task::perform(async move {
                            let tasks = client.download_folder_tree(remote, local_base).await.ok()?;
                            Some(tasks)
                        }, |res| if let Some(tasks) = res { 
                            Message::StageTransfer(TransferType::Download(gen_id(), tasks, 0)) 
                        } else { 
                            Message::OperationFailed("Scan failed".into()) 
                        })
                    } else {
                        // Recursively call update for single file download
                        self.update(Message::StageTransfer(TransferType::Download(gen_id(), vec![(remote, local_base)], item.size)))
                    }
                } else {
                    self.status = Status::Idle;
                    Task::none()
                }
            }

            // --- TRANSFER EXECUTION ---
            Message::StartTransferPressed => {
                if let Some(tt) = self.staged_transfer.take() {
                    self.active_concurrency = self.concurrency_setting;
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
                self.status = Status::Transferring(TransferProgress { total_files: files, finished_files: 0, total_bytes: bytes, transferred_bytes: 0, start_time: Instant::now(), current_speed: 0.0 });
                Task::none()
            }
            Message::TransferItemFinished(bytes, _) => {
                if let Status::Transferring(p) = &mut self.status {
                    p.finished_files += 1;
                    p.transferred_bytes += bytes;
                    let elapsed = p.start_time.elapsed().as_secs_f64();
                    if elapsed > 0.0 { p.current_speed = p.transferred_bytes as f64 / elapsed; }
                }
                Task::none()
            }
            Message::TransferCompleted => {
                self.status = Status::Success("Transfer Complete!".into());
                self.active_transfer = None;
                self.update(Message::RefreshList)
            }
            Message::OperationFailed(s) => { self.status = Status::Error(s); Task::none() }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if self.state == AppState::Login { return self.view_login(); }

        let sidebar = self.view_sidebar();
        let content = self.view_file_list();
        let status = self.view_status_bar();

        column![
            self.view_header(), 
            horizontal_rule(1), 
            row![sidebar, vertical_rule(1), content].height(Length::Fill), 
            horizontal_rule(1), 
            status
        ].into()
    }

    fn view_login(&self) -> Element<'_, Message> {
        container(column![
            text("☁️ pCloud Fast Transfer").size(30).color(Color::from_rgb(0.2, 0.6, 1.0)),
            Space::with_height(20),
            text_input("Email", &self.username).on_input(Message::UsernameChanged).padding(10).style(style_input),
            text_input("Password", &self.password).on_input(Message::PasswordChanged).padding(10).secure(true).style(style_input),
            Space::with_height(20),
            button(text("Login").align_x(alignment::Horizontal::Center)).on_press(Message::LoginPressed).width(Length::Fill).padding(10).style(style_primary)
        ].width(300).align_x(Alignment::Center))
        .width(Length::Fill).height(Length::Fill).center_x(Length::Fill).center_y(Length::Fill).style(|_| container::Style { background: Some(Color::from_rgb(0.1, 0.1, 0.1).into()), ..Default::default() }).into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let is_busy = self.is_busy();
        let btn = |l, m| {
            let b = button(text(l).align_x(alignment::Horizontal::Center)).width(Length::Fill).padding(10).style(style_primary);
            if !is_busy { b.on_press(m) } else { b }
        };

        container(column![
            text("Actions").size(12).color(Color::from_rgb(0.5, 0.5, 0.5)),
            Space::with_height(10),
            btn("⬆️ Upload Files", Message::UploadFilePressed),
            Space::with_height(5),
            btn("⬆️ Upload Folder", Message::UploadFolderPressed),
            Space::with_height(20),
            btn("⬇️ Download", Message::DownloadPressed),
            
            Space::with_height(30),
            text(format!("Concurrency: {}", self.concurrency_setting)).size(12).color(Color::from_rgb(0.7, 0.7, 0.7)),
            slider(1.0..=20.0, self.concurrency_setting as f64, Message::ConcurrencyChanged).step(1.0),
            
            Space::with_height(30),
            text("Navigation").size(12).color(Color::from_rgb(0.5, 0.5, 0.5)),
            Space::with_height(10),
            button(text("⬅ Go Up")).width(Length::Fill).padding(8).style(style_secondary).on_press(Message::NavigateUp),
            Space::with_height(5),
            button(text("🔄 Refresh")).width(Length::Fill).padding(8).style(style_secondary).on_press(Message::RefreshList),
        ].width(200)).padding(20).style(|_| container::Style { background: Some(Color::from_rgb(0.12, 0.12, 0.12).into()), ..Default::default() }).height(Length::Fill).into()
    }

    fn view_file_list(&self) -> Element<'_, Message> {
        let list = column(self.file_list.iter().map(|item| {
            let is_sel = self.selected_item.as_ref().map(|i| i.name == item.name).unwrap_or(false);
            let icon = if item.isfolder { "📁" } else { "📄" };
            let row_c = row![text(icon), Space::with_width(10), text(&item.name), horizontal_space(), text(format_bytes(item.size)).size(12).color(Color::from_rgb(0.7, 0.7, 0.7))].align_y(Alignment::Center).padding(10);
            button(row_c).width(Length::Fill).style(move |_, s| {
                let bg = if is_sel { Color::from_rgb(0.2, 0.3, 0.5) } else if s == button::Status::Hovered { Color::from_rgb(0.2, 0.2, 0.2) } else { Color::TRANSPARENT };
                button::Style { background: Some(bg.into()), text_color: Color::WHITE, ..Default::default() }
            }).on_press(if item.isfolder { Message::NavigateTo(item.name.clone()) } else { Message::SelectItem(item.clone()) }).into()
        }).collect::<Vec<_>>()).spacing(2);
        
        scrollable(list).height(Length::Fill).into()
    }

    fn view_header(&self) -> Element<'_, Message> {
        row![
            text(format!("📂 {}", self.current_path)).size(14),
            horizontal_space(),
            text(format!("👤 {}", self.username)).size(14),
            Space::with_width(20),
            button(text("Logout").size(12)).style(style_secondary).on_press(Message::LogoutPressed).padding([5, 10])
        ].padding(10).align_y(Alignment::Center).width(Length::Fill).into()
    }

    fn view_status_bar(&self) -> Element<'_, Message> {
        let content = match &self.status {
            Status::Idle => row![text("Ready").size(12)],
            Status::Working(s) => row![text(s).size(12).color(Color::from_rgb(0.4, 0.8, 1.0))],
            Status::Success(s) => row![text(s).size(12).color(Color::from_rgb(0.4, 1.0, 0.4))],
            Status::Error(s) => row![text(format!("Error: {}", s)).size(12).color(Color::from_rgb(1.0, 0.4, 0.4))],
            Status::ReadyToUpload(count, bytes) => row![
                text(format!("Selected {} files ({})", count, format_bytes(*bytes))).size(12),
                horizontal_space(),
                // Manual Start Button
                button(text("Start Transfer").size(12)).padding([5, 15]).style(style_primary).on_press(Message::StartTransferPressed),
                Space::with_width(10),
                button(text("Cancel").size(12)).padding([5, 10]).style(style_secondary).on_press(Message::CancelTransferPressed),
            ].align_y(Alignment::Center),
            Status::Transferring(p) => {
                let pct = if p.total_files > 0 { (p.finished_files as f32 / p.total_files as f32) * 100.0 } else { 0.0 };
                row![
                    progress_bar(0.0..=100.0, pct).height(10).width(Length::Fixed(200.0)).style(style_bar),
                    Space::with_width(10),
                    text(format!("{}/{} • {:.1} MB/s", p.finished_files, p.total_files, p.current_speed / 1_000_000.0)).size(12)
                ].align_y(Alignment::Center)
            }
        };
        container(content).padding(10).style(|_| container::Style { background: Some(Color::from_rgb(0.08, 0.08, 0.08).into()), ..Default::default() }).width(Length::Fill).into()
    }
}

// Styles & Helpers
fn gen_id() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos() as u64 }
fn format_bytes(b: u64) -> String { if b < 1024 { return format!("{} B", b); } let e = (b as f64).ln() / 1024f64.ln(); format!("{:.1} {}B", b as f64 / 1024f64.powf(e), "KMGTPE".chars().nth(e as usize - 1).unwrap_or('?')) }
fn style_input(_: &Theme, _: text_input::Status) -> text_input::Style { text_input::Style { background: Background::Color(Color::from_rgb(0.15, 0.15, 0.15)), border: iced::Border { color: Color::from_rgb(0.3, 0.3, 0.3), width: 1.0, radius: 4.0.into() }, icon: Color::WHITE, placeholder: Color::from_rgb(0.4, 0.4, 0.4), value: Color::WHITE, selection: Color::from_rgb(0.2, 0.4, 0.8) } }
fn style_primary(_: &Theme, s: button::Status) -> button::Style { let b = button::Style { background: Some(Color::from_rgb(0.0, 0.47, 0.95).into()), text_color: Color::WHITE, border: iced::Border { radius: 4.0.into(), ..Default::default() }, ..Default::default() }; match s { button::Status::Hovered => button::Style { background: Some(Color::from_rgb(0.1, 0.55, 1.0).into()), ..b }, button::Status::Pressed => button::Style { background: Some(Color::from_rgb(0.0, 0.4, 0.8).into()), ..b }, _ => b } }
fn style_secondary(_: &Theme, s: button::Status) -> button::Style { let b = button::Style { background: Some(Color::from_rgb(0.2, 0.2, 0.2).into()), text_color: Color::WHITE, border: iced::Border { radius: 4.0.into(), ..Default::default() }, ..Default::default() }; match s { button::Status::Hovered => button::Style { background: Some(Color::from_rgb(0.25, 0.25, 0.25).into()), ..b }, _ => b } }
fn style_bar(_: &Theme) -> progress_bar::Style { progress_bar::Style { background: Background::Color(Color::from_rgb(0.2, 0.2, 0.2)), bar: Background::Color(Color::from_rgb(0.0, 0.47, 0.95)), border: iced::Border { radius: 2.0.into(), ..Default::default() } } }
