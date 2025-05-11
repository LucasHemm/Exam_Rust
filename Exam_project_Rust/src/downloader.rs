use std::{fs::File, io::Write, process::Stdio};
use rust_embed::RustEmbed;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc::UnboundedSender,
};
use crate::progress::parse_progress_from_line;

#[derive(RustEmbed)]
#[folder = "assets/"]
pub struct Asset;

pub async fn spawn_download(
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