use iced::widget::{button, column, container, row, scrollable, text, text_input, Space, horizontal_rule};
use iced::{Alignment, Element, Length, Task, Color};
use pcloud_rust::{PCloudClient, Region, FileItem};
use std::path::PathBuf;

pub fn main() -> iced::Result {
    iced::application("pCloud Rust Manager", PCloudGui::update, PCloudGui::view)
        .theme(PCloudGui::theme)
        .run_with(PCloudGui::new)
}

enum AppState {
    Login,
    Dashboard,
}

struct PCloudGui {
    state: AppState,
    username: String,
    password: String,
    client: PCloudClient,
    current_path: String,
    file_list: Vec<FileItem>,
    selected_item: Option<FileItem>,
    status_message: String,
    is_loading: bool,
}

#[derive(Debug, Clone)]
enum Message {
    UsernameChanged(String),
    PasswordChanged(String),
    LoginPressed,
    LoginResult(Result<String, String>),
    RefreshList,
    ListResult(Result<Vec<FileItem>, String>),
    NavigateTo(String),
    NavigateUp,
    SelectItem(FileItem),

    UploadFilePressed,
    UploadFolderPressed,
    UploadSelected(Option<Vec<PathBuf>>),
    UploadFolderSelected(Option<PathBuf>),
    BatchResult((u32, u32)),

    DownloadPressed,
    DownloadDestSelected(Option<PathBuf>),
}

impl PCloudGui {
    fn new() -> (Self, Task<Message>) {
        (
            Self {
                state: AppState::Login,
                username: String::new(),
                password: String::new(),
                client: PCloudClient::new(None, Region::US, 8),
                current_path: "/".to_string(),
                file_list: Vec::new(),
                selected_item: None,
                status_message: "Ready.".to_string(),
                is_loading: false,
            },
            Task::none(),
        )
    }

    fn theme(&self) -> iced::Theme {
        iced::Theme::Dark
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // --- LOGIN ---
            Message::UsernameChanged(v) => {
                self.username = v;
                Task::none()
            }
            Message::PasswordChanged(v) => {
                self.password = v;
                Task::none()
            }
            Message::LoginPressed => {
                self.is_loading = true;
                self.status_message = "Authenticating...".to_string();
                let mut client = self.client.clone();
                let (u, p) = (self.username.clone(), self.password.clone());
                Task::perform(
                    async move {
                        client.login(&u, &p).await.map_err(|e| e.to_string())
                    },
                    Message::LoginResult,
                )
            }
            Message::LoginResult(res) => {
                self.is_loading = false;
                match res {
                    Ok(token) => {
                        self.state = AppState::Dashboard;
                        self.client.set_token(token);
                        self.update(Message::RefreshList)
                    }
                    Err(e) => {
                        self.status_message = format!("Error: {}", e);
                        Task::none()
                    }
                }
            }

            // --- NAVIGATION ---
            Message::RefreshList => {
                self.status_message = "Loading...".to_string();
                let client = self.client.clone();
                let path = self.current_path.clone();
                Task::perform(
                    async move {
                        client.list_folder(&path).await.map_err(|e| e.to_string())
                    },
                    Message::ListResult,
                )
            }
            Message::ListResult(res) => {
                match res {
                    Ok(list) => {
                        self.file_list = list;
                        self.status_message = format!("Path: {} ({} items)", self.current_path, self.file_list.len());
                        eprintln!("DEBUG: Successfully loaded {} items from {}", self.file_list.len(), self.current_path);
                    }
                    Err(e) => {
                        eprintln!("ERROR: Failed to list folder: {}", e);
                        self.status_message = format!("Error loading folder: {}", e);
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
                    self.current_path = if new.is_empty() {
                        "/".to_string()
                    } else {
                        new
                    };
                    self.selected_item = None;
                    self.update(Message::RefreshList)
                } else {
                    Task::none()
                }
            }
            Message::SelectItem(item) => {
                self.selected_item = Some(item);
                Task::none()
            }

            // --- UPLOAD FILES ---
            Message::UploadFilePressed => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Select files")
                        .pick_files()
                        .await
                        .map(|h| h.into_iter().map(|f| f.path().to_path_buf()).collect())
                },
                Message::UploadSelected,
            ),
            Message::UploadSelected(opt) => {
                if let Some(paths) = opt {
                    self.status_message = format!("Uploading {} files...", paths.len());
                    let client = self.client.clone();
                    let remote = self.current_path.clone();

                    // Flatten simple file uploads
                    let tasks: Vec<(String, String)> = paths
                        .iter()
                        .map(|p| (p.to_string_lossy().to_string(), remote.clone()))
                        .collect();

                    Task::perform(
                        async move { client.upload_files(tasks).await },
                        Message::BatchResult,
                    )
                } else {
                    Task::none()
                }
            }

            // --- UPLOAD FOLDER ---
            Message::UploadFolderPressed => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Select Folder")
                        .pick_folder()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                Message::UploadFolderSelected,
            ),
            Message::UploadFolderSelected(opt) => {
                if let Some(path) = opt {
                    self.status_message = "Scanning folder structure...".to_string();
                    let client = self.client.clone();
                    let local_root = path.to_string_lossy().to_string();
                    let remote_base = self.current_path.clone();

                    // Complex Logic: Scan -> Create Structure -> Upload
                    Task::perform(
                        async move {
                            match client.upload_folder_tree(local_root, remote_base).await {
                                Ok(tasks) => {
                                    println!("Found {} files to upload", tasks.len());
                                    client.upload_files(tasks).await
                                }
                                Err(e) => {
                                    eprintln!("Error scanning folder: {}", e);
                                    (0, 1) // Simple error signal
                                }
                            }
                        },
                        Message::BatchResult,
                    )
                } else {
                    Task::none()
                }
            }

            // --- DOWNLOAD ---
            Message::DownloadPressed => {
                if self.selected_item.is_some() {
                    Task::perform(
                        async {
                            rfd::AsyncFileDialog::new()
                                .set_title("Save to...")
                                .pick_folder()
                                .await
                                .map(|h| h.path().to_path_buf())
                        },
                        Message::DownloadDestSelected,
                    )
                } else {
                    self.status_message = "Select an item first.".to_string();
                    Task::none()
                }
            }
            Message::DownloadDestSelected(opt) => {
                if let Some(local_path) = opt {
                    let client = self.client.clone();
                    let item = self.selected_item.clone().unwrap();
                    let local_base = local_path.to_string_lossy().to_string();

                    let remote_full_path = if self.current_path == "/" {
                        format!("/{}", item.name)
                    } else {
                        format!("{}/{}", self.current_path, item.name)
                    };

                    if item.isfolder {
                        self.status_message = "Scanning remote folder...".to_string();
                        Task::perform(
                            async move {
                                match client.download_folder_tree(remote_full_path, local_base).await {
                                    Ok(tasks) => {
                                        println!("Found {} files to download", tasks.len());
                                        client.download_files(tasks).await
                                    }
                                    Err(e) => {
                                        eprintln!("Error scanning remote folder: {}", e);
                                        (0, 1)
                                    }
                                }
                            },
                            Message::BatchResult,
                        )
                    } else {
                        self.status_message = "Downloading file...".to_string();
                        // Single file download treated as batch of 1
                        Task::perform(
                            async move { client.download_files(vec![(remote_full_path, local_base)]).await },
                            Message::BatchResult,
                        )
                    }
                } else {
                    Task::none()
                }
            }

            Message::BatchResult((success, failed)) => {
                self.status_message = format!("Done! Success: {}, Failed: {}", success, failed);
                self.update(Message::RefreshList)
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if let AppState::Login = self.state {
            return column![
                text("pCloud Login").size(30),
                Space::with_height(20),
                text_input("Email", &self.username)
                    .on_input(Message::UsernameChanged)
                    .padding(10),
                text_input("Password", &self.password)
                    .on_input(Message::PasswordChanged)
                    .padding(10)
                    .secure(true),
                Space::with_height(20),
                button("Login").on_press(Message::LoginPressed).padding(10),
                Space::with_height(20),
                text(&self.status_message)
            ]
            .width(400)
            .align_x(Alignment::Center)
            .padding(50)
            .into();
        }

        let toolbar = row![
            button("⬆️ File")
                .on_press(Message::UploadFilePressed)
                .padding(10),
            Space::with_width(5),
            button("⬆️ Folder")
                .on_press(Message::UploadFolderPressed)
                .padding(10),
            Space::with_width(20),
            button("⬇️ Download")
                .on_press(Message::DownloadPressed)
                .padding(10),
            Space::with_width(20),
            button("Refresh")
                .on_press(Message::RefreshList)
                .padding(10),
        ]
        .padding(10);

        let nav = row![
            button("⬅ Back").on_press(Message::NavigateUp).padding(5),
            Space::with_width(10),
            text(&self.current_path)
        ]
        .padding(10)
        .align_y(Alignment::Center);

        let mut col = column![].spacing(5).padding(10);
        for item in &self.file_list {
            let icon = if item.isfolder { "📁" } else { "📄" };
            let row_content = row![
                text(format!("{} {}", icon, item.name)).width(Length::Fill),
                text(if item.isfolder {
                    "".into()
                } else {
                    format!("{} bytes", item.size)
                })
                .width(Length::Fixed(150.0))
            ];

            let btn = button(row_content).width(Length::Fill);

            col = col.push(if item.isfolder {
                btn.on_press(Message::NavigateTo(item.name.clone()))
            } else {
                btn.on_press(Message::SelectItem(item.clone()))
            });
        }

        column![
            toolbar,
            horizontal_rule(1),
            nav,
            scrollable(col).height(Length::Fill),
            horizontal_rule(1),
            container(text(&self.status_message).size(12))
                .padding(5)
                .style(|_theme| {
                    container::Style {
                        text_color: Some(Color::WHITE),
                        background: Some(Color::from_rgb(0.1, 0.1, 0.1).into()),
                        ..Default::default()
                    }
                })
        ]
        .into()
    }
}
