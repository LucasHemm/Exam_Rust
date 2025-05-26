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

/// Spawns an external download process (yt-dlp) with given URL and quality,
/// reporting progress updates via the provided channel.
pub async fn spawn_download(
    url: String,
    quality: String,
    download_folder: String,
    progress_tx: UnboundedSender<f32>, // ✅ channel to send percentage progress
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Determine binary name based on target OS
    let bin = if cfg!(target_os = "windows") { "yt-dlp.exe" } else { "yt-dlp" };
    // Load the embedded binary data from assets/
    let data = Asset::get(bin).ok_or("Missing yt-dlp")?;
    // Create a temp file path for the binary
    let tmp = std::env::temp_dir().join(bin);
    // If the binary is not already present, write it out
    if !tmp.exists() {
        let mut f = File::create(&tmp)?;
        f.write_all(&data.data)?;
        // On Unix, make the temp file executable
        #[cfg(unix)]
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }

    // Build command-line arguments for yt-dlp
    let mut args = vec!["-f".to_owned(), format!("best[height<={}]", match quality.as_str() {
        "1080p" => "1080",
        "720p" => "720",
        "480p" => "480",
        "360p" => "360",
        "Audio Only" => "bestaudio",
        _ => "best",
    })];

    // Configure custom progress output template
    args.push("--progress-template".to_owned());
    args.push("downloaded_bytes:%(progress._percent_str)s".to_owned());
    args.push("--newline".to_owned());

    // Specify output file pattern and directory
    args.push("-o".to_owned());
    args.push(format!("{}/%(title)s.%(ext)s", download_folder));
    // Finally, add the URL to download
    args.push(url);

    // Spawn the yt-dlp child process, capturing stdout and stderr
    let mut child = Command::new(tmp)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Read process stdout line by line to monitor progress
    let out = child.stdout.take().unwrap();
    let mut lines = BufReader::new(out).lines();
    while let Some(line) = lines.next_line().await? {
        // Debug print each line from yt-dlp
        println!("DBG> {}", line);
        // Parse percentage from the progress line and send it through the channel
        if let Some(pct) = parse_progress_from_line(&line) {
            let _ = progress_tx.send(pct);
        }
    }
    Ok(())
}
