#!/usr/bin/env python3
"""
pCloud Fast Transfer - Modern GUI
A sleek, modern interface for fast pCloud uploads and downloads
"""

import os
import sys
import json
import threading
from pathlib import Path
from typing import List, Optional, Tuple
import tkinter as tk
from tkinter import filedialog, messagebox
import customtkinter as ctk

from pcloud_fast_transfer import PCloudClient, TransferStats


# Set appearance mode and color theme
ctk.set_appearance_mode("dark")  # Modes: "dark", "light", "system"
ctk.set_default_color_theme("blue")  # Themes: "blue", "green", "dark-blue"


class AuthDialog(ctk.CTkToplevel):
    """Authentication dialog for pCloud credentials"""

    def __init__(self, parent):
        super().__init__(parent)

        self.title("pCloud Authentication")
        self.geometry("600x500")
        self.resizable(True, True)
        self.minsize(500, 450)  # Set minimum size

        # Center the window
        self.update_idletasks()
        x = (self.winfo_screenwidth() // 2) - (600 // 2)
        y = (self.winfo_screenheight() // 2) - (500 // 2)
        self.geometry(f"600x500+{x}+{y}")

        self.result = None
        self.transient(parent)
        self.grab_set()

        self._create_widgets()

    def _create_widgets(self):
        """Create authentication form widgets"""
        # Title
        title = ctk.CTkLabel(
            self,
            text="🔐 pCloud Authentication",
            font=ctk.CTkFont(size=24, weight="bold")
        )
        title.pack(pady=(30, 20))

        subtitle = ctk.CTkLabel(
            self,
            text="Enter your credentials to connect to pCloud",
            font=ctk.CTkFont(size=12),
            text_color="gray"
        )
        subtitle.pack(pady=(0, 30))

        # Form frame
        form_frame = ctk.CTkFrame(self)
        form_frame.pack(pady=20, padx=40, fill="both", expand=True)

        # Username
        ctk.CTkLabel(form_frame, text="Email:", font=ctk.CTkFont(size=12)).pack(pady=(20, 5), anchor="w")
        self.username_entry = ctk.CTkEntry(form_frame, width=400, placeholder_text="your-email@example.com")
        self.username_entry.pack(pady=(0, 15))

        # Password
        ctk.CTkLabel(form_frame, text="Password:", font=ctk.CTkFont(size=12)).pack(pady=(0, 5), anchor="w")
        self.password_entry = ctk.CTkEntry(form_frame, width=400, show="*", placeholder_text="Your password")
        self.password_entry.pack(pady=(0, 15))

        # Region
        ctk.CTkLabel(form_frame, text="Region:", font=ctk.CTkFont(size=12)).pack(pady=(0, 5), anchor="w")
        self.region_var = ctk.StringVar(value="us")
        region_frame = ctk.CTkFrame(form_frame)
        region_frame.pack(pady=(0, 15))

        ctk.CTkRadioButton(
            region_frame,
            text="US",
            variable=self.region_var,
            value="us"
        ).pack(side="left", padx=10)

        ctk.CTkRadioButton(
            region_frame,
            text="EU",
            variable=self.region_var,
            value="eu"
        ).pack(side="left", padx=10)

        # Buttons
        button_frame = ctk.CTkFrame(self)
        button_frame.pack(pady=20, padx=40, fill="x")

        ctk.CTkButton(
            button_frame,
            text="Login",
            command=self._on_login,
            width=180,
            height=40,
            font=ctk.CTkFont(size=14, weight="bold")
        ).pack(side="left", padx=(0, 10))

        ctk.CTkButton(
            button_frame,
            text="Cancel",
            command=self._on_cancel,
            width=180,
            height=40,
            font=ctk.CTkFont(size=14),
            fg_color="gray",
            hover_color="darkgray"
        ).pack(side="left")

    def _on_login(self):
        """Handle login button click"""
        username = self.username_entry.get().strip()
        password = self.password_entry.get().strip()
        region = self.region_var.get()

        if not username or not password:
            messagebox.showerror("Error", "Please enter both email and password")
            return

        self.result = (username, password, region)
        self.destroy()

    def _on_cancel(self):
        """Handle cancel button click"""
        self.result = None
        self.destroy()


class ProgressDialog(ctk.CTkToplevel):
    """Progress dialog for file transfers"""

    def __init__(self, parent, title="Transfer Progress"):
        super().__init__(parent)

        self.title(title)
        self.geometry("700x500")
        self.resizable(True, True)
        self.minsize(600, 450)  # Set minimum size

        # Center the window
        self.update_idletasks()
        x = (self.winfo_screenwidth() // 2) - (700 // 2)
        y = (self.winfo_screenheight() // 2) - (500 // 2)
        self.geometry(f"700x500+{x}+{y}")

        self.transient(parent)
        self.protocol("WM_DELETE_WINDOW", self._on_close)

        self.is_cancelled = False
        self._create_widgets()

    def _create_widgets(self):
        """Create progress widgets"""
        # Title
        self.title_label = ctk.CTkLabel(
            self,
            text="📤 Transferring Files",
            font=ctk.CTkFont(size=20, weight="bold")
        )
        self.title_label.pack(pady=(20, 10))

        # Progress frame
        progress_frame = ctk.CTkFrame(self)
        progress_frame.pack(pady=20, padx=40, fill="both", expand=True)

        # Overall progress
        ctk.CTkLabel(progress_frame, text="Overall Progress:", font=ctk.CTkFont(size=12)).pack(pady=(20, 5), anchor="w")
        self.overall_progress = ctk.CTkProgressBar(progress_frame, width=500)
        self.overall_progress.pack(pady=(0, 5))
        self.overall_progress.set(0)

        self.overall_label = ctk.CTkLabel(progress_frame, text="0%", font=ctk.CTkFont(size=11))
        self.overall_label.pack(pady=(0, 15))

        # Current file
        ctk.CTkLabel(progress_frame, text="Current File:", font=ctk.CTkFont(size=12)).pack(pady=(10, 5), anchor="w")
        self.current_file_label = ctk.CTkLabel(
            progress_frame,
            text="Preparing...",
            font=ctk.CTkFont(size=11),
            text_color="gray"
        )
        self.current_file_label.pack(pady=(0, 15), anchor="w")

        # Transfer stats
        stats_frame = ctk.CTkFrame(progress_frame)
        stats_frame.pack(pady=10, fill="x")

        # Speed
        stat_col1 = ctk.CTkFrame(stats_frame, fg_color="transparent")
        stat_col1.pack(side="left", expand=True, fill="both", padx=10)
        ctk.CTkLabel(stat_col1, text="Speed:", font=ctk.CTkFont(size=11)).pack(anchor="w")
        self.speed_label = ctk.CTkLabel(
            stat_col1,
            text="0 MB/s",
            font=ctk.CTkFont(size=16, weight="bold"),
            text_color="#1f6aa5"
        )
        self.speed_label.pack(anchor="w")

        # Files completed
        stat_col2 = ctk.CTkFrame(stats_frame, fg_color="transparent")
        stat_col2.pack(side="left", expand=True, fill="both", padx=10)
        ctk.CTkLabel(stat_col2, text="Files:", font=ctk.CTkFont(size=11)).pack(anchor="w")
        self.files_label = ctk.CTkLabel(
            stat_col2,
            text="0 / 0",
            font=ctk.CTkFont(size=16, weight="bold"),
            text_color="#1f6aa5"
        )
        self.files_label.pack(anchor="w")

        # ETA
        stat_col3 = ctk.CTkFrame(stats_frame, fg_color="transparent")
        stat_col3.pack(side="left", expand=True, fill="both", padx=10)
        ctk.CTkLabel(stat_col3, text="ETA:", font=ctk.CTkFont(size=11)).pack(anchor="w")
        self.eta_label = ctk.CTkLabel(
            stat_col3,
            text="Calculating...",
            font=ctk.CTkFont(size=16, weight="bold"),
            text_color="#1f6aa5"
        )
        self.eta_label.pack(anchor="w")

        # Cancel button
        self.cancel_button = ctk.CTkButton(
            self,
            text="Cancel",
            command=self._on_close,
            width=200,
            height=40,
            font=ctk.CTkFont(size=14),
            fg_color="gray",
            hover_color="darkgray"
        )
        self.cancel_button.pack(pady=20)

    def update_progress(self, stats: TransferStats, current_file: str = ""):
        """Update progress display"""
        if stats.total_bytes > 0:
            progress = stats.transferred_bytes / stats.total_bytes
            self.overall_progress.set(progress)
            self.overall_label.configure(text=f"{progress * 100:.1f}%")

        self.speed_label.configure(text=stats.format_speed())
        self.files_label.configure(text=f"{stats.files_completed} / {stats.files_total}")

        if current_file:
            self.current_file_label.configure(text=current_file)

        eta = stats.get_eta()
        if eta > 0:
            self.eta_label.configure(text=f"{int(eta)}s")
        else:
            self.eta_label.configure(text="Calculating...")

    def _on_close(self):
        """Handle window close"""
        self.is_cancelled = True
        self.destroy()


class PCloudGUI(ctk.CTk):
    """Main GUI application"""

    def __init__(self):
        super().__init__()

        self.title("pCloud Fast Transfer")
        self.geometry("1000x700")

        # Center the window
        self.update_idletasks()
        x = (self.winfo_screenwidth() // 2) - (1000 // 2)
        y = (self.winfo_screenheight() // 2) - (700 // 2)
        self.geometry(f"1000x700+{x}+{y}")

        self.client: Optional[PCloudClient] = None
        self.current_path = "/"
        self.selected_files: List[dict] = []
        self.duplicate_mode = "rename"  # Default duplicate handling mode

        self._create_widgets()
        self._try_auto_login()

    def _create_widgets(self):
        """Create main window widgets"""
        # Header
        header_frame = ctk.CTkFrame(self, height=80, corner_radius=0)
        header_frame.pack(fill="x", padx=0, pady=0)
        header_frame.pack_propagate(False)

        # Title
        title_label = ctk.CTkLabel(
            header_frame,
            text="☁️ pCloud Fast Transfer",
            font=ctk.CTkFont(size=28, weight="bold")
        )
        title_label.pack(side="left", padx=30, pady=20)

        # Settings button
        self.settings_button = ctk.CTkButton(
            header_frame,
            text="⚙️ Settings",
            command=self._show_settings,
            width=120,
            height=35,
            font=ctk.CTkFont(size=13)
        )
        self.settings_button.pack(side="right", padx=20, pady=20)

        # Login/Logout button
        self.auth_button = ctk.CTkButton(
            header_frame,
            text="🔐 Login",
            command=self._toggle_auth,
            width=120,
            height=35,
            font=ctk.CTkFont(size=13)
        )
        self.auth_button.pack(side="right", padx=10, pady=20)

        # Status label
        self.status_label = ctk.CTkLabel(
            header_frame,
            text="Not connected",
            font=ctk.CTkFont(size=12),
            text_color="gray"
        )
        self.status_label.pack(side="right", padx=10, pady=20)

        # Main content area
        content_frame = ctk.CTkFrame(self)
        content_frame.pack(fill="both", expand=True, padx=20, pady=20)

        # Tab view
        self.tabview = ctk.CTkTabview(content_frame, width=950, height=550)
        self.tabview.pack(fill="both", expand=True)

        # Create tabs
        self.tabview.add("📤 Upload")
        self.tabview.add("📥 Download")
        self.tabview.add("📁 Browse")

        self._create_upload_tab()
        self._create_download_tab()
        self._create_browse_tab()

    def _create_upload_tab(self):
        """Create upload tab"""
        tab = self.tabview.tab("📤 Upload")

        # Instructions
        info_frame = ctk.CTkFrame(tab)
        info_frame.pack(pady=20, padx=20, fill="x")

        ctk.CTkLabel(
            info_frame,
            text="📤 Upload Files to pCloud",
            font=ctk.CTkFont(size=18, weight="bold")
        ).pack(pady=(15, 5))

        ctk.CTkLabel(
            info_frame,
            text="Select files or folders to upload with parallel transfer support",
            font=ctk.CTkFont(size=12),
            text_color="gray"
        ).pack(pady=(0, 15))

        # File selection area
        selection_frame = ctk.CTkFrame(tab)
        selection_frame.pack(pady=10, padx=20, fill="both", expand=True)

        # Selected files list
        ctk.CTkLabel(
            selection_frame,
            text="Selected Files:",
            font=ctk.CTkFont(size=14, weight="bold")
        ).pack(pady=(15, 10), anchor="w")

        # Scrollable frame for file list
        self.upload_files_frame = ctk.CTkScrollableFrame(selection_frame, height=200)
        self.upload_files_frame.pack(pady=(0, 15), padx=10, fill="both", expand=True)

        self.upload_files_list = []

        # Buttons
        button_frame = ctk.CTkFrame(tab)
        button_frame.pack(pady=10, padx=20, fill="x")

        ctk.CTkButton(
            button_frame,
            text="📁 Select Files",
            command=self._select_upload_files,
            width=180,
            height=40,
            font=ctk.CTkFont(size=14)
        ).pack(side="left", padx=(0, 10))

        ctk.CTkButton(
            button_frame,
            text="📂 Select Folder",
            command=self._select_upload_folder,
            width=180,
            height=40,
            font=ctk.CTkFont(size=14)
        ).pack(side="left", padx=(0, 10))

        ctk.CTkButton(
            button_frame,
            text="🗑️ Clear",
            command=self._clear_upload_list,
            width=120,
            height=40,
            font=ctk.CTkFont(size=14),
            fg_color="gray",
            hover_color="darkgray"
        ).pack(side="left", padx=(0, 10))

        # Remote path
        path_frame = ctk.CTkFrame(tab)
        path_frame.pack(pady=10, padx=20, fill="x")

        ctk.CTkLabel(
            path_frame,
            text="Remote Path:",
            font=ctk.CTkFont(size=13)
        ).pack(side="left", padx=(10, 10))

        self.upload_remote_path = ctk.CTkEntry(path_frame, width=300, placeholder_text="/")
        self.upload_remote_path.insert(0, "/")
        self.upload_remote_path.pack(side="left", padx=(0, 10))

        # Upload button
        upload_frame = ctk.CTkFrame(tab)
        upload_frame.pack(pady=15, padx=20, fill="x")

        self.upload_button = ctk.CTkButton(
            upload_frame,
            text="⬆️ Upload Files",
            command=self._start_upload,
            width=300,
            height=50,
            font=ctk.CTkFont(size=16, weight="bold"),
            fg_color="#1f6aa5",
            hover_color="#165a8f"
        )
        self.upload_button.pack()

    def _create_download_tab(self):
        """Create download tab"""
        tab = self.tabview.tab("📥 Download")

        # Instructions
        info_frame = ctk.CTkFrame(tab)
        info_frame.pack(pady=20, padx=20, fill="x")

        ctk.CTkLabel(
            info_frame,
            text="📥 Download Files from pCloud",
            font=ctk.CTkFont(size=18, weight="bold")
        ).pack(pady=(15, 5))

        ctk.CTkLabel(
            info_frame,
            text="Browse your pCloud files and select items to download",
            font=ctk.CTkFont(size=12),
            text_color="gray"
        ).pack(pady=(0, 15))

        # Path navigation
        nav_frame = ctk.CTkFrame(tab)
        nav_frame.pack(pady=10, padx=20, fill="x")

        ctk.CTkLabel(nav_frame, text="Path:", font=ctk.CTkFont(size=13)).pack(side="left", padx=(10, 10))

        self.download_path_entry = ctk.CTkEntry(nav_frame, width=400)
        self.download_path_entry.insert(0, "/")
        self.download_path_entry.pack(side="left", padx=(0, 10))

        ctk.CTkButton(
            nav_frame,
            text="🔄 Refresh",
            command=self._refresh_download_list,
            width=120,
            height=35,
            font=ctk.CTkFont(size=13)
        ).pack(side="left")

        # Files list
        self.download_files_frame = ctk.CTkScrollableFrame(tab, height=280)
        self.download_files_frame.pack(pady=(10, 15), padx=20, fill="both", expand=True)

        # Download options
        options_frame = ctk.CTkFrame(tab)
        options_frame.pack(pady=10, padx=20, fill="x")

        ctk.CTkLabel(
            options_frame,
            text="Download to:",
            font=ctk.CTkFont(size=13)
        ).pack(side="left", padx=(10, 10))

        self.download_local_path = ctk.CTkEntry(options_frame, width=400)
        self.download_local_path.insert(0, str(Path.home() / "Downloads"))
        self.download_local_path.pack(side="left", padx=(0, 10))

        ctk.CTkButton(
            options_frame,
            text="Browse",
            command=self._select_download_folder,
            width=100,
            height=35
        ).pack(side="left")

        # Download button
        download_frame = ctk.CTkFrame(tab)
        download_frame.pack(pady=15, padx=20, fill="x")

        self.download_button = ctk.CTkButton(
            download_frame,
            text="⬇️ Download Selected",
            command=self._start_download,
            width=300,
            height=50,
            font=ctk.CTkFont(size=16, weight="bold"),
            fg_color="#1f6aa5",
            hover_color="#165a8f"
        )
        self.download_button.pack()

    def _create_browse_tab(self):
        """Create browse tab"""
        tab = self.tabview.tab("📁 Browse")

        # Instructions
        info_frame = ctk.CTkFrame(tab)
        info_frame.pack(pady=20, padx=20, fill="x")

        ctk.CTkLabel(
            info_frame,
            text="📁 Browse pCloud Files",
            font=ctk.CTkFont(size=18, weight="bold")
        ).pack(pady=(15, 5))

        ctk.CTkLabel(
            info_frame,
            text="Navigate and manage your pCloud storage",
            font=ctk.CTkFont(size=12),
            text_color="gray"
        ).pack(pady=(0, 15))

        # Path navigation
        nav_frame = ctk.CTkFrame(tab)
        nav_frame.pack(pady=10, padx=20, fill="x")

        ctk.CTkButton(
            nav_frame,
            text="⬆️ Up",
            command=self._go_up_directory,
            width=80,
            height=35
        ).pack(side="left", padx=(10, 10))

        ctk.CTkLabel(nav_frame, text="Path:", font=ctk.CTkFont(size=13)).pack(side="left", padx=(10, 10))

        self.browse_path_label = ctk.CTkLabel(
            nav_frame,
            text="/",
            font=ctk.CTkFont(size=13, weight="bold"),
            text_color="#1f6aa5"
        )
        self.browse_path_label.pack(side="left", padx=(0, 10))

        ctk.CTkButton(
            nav_frame,
            text="🔄 Refresh",
            command=self._refresh_browse,
            width=100,
            height=35
        ).pack(side="right", padx=10)

        # Files list
        self.browse_files_frame = ctk.CTkScrollableFrame(tab, height=380)
        self.browse_files_frame.pack(pady=(10, 15), padx=20, fill="both", expand=True)

    def _try_auto_login(self):
        """Try to auto-login with cached token"""
        try:
            cache_dir = Path.home() / '.pcloud_fast_transfer'
            token_file = cache_dir / 'auth_token.json'

            if token_file.exists():
                with open(token_file, 'r') as f:
                    data = json.load(f)
                    token = data.get('auth_token')
                    region = data.get('region', 'us')

                    if token:
                        self.client = PCloudClient(auth_token=token, region=region, workers=4, duplicate_mode=self.duplicate_mode)
                        self._update_auth_status(True)
                        self._refresh_browse()
        except Exception as e:
            print(f"Auto-login failed: {e}")

    def _toggle_auth(self):
        """Toggle authentication"""
        if self.client:
            # Logout
            self.client = None
            self._update_auth_status(False)
        else:
            # Login
            dialog = AuthDialog(self)
            self.wait_window(dialog)

            if dialog.result:
                username, password, region = dialog.result
                try:
                    self.client = PCloudClient(
                        username=username,
                        password=password,
                        region=region,
                        workers=4,
                        duplicate_mode=self.duplicate_mode
                    )
                    self._update_auth_status(True)
                    self._refresh_browse()
                except Exception as e:
                    messagebox.showerror("Authentication Failed", f"Could not authenticate:\n{e}")
                    self.client = None

    def _update_auth_status(self, authenticated: bool):
        """Update authentication status display"""
        if authenticated:
            self.auth_button.configure(text="🔓 Logout")
            self.status_label.configure(text="✓ Connected", text_color="green")
        else:
            self.auth_button.configure(text="🔐 Login")
            self.status_label.configure(text="Not connected", text_color="gray")

    def _show_settings(self):
        """Show settings dialog"""
        if not self.client:
            messagebox.showinfo("Settings", "Please login first to access settings")
            return

        dialog = ctk.CTkToplevel(self)
        dialog.title("Settings")
        dialog.geometry("550x550")
        dialog.resizable(True, True)
        dialog.minsize(500, 500)  # Set minimum size

        # Center
        dialog.update_idletasks()
        x = (dialog.winfo_screenwidth() // 2) - (550 // 2)
        y = (dialog.winfo_screenheight() // 2) - (550 // 2)
        dialog.geometry(f"550x550+{x}+{y}")

        dialog.transient(self)
        dialog.grab_set()

        # Workers setting
        ctk.CTkLabel(dialog, text="⚙️ Settings", font=ctk.CTkFont(size=20, weight="bold")).pack(pady=20)

        settings_frame = ctk.CTkFrame(dialog)
        settings_frame.pack(pady=20, padx=40, fill="both", expand=True)

        ctk.CTkLabel(settings_frame, text="Parallel Workers:", font=ctk.CTkFont(size=13)).pack(pady=(20, 10), anchor="w")

        workers_var = ctk.IntVar(value=self.client.workers if self.client else 4)
        workers_slider = ctk.CTkSlider(
            settings_frame,
            from_=1,
            to=16,
            number_of_steps=15,
            variable=workers_var,
            width=300
        )
        workers_slider.pack(pady=(0, 5))

        workers_label = ctk.CTkLabel(settings_frame, text=f"{workers_var.get()} workers", font=ctk.CTkFont(size=12))
        workers_label.pack(pady=(0, 20))

        def update_workers_label(value):
            workers_label.configure(text=f"{int(value)} workers")

        workers_slider.configure(command=update_workers_label)

        # Appearance mode
        ctk.CTkLabel(settings_frame, text="Appearance:", font=ctk.CTkFont(size=13)).pack(pady=(10, 10), anchor="w")

        appearance_var = ctk.StringVar(value=ctk.get_appearance_mode())
        appearance_menu = ctk.CTkOptionMenu(
            settings_frame,
            values=["Dark", "Light", "System"],
            variable=appearance_var,
            width=300
        )
        appearance_menu.pack(pady=(0, 20))

        # Duplicate mode
        ctk.CTkLabel(settings_frame, text="Duplicate Handling:", font=ctk.CTkFont(size=13)).pack(pady=(10, 10), anchor="w")

        duplicate_var = ctk.StringVar(value=self.duplicate_mode)
        duplicate_menu = ctk.CTkOptionMenu(
            settings_frame,
            values=["skip", "overwrite", "rename"],
            variable=duplicate_var,
            width=300
        )
        duplicate_menu.pack(pady=(0, 10))

        # Duplicate mode help text
        duplicate_help = ctk.CTkLabel(
            settings_frame,
            text="skip: Skip files that already exist\noverwrite: Replace existing files\nrename: Auto-rename new files (file.txt → file (1).txt)",
            font=ctk.CTkFont(size=10),
            text_color="gray",
            justify="left"
        )
        duplicate_help.pack(pady=(0, 20), anchor="w")

        def save_settings():
            if self.client:
                self.client.workers = workers_var.get()
                self.client.duplicate_mode = duplicate_var.get()
            self.duplicate_mode = duplicate_var.get()
            ctk.set_appearance_mode(appearance_var.get())
            dialog.destroy()

        ctk.CTkButton(
            dialog,
            text="Save",
            command=save_settings,
            width=150,
            height=40
        ).pack(pady=20)

    def _select_upload_files(self):
        """Select files for upload"""
        files = filedialog.askopenfilenames(title="Select files to upload")
        for file in files:
            if file not in self.upload_files_list:
                self.upload_files_list.append(file)
        self._update_upload_list()

    def _select_upload_folder(self):
        """Select folder for upload"""
        folder = filedialog.askdirectory(title="Select folder to upload")
        if folder:
            # Add all files in folder
            for root, dirs, files in os.walk(folder):
                for file in files:
                    file_path = os.path.join(root, file)
                    if file_path not in self.upload_files_list:
                        self.upload_files_list.append(file_path)
            self._update_upload_list()

    def _update_upload_list(self):
        """Update upload files list display"""
        # Clear existing widgets
        for widget in self.upload_files_frame.winfo_children():
            widget.destroy()

        # Add file entries
        for i, file_path in enumerate(self.upload_files_list):
            file_frame = ctk.CTkFrame(self.upload_files_frame)
            file_frame.pack(fill="x", pady=2, padx=5)

            file_name = Path(file_path).name
            file_size = Path(file_path).stat().st_size
            size_str = self._format_size(file_size)

            ctk.CTkLabel(
                file_frame,
                text=f"📄 {file_name}",
                font=ctk.CTkFont(size=11),
                anchor="w"
            ).pack(side="left", padx=10, fill="x", expand=True)

            ctk.CTkLabel(
                file_frame,
                text=size_str,
                font=ctk.CTkFont(size=10),
                text_color="gray"
            ).pack(side="left", padx=10)

            remove_btn = ctk.CTkButton(
                file_frame,
                text="✕",
                width=30,
                height=25,
                command=lambda idx=i: self._remove_upload_file(idx),
                fg_color="gray",
                hover_color="darkred"
            )
            remove_btn.pack(side="left", padx=5)

    def _remove_upload_file(self, index: int):
        """Remove file from upload list"""
        if 0 <= index < len(self.upload_files_list):
            self.upload_files_list.pop(index)
            self._update_upload_list()

    def _clear_upload_list(self):
        """Clear upload files list"""
        self.upload_files_list.clear()
        self._update_upload_list()

    def _start_upload(self):
        """Start upload process"""
        if not self.client:
            messagebox.showerror("Error", "Please login first")
            return

        if not self.upload_files_list:
            messagebox.showwarning("No Files", "Please select files to upload")
            return

        remote_path = self.upload_remote_path.get().strip() or "/"

        # Start upload in thread
        progress_dialog = ProgressDialog(self, "Upload Progress")

        def upload_task():
            try:
                total_size = sum(Path(f).stat().st_size for f in self.upload_files_list)
                stats = TransferStats(total_bytes=total_size, files_total=len(self.upload_files_list))

                def progress_callback(s):
                    if not progress_dialog.is_cancelled:
                        self.after(0, lambda: progress_dialog.update_progress(s))

                uploaded, skipped, failed = self.client.upload_files(
                    self.upload_files_list,
                    remote_path,
                    progress_callback
                )

                if not progress_dialog.is_cancelled:
                    self.after(0, lambda: progress_dialog.destroy())
                    self.after(0, lambda: messagebox.showinfo(
                        "Upload Complete",
                        f"Upload completed!\n\nUploaded: {uploaded}\nSkipped: {skipped}\nFailed: {failed}"
                    ))

            except Exception as e:
                self.after(0, lambda: progress_dialog.destroy())
                self.after(0, lambda: messagebox.showerror("Upload Error", f"Upload failed:\n{e}"))

        thread = threading.Thread(target=upload_task, daemon=True)
        thread.start()

    def _refresh_download_list(self):
        """Refresh download files list"""
        if not self.client:
            messagebox.showerror("Error", "Please login first")
            return

        path = self.download_path_entry.get().strip() or "/"
        self._load_remote_files(path, self.download_files_frame, download_mode=True)

    def _refresh_browse(self):
        """Refresh browse files list"""
        if not self.client:
            return

        self._load_remote_files(self.current_path, self.browse_files_frame)
        self.browse_path_label.configure(text=self.current_path)

    def _load_remote_files(self, path: str, frame, download_mode=False):
        """Load remote files into frame"""
        # Clear existing widgets
        for widget in frame.winfo_children():
            widget.destroy()

        try:
            contents = self.client.list_folder(path)

            if not contents:
                ctk.CTkLabel(
                    frame,
                    text="📭 Empty folder",
                    font=ctk.CTkFont(size=14),
                    text_color="gray"
                ).pack(pady=40)
                return

            for item in contents:
                self._create_file_item(frame, item, path, download_mode)

        except Exception as e:
            ctk.CTkLabel(
                frame,
                text=f"❌ Error loading files:\n{e}",
                font=ctk.CTkFont(size=12),
                text_color="red"
            ).pack(pady=40)

    def _create_file_item(self, parent, item: dict, current_path: str, download_mode=False):
        """Create a file item widget"""
        item_frame = ctk.CTkFrame(parent)
        item_frame.pack(fill="x", pady=2, padx=5)

        is_folder = item.get('isfolder', False)
        name = item.get('name', 'Unknown')
        size = item.get('size', 0)

        # Icon and name
        icon = "📁" if is_folder else "📄"
        name_label = ctk.CTkLabel(
            item_frame,
            text=f"{icon} {name}",
            font=ctk.CTkFont(size=12),
            anchor="w"
        )
        name_label.pack(side="left", padx=10, fill="x", expand=True)

        # Size
        if not is_folder:
            size_str = self._format_size(size)
            ctk.CTkLabel(
                item_frame,
                text=size_str,
                font=ctk.CTkFont(size=10),
                text_color="gray"
            ).pack(side="left", padx=10)

        # Action button
        if download_mode and not is_folder:
            check_var = ctk.BooleanVar()
            check_btn = ctk.CTkCheckBox(
                item_frame,
                text="",
                variable=check_var,
                width=30,
                command=lambda: self._toggle_file_selection(item, current_path, check_var.get())
            )
            check_btn.pack(side="left", padx=5)
        elif is_folder and not download_mode:
            open_btn = ctk.CTkButton(
                item_frame,
                text="Open",
                width=80,
                height=28,
                command=lambda: self._open_folder(f"{current_path.rstrip('/')}/{name}")
            )
            open_btn.pack(side="left", padx=5)

    def _toggle_file_selection(self, item: dict, path: str, selected: bool):
        """Toggle file selection for download"""
        file_info = (f"{path.rstrip('/')}/{item['name']}", item)

        if selected:
            if file_info not in self.selected_files:
                self.selected_files.append(file_info)
        else:
            if file_info in self.selected_files:
                self.selected_files.remove(file_info)

    def _open_folder(self, path: str):
        """Open a folder in browse view"""
        self.current_path = path
        self._refresh_browse()

    def _go_up_directory(self):
        """Go up one directory"""
        if self.current_path != "/":
            parent = str(Path(self.current_path).parent)
            if parent == ".":
                parent = "/"
            self.current_path = parent
            self._refresh_browse()

    def _select_download_folder(self):
        """Select local folder for downloads"""
        folder = filedialog.askdirectory(title="Select download location")
        if folder:
            self.download_local_path.delete(0, tk.END)
            self.download_local_path.insert(0, folder)

    def _start_download(self):
        """Start download process"""
        if not self.client:
            messagebox.showerror("Error", "Please login first")
            return

        if not self.selected_files:
            messagebox.showwarning("No Files", "Please select files to download")
            return

        local_path = self.download_local_path.get().strip()

        # Prepare download list
        download_list = []
        for remote_path, item in self.selected_files:
            local_file = os.path.join(local_path, item['name'])
            download_list.append((remote_path, local_file))

        # Start download in thread
        progress_dialog = ProgressDialog(self, "Download Progress")

        def download_task():
            try:
                downloaded, skipped, failed = self.client.download_files(download_list)

                if not progress_dialog.is_cancelled:
                    self.after(0, lambda: progress_dialog.destroy())
                    self.after(0, lambda: messagebox.showinfo(
                        "Download Complete",
                        f"Download completed!\n\nDownloaded: {downloaded}\nSkipped: {skipped}\nFailed: {failed}"
                    ))

            except Exception as e:
                self.after(0, lambda: progress_dialog.destroy())
                self.after(0, lambda: messagebox.showerror("Download Error", f"Download failed:\n{e}"))

        thread = threading.Thread(target=download_task, daemon=True)
        thread.start()

    def _format_size(self, size: int) -> str:
        """Format size as human-readable string"""
        for unit in ['B', 'KB', 'MB', 'GB', 'TB']:
            if size < 1024.0:
                return f"{size:.1f} {unit}"
            size /= 1024.0
        return f"{size:.1f} PB"


def main():
    """Main entry point"""
    app = PCloudGUI()
    app.mainloop()


if __name__ == '__main__':
    main()
