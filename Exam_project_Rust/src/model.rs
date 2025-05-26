/// Represents the current state of a download
#[derive(Clone)]
pub enum DownloadStatus {
    /// Download is in progress
    Downloading,
    /// Download has completed successfully
    Done,
}

/// Data structure for tracking a download task in the UI
pub struct DownloadTask {
    /// Human-readable title (e.g., video title or ID)
    pub title: String,
    /// Unique video identifier (extracted from YouTube URL)
    pub video_id: String,
    /// Current status of the download (Downloading or Done)
    pub status: DownloadStatus,
    /// Progress percentage (0.0 to 1.0)
    pub progress: f32,
}