use eframe::{egui, App, Frame};
use once_cell::sync::OnceCell;
use rfd::FileDialog;
use rust_embed::RustEmbed;
use std::{collections::HashMap, fs::File, io::Write, process::Stdio, sync::Arc, time::Instant};
use tokio::{process::Command, runtime::Runtime};
use egui::{ColorImage, TextureOptions};

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Asset;

static RUNTIME: OnceCell<Arc<Runtime>> = OnceCell::new();

fn main() -> Result<(), eframe::Error> {
    let rt = Arc::new(Runtime::new().unwrap());
    RUNTIME.set(rt).unwrap();

    let options = eframe::NativeOptions::default();
    eframe::run_native("YouTube Downloader", options, Box::new(|_cc| Box::new(MyApp::default())))
}

#[derive(Clone)]
enum DownloadStatus {
    Downloading,
    Done,
}


struct DownloadTask {
    title: String,
    video_id: String,
    status: DownloadStatus,
    progress: f32,
    started: Instant,
}

struct MyApp {
    url_input: String,
    download_folder: String,
    selected_quality: String,
    quality_options: Vec<String>,
    status: String,
    downloads: Vec<DownloadTask>,
    thumbnails: HashMap<String, egui::TextureHandle>,
    thumbnail_results: Arc<std::sync::Mutex<Vec<(String, ColorImage)>>>,
}

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
            status: String::new(),
            downloads: Vec::new(),
            thumbnails: HashMap::new(),
            thumbnail_results: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {

        let mut pending = self.thumbnail_results.lock().unwrap();
        for (video_id, image) in pending.drain(..) {
            let tex = ctx.load_texture(&video_id, image, TextureOptions::default());
            self.thumbnails.insert(video_id, tex);
        }
        drop(pending); // unlock mutex early


        // Right-side download panel
        egui::SidePanel::right("downloads_panel").show(ctx, |ui| {
            ui.heading("Active Downloads");
            ui.separator();

            for task in &mut self.downloads {
                let progress = task.progress;
                let status = match &task.status {
                    DownloadStatus::Downloading => "⬇️ Downloading",
                    DownloadStatus::Done => "✅ Done",
                };

                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        if let Some(thumbnail) = self.thumbnails.get(&task.video_id) {
                            ui.image(thumbnail);
                        }
                        ui.vertical(|ui| {
                            ui.label(&task.title);
                            ui.label(status);
                            ui.add(egui::ProgressBar::new(progress).show_percentage());
                        });
                    });
                });

                // Simulate progress
                if matches!(task.status, DownloadStatus::Downloading) {
                    let elapsed = task.started.elapsed().as_secs_f32();
                    task.progress = (elapsed / 10.0).min(1.0);
                    if task.progress >= 1.0 {
                        task.status = DownloadStatus::Done;
                    }
                }
            }
        });

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("YouTube Downloader");

            ui.label("Paste YouTube video URL:");
            ui.text_edit_singleline(&mut self.url_input);

            ui.horizontal(|ui| {
                ui.label("Download folder:");
                ui.text_edit_singleline(&mut self.download_folder);
                if ui.button("Browse…").clicked() {
                    if let Some(folder) =
                        FileDialog::new().set_directory(&self.download_folder).pick_folder()
                    {
                        self.download_folder = folder.display().to_string();
                    }
                }
            });

            ui.label("Select Video Quality:");
            egui::ComboBox::from_label("")
                .selected_text(&self.selected_quality)
                .show_ui(ui, |ui| {
                    for quality in &self.quality_options {
                        ui.selectable_value(&mut self.selected_quality, quality.clone(), quality);
                    }
                });

            if ui.button("Download").clicked() {
                let url = self.url_input.trim().to_string();
                let quality = self.selected_quality.clone();
                let folder = self.download_folder.clone();

                if let Some(video_id) = extract_video_id(&url) {
                    let title = format!("Video ID: {}", video_id);

                    self.downloads.push(DownloadTask {
                        title: title.clone(),
                        video_id: video_id.clone(),
                        status: DownloadStatus::Downloading,
                        progress: 0.0,
                        started: Instant::now(),
                    });

                    // Fetch thumbnail in background thread
                    let id_clone = video_id.clone();
                    let ctx_clone = ctx.clone();
                    let result_target = self.thumbnail_results.clone();

                    let rt = RUNTIME.get().unwrap().clone();
                    rt.spawn_blocking(move || {
                        if let Some(image) = fetch_thumbnail(&id_clone) {
                            if let Ok(mut pending) = result_target.lock() {
                                pending.push((id_clone.clone(), image));
                            }
                            ctx_clone.request_repaint(); // safely ask for GUI update
                        }
                    });

                    // Start download task
                    let rt = RUNTIME.get().unwrap().clone();
                    rt.spawn(async move {
                        if let Err(e) = spawn_download(&url, &quality, &folder).await {
                            eprintln!("Download error: {}", e);
                        }
                    });
                }

                self.url_input.clear();
            }

            ui.label(&self.status);
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

fn extract_video_id(url: &str) -> Option<String> {
    url.split("v=").nth(1)
        .and_then(|s| s.split('&').next())
        .map(|s| s.to_string())
}

fn fetch_thumbnail(video_id: &str) -> Option<egui::ColorImage> {
    let url = format!("https://img.youtube.com/vi/{}/hqdefault.jpg", video_id);
    let resp = reqwest::blocking::get(&url).ok()?.bytes().ok()?;
    let img = image::load_from_memory(&resp).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, &img))
}

async fn spawn_download(
    url: &str,
    quality: &str,
    download_folder: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let file_name = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    let bin_data = Asset::get(file_name).ok_or("Missing embedded yt-dlp")?;
    let tmp_path = std::env::temp_dir().join(file_name);

    if !tmp_path.exists() {
        let mut file = File::create(&tmp_path)?;
        file.write_all(&bin_data.data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    let mut args: Vec<String> = vec!["-f".to_owned()];
    let format_expr = match quality {
        "1080p" => "best[height<=1080]",
        "720p" => "best[height<=720]",
        "480p" => "best[height<=480]",
        "360p" => "best[height<=360]",
        "Audio Only" => "bestaudio",
        _ => "best",
    };
    args.push(format_expr.to_owned());

    args.push("-o".to_owned());
    args.push(format!("{}/%(title)s.%(ext)s", download_folder));
    args.push(url.to_owned());

    Command::new(tmp_path)
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    Ok(())
}
