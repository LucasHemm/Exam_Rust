use eframe::{egui, App, Frame};
use once_cell::sync::OnceCell;
use rfd::FileDialog;
use rust_embed::RustEmbed;
use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    process::Stdio,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    runtime::Runtime,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};
use egui::{ColorImage, TextureOptions, Visuals};

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Asset;

static RUNTIME: OnceCell<Arc<Runtime>> = OnceCell::new();

fn main() -> Result<(), eframe::Error> {
    let rt = Arc::new(Runtime::new().unwrap());
    RUNTIME.set(rt).unwrap();

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "YouTube Downloader",
        options,
        Box::new(|cc| {

            let visuals = Visuals::dark();                        // ← switch to dark base
            

            cc.egui_ctx.set_visuals(visuals);

            Box::new(MyApp::default())
        }),
    )
}


#[derive(Clone)]
enum DownloadStatus {
    Downloading,
    Done,
}

fn parse_progress_from_line(line: &str) -> Option<f32> {
    if let Some(rest) = line.strip_prefix("downloaded_bytes:") {
        let trimmed = rest.trim();
        if let Some(number) = trimmed.strip_suffix('%') {
            if let Ok(v) = number.trim().parse::<f32>() {
                return Some(v / 100.0);
            }
        }
    }
    None
}

struct DownloadTask {
    title: String,
    video_id: String,
    status: DownloadStatus,
    progress: f32,
}

struct MyApp {
    url_input: String,
    download_folder: String,
    selected_quality: String,
    quality_options: Vec<String>,
    downloads: Vec<DownloadTask>,
    thumbnails: HashMap<String, egui::TextureHandle>,
    thumbnail_results: Arc<Mutex<Vec<(String, ColorImage)>>>,
    progress_rxs: HashMap<String, UnboundedReceiver<f32>>, // ✅ changed from Option<Receiver>
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
            downloads: Vec::new(),
            thumbnails: HashMap::new(),
            thumbnail_results: Arc::new(Mutex::new(Vec::new())),
            progress_rxs: HashMap::new(), // ✅
        }
    }
}

impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // ✅ 1️⃣ Check all progress channels
        for (id, rx) in self.progress_rxs.iter_mut() {
            while let Ok(prog) = rx.try_recv() {
                if let Some(task) = self.downloads.iter_mut().find(|t| &t.video_id == id) {
                    // only increase, never go backwards
                    if prog > task.progress {
                        task.progress = prog;
                        if task.progress >= 1.0 {
                            task.status = DownloadStatus::Done;
                        }
                    }
                }
            }
        }

        // 2️⃣ Process fetched thumbnails
        {
            let mut pending = self.thumbnail_results.lock().unwrap();
            for (vid, img) in pending.drain(..) {
                let tex = ctx.load_texture(&vid, img, TextureOptions::default());
                self.thumbnails.insert(vid, tex);
            }
        }

        // 3️⃣ Right-side download panel
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
                                if let Some(tex) = self.thumbnails.get(&task.video_id) {
                                    ui.image(tex);
                                }
                                ui.vertical(|ui| {
                                    ui.label(&task.title);
                                    ui.label(status_text);
                                    ui.add(egui::ProgressBar::new(task.progress).show_percentage());
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

                                            // ✅ Remove Button
                                            if ui.add(egui::Button::new("❌").fill(egui::Color32::RED)).clicked() {
                                                to_remove.push(task.video_id.clone());
                                            }
                                        });
                                    }
                                });
                            });
                        });
                    }

                    // ✅ Actually remove tasks after iterating
                    if !to_remove.is_empty() {
                        self.downloads.retain(|t| !to_remove.contains(&t.video_id));
                        for id in to_remove {
                            self.progress_rxs.remove(&id);
                        }
                    }
                });
        });

        // 4️⃣ Main panel
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
                    for q in &self.quality_options {
                        ui.selectable_value(&mut self.selected_quality, q.clone(), q);
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
                    });

                    // Spawn thumbnail fetcher
                    {
                        let id_c = video_id.clone();
                        let results = Arc::clone(&self.thumbnail_results);
                        let ctx_c = ctx.clone();
                        RUNTIME
                            .get()
                            .unwrap()
                            .spawn_blocking(move || {
                                if let Some(img) = fetch_thumbnail(&id_c) {
                                    results.lock().unwrap().push((id_c.clone(), img));
                                    ctx_c.request_repaint();
                                }
                            });
                    }

                    // ✅ Create a new progress channel per video_id
                    let (tx, rx) = unbounded_channel();
                    self.progress_rxs.insert(video_id.clone(), rx);

                    // Launch yt-dlp download
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

                self.url_input.clear();
            }
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Extracts YouTube “v=” ID
fn extract_video_id(url: &str) -> Option<String> {
    url.split("v=").nth(1).and_then(|s| s.split('&').next()).map(|s| s.to_string())
}

/// Downloads yt-dlp + parses progress
async fn spawn_download(
    url: String,
    quality: String,
    download_folder: String,
    progress_tx: UnboundedSender<f32>, // ✅ just progress
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bin = if cfg!(target_os = "windows") { "yt-dlp.exe" } else { "yt-dlp" };
    let data = Asset::get(bin).ok_or("Missing yt-dlp")?;
    let tmp = std::env::temp_dir().join(bin);
    if !tmp.exists() {
        let mut f = File::create(&tmp)?;
        f.write_all(&data.data)?;
        #[cfg(unix)]
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }

    let mut args = vec!["-f".to_owned(), format!("best[height<={}]", match quality.as_str() {
        "1080p" => "1080",
        "720p" => "720",
        "480p" => "480",
        "360p" => "360",
        "Audio Only" => "bestaudio",
        _ => "best",
    })];

    args.push("--progress-template".to_owned());
    args.push("downloaded_bytes:%(progress._percent_str)s".to_owned());
    args.push("--newline".to_owned());

    args.push("-o".to_owned());
    args.push(format!("{}/%(title)s.%(ext)s", download_folder));
    args.push(url);

    let mut child = Command::new(tmp)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let out = child.stdout.take().unwrap();
    let mut lines = BufReader::new(out).lines();
    while let Some(line) = lines.next_line().await? {
        println!("DBG> {}", line);
        if let Some(pct) = parse_progress_from_line(&line) {
            let _ = progress_tx.send(pct);
        }
    }
    Ok(())
}

/// Fetches thumbnail image
fn fetch_thumbnail(video_id: &str) -> Option<ColorImage> {
    let url = format!("https://img.youtube.com/vi/{}/hqdefault.jpg", video_id);
    let resp = reqwest::blocking::get(&url).ok()?.bytes().ok()?;
    let img = image::load_from_memory(&resp).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, &img))
}
