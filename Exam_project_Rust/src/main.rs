//! Main application for the YouTube Downloader GUI

// Thumbnail fetching module
mod thumbnail;
// External downloader spawning logic (yt-dlp)
mod downloader;
// Progress parsing utilities
mod progress;
// Data models for download tasks and status
mod model;
use model::{DownloadTask, DownloadStatus};

// Asynchronous download function from downloader module
use downloader::spawn_download;

// eframe/egui for GUI application framework
use eframe::{egui, App, Frame};
// OnceCell for single-time runtime initialization
use once_cell::sync::OnceCell;
// FileDialog for folder selection dialogs
use rfd::FileDialog;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::{
    runtime::Runtime,
    sync::mpsc::{unbounded_channel, UnboundedReceiver},
};
use egui::{ColorImage, TextureOptions, Visuals};

// Global Tokio runtime stored in a OnceCell for lazy init
static RUNTIME: OnceCell<Arc<Runtime>> = OnceCell::new();

/// Program entry point: initializes runtime and launches GUI
fn main() -> Result<(), eframe::Error> {
    // Create a new Tokio runtime and store it globally
    let rt = Arc::new(Runtime::new().unwrap());
    RUNTIME.set(rt).unwrap();

    // Configure default native options for egui window
    let options = eframe::NativeOptions::default();
    // Run the application
    eframe::run_native(
        "YouTube Downloader",
        options,
        Box::new(|cc| {
            // Use dark theme visuals
            let visuals = Visuals::dark();
            cc.egui_ctx.set_visuals(visuals);
            // Instantiate default app state
            Box::new(MyApp::default())
        }),
    )
}

/// Application state for the GUI
struct MyApp {
    /// Input field for YouTube URL
    url_input: String,
    /// Destination folder for downloads
    download_folder: String,
    /// Selected quality option
    selected_quality: String,
    /// Available quality options
    quality_options: Vec<String>,
    /// List of active download tasks
    downloads: Vec<DownloadTask>,
    /// Cached textures for video thumbnails
    thumbnails: HashMap<String, egui::TextureHandle>,
    /// Incoming thumbnail fetch results (video_id, image)
    thumbnail_results: Arc<Mutex<Vec<(String, ColorImage)>>>,
    /// Progress update channels for each video_id
    progress_rxs: HashMap<String, UnboundedReceiver<f32>>,
}

/// Default initial state for MyApp
impl Default for MyApp {
    fn default() -> Self {
        Self {
            url_input: String::new(),
            download_folder: "./downloads".to_string(),
            selected_quality: "720p".to_string(),
            quality_options: vec![
                "1080p".to_string(),
                "720p".to_string(),
                "480p".to_string(),
                "360p".to_string(),
                "Audio Only".to_string(),
            ],
            downloads: Vec::new(),
            thumbnails: HashMap::new(),
            thumbnail_results: Arc::new(Mutex::new(Vec::new())),
            progress_rxs: HashMap::new(),
        }
    }
}

/// GUI update loop: called each frame to redraw and handle interactions
impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // 1️⃣ Poll all progress channels for updates
        for (id, rx) in self.progress_rxs.iter_mut() {
            while let Ok(prog) = rx.try_recv() {
                if let Some(task) = self.downloads.iter_mut().find(|t| &t.video_id == id) {
                    // Only update if progress increased
                    if prog > task.progress {
                        task.progress = prog;
                        // Mark as done at 100%
                        if task.progress >= 1.0 {
                            task.status = DownloadStatus::Done;
                        }
                    }
                }
            }
        }

        // 2️⃣ Handle completed thumbnail fetches
        {
            let mut pending = self.thumbnail_results.lock().unwrap();
            for (vid, img) in pending.drain(..) {
                // Load image into egui texture and cache it
                let tex = ctx.load_texture(&vid, img, TextureOptions::default());
                self.thumbnails.insert(vid, tex);
            }
        }

        // 3️⃣ Right-side panel: list of active downloads
        egui::SidePanel::right("downloads_panel").show(ctx, |ui| {
            ui.heading("Active Downloads");
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let mut to_remove = vec![];

                    for task in &self.downloads {
                        let status_text = match task.status {
                            DownloadStatus::Downloading => "⬇️ Downloading",
                            DownloadStatus::Done => "✅ Done",
                        };
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                // Show thumbnail if available
                                if let Some(tex) = self.thumbnails.get(&task.video_id) {
                                    ui.image(tex);
                                }
                                ui.vertical(|ui| {
                                    ui.label(&task.title);
                                    ui.label(status_text);
                                    ui.add(egui::ProgressBar::new(task.progress).show_percentage());
                                    // When done, provide folder and remove options
                                    if matches!(task.status, DownloadStatus::Done) {
                                        ui.horizontal(|ui| {
                                            if ui.button("Open Folder").clicked() {
                                                let folder = self.download_folder.clone();
                                                std::thread::spawn(move || {
                                                    #[cfg(target_os = "windows")]
                                                    {
                                                        let _ = std::process::Command::new("explorer").arg(folder).spawn();
                                                    }
                                                    #[cfg(target_os = "macos")]
                                                    {
                                                        let _ = std::process::Command::new("open").arg(folder).spawn();
                                                    }
                                                    #[cfg(all(unix, not(target_os = "macos")))]
                                                    {
                                                        let _ = std::process::Command::new("xdg-open").arg(folder).spawn();
                                                    }
                                                });
                                            }
                                            // Queue removal of finished task
                                            if ui.add(egui::Button::new("❌").fill(egui::Color32::RED)).clicked() {
                                                to_remove.push(task.video_id.clone());
                                            }
                                        });
                                    }
                                });
                            });
                        });
                    }

                    // Remove tasks and their channels after iteration
                    if !to_remove.is_empty() {
                        self.downloads.retain(|t| !to_remove.contains(&t.video_id));
                        for id in to_remove {
                            self.progress_rxs.remove(&id);
                        }
                    }
                });
        });

        // 4️⃣ Main panel: inputs for URL, folder, quality, and Download button
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("YouTube Downloader");

            // URL input field
            ui.label("Paste YouTube video URL:");
            ui.text_edit_singleline(&mut self.url_input);

            // Folder selection
            ui.horizontal(|ui| {
                ui.label("Download folder:");
                ui.text_edit_singleline(&mut self.download_folder);
                if ui.button("Browse…").clicked() {
                    if let Some(folder) = FileDialog::new().set_directory(&self.download_folder).pick_folder() {
                        self.download_folder = folder.display().to_string();
                    }
                }
            });

            // Quality dropdown
            ui.label("Select Video Quality:");
            egui::ComboBox::from_label("")
                .selected_text(&self.selected_quality)
                .show_ui(ui, |ui| {
                    for q in &self.quality_options {
                        ui.selectable_value(&mut self.selected_quality, q.clone(), q);
                    }
                });

            // Download button logic
            if ui.button("Download").clicked() {
                let url = self.url_input.trim().to_string();
                let quality = self.selected_quality.clone();
                let folder = self.download_folder.clone();

                // Extract video ID from URL
                if let Some(video_id) = extract_video_id(&url) {
                    let title = format!("Video ID: {}", video_id);

                    // Add new download task
                    self.downloads.push(DownloadTask {
                        title: title.clone(),
                        video_id: video_id.clone(),
                        status: DownloadStatus::Downloading,
                        progress: 0.0,
                    });

                    // Spawn thumbnail fetch in blocking task
                    {
                        let id_c = video_id.clone();
                        let results = Arc::clone(&self.thumbnail_results);
                        let ctx_c = ctx.clone();
                        RUNTIME
                            .get()
                            .unwrap()
                            .spawn_blocking(move || {
                                if let Some(img) = thumbnail::fetch_thumbnail(&id_c) {
                                    results.lock().unwrap().push((id_c.clone(), img));
                                    ctx_c.request_repaint();
                                }
                            });
                    }

                    // Create progress channel and insert receiver
                    let (tx, rx) = unbounded_channel();
                    self.progress_rxs.insert(video_id.clone(), rx);

                    // Launch asynchronous download task
                    RUNTIME
                        .get()
                        .unwrap()
                        .spawn(spawn_download(
                            url.clone(),
                            quality.clone(),
                            folder.clone(),
                            tx,
                        ));
                }

                // Clear URL input after starting download
                self.url_input.clear();
            }
        });

        // Request periodic repaint for progress updates
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Extracts the 'v' parameter from a YouTube URL
fn extract_video_id(url: &str) -> Option<String> {
    url.split("v=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .map(|s| s.to_string())
}
