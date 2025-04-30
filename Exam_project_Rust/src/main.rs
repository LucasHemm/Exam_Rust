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
use egui::{ColorImage, TextureOptions};

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
        Box::new(|_cc| Box::new(MyApp::default())),
    )
}

#[derive(Clone)]
enum DownloadStatus {
    Downloading,
    Done,
}

fn parse_progress_from_line(line: &str) -> Option<f32> {
    if let Some(rest) = line.strip_prefix("downloaded_bytes:") {
        // rest might be " 62.0%"
        let trimmed = rest.trim();
        if let Some(number) = trimmed.strip_suffix('%') {
            // number is "62.0"
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
    progress_rx: Option<UnboundedReceiver<(String, f32)>>,
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
            progress_rx: None,
        }
    }
}

impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // 1️⃣ Process incoming progress updates on main thread
        if let Some(rx) = &mut self.progress_rx {
            while let Ok((id, prog)) = rx.try_recv() {
                if let Some(task) = self.downloads.iter_mut().find(|t| t.video_id == id) {
                    task.progress = prog;
                    if prog >= 1.0 {
                        task.status = DownloadStatus::Done;
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
                                });
                            });
                        });
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

                    // 4.1 Push a new task
                    self.downloads.push(DownloadTask {
                        title: title.clone(),
                        video_id: video_id.clone(),
                        status: DownloadStatus::Downloading,
                        progress: 0.0,
                    });

                    // 4.2 Fetch thumbnail in background
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

                    // 4.3 Setup progress channel
                    let (tx, rx) = unbounded_channel();
                    self.progress_rx = Some(rx);

                    // 4.4 Spawn download with real progress
                    RUNTIME
                        .get()
                        .unwrap()
                        .spawn(spawn_download(
                            url.clone(),
                            quality.clone(),
                            folder.clone(),
                            tx,
                            video_id.clone(),
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
    progress_tx: UnboundedSender<(String, f32)>,
    video_id: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. extract embedded yt-dlp
    let bin = if cfg!(target_os = "windows") { "yt-dlp.exe" } else { "yt-dlp" };
    let data = Asset::get(bin).ok_or("Missing yt-dlp")?;
    let tmp = std::env::temp_dir().join(bin);
    if !tmp.exists() {
        let mut f = File::create(&tmp)?;
        f.write_all(&data.data)?;
        #[cfg(unix)]
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }

    // 2. build args
    let mut args = vec!["-f".to_owned(), format!("best[height<={}]", match quality.as_str() {
        "1080p" => "1080",
        "720p" => "720",
        "480p" => "480",
        "360p" => "360",
        "Audio Only" => "bestaudio",
        _ => "best",
    })];

    // new, correct:
    args.push("--progress-template".to_owned());
    // this expands to e.g. "downloaded_bytes:  62.0%"
    // by pulling the _percent_str from the progress dict
    args.push("downloaded_bytes:%(progress._percent_str)s".to_owned());
    args.push("--newline".to_owned());


    args.push("-o".to_owned());
    args.push(format!("{}/%(title)s.%(ext)s", download_folder));
    args.push(url);

    // 3. spawn process and read its stdout
    let mut child = Command::new(tmp)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    let out = child.stdout.take().unwrap();
    let mut lines = BufReader::new(out).lines();
    while let Some(line) = lines.next_line().await? {
        println!("DBG> {}", line);

        if let Some(pct) = parse_progress_from_line(&line) {
            let _ = progress_tx.send((video_id.clone(), pct));
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
