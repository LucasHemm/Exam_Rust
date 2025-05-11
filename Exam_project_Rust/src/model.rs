#[derive(Clone)]
pub enum DownloadStatus {
    Downloading,
    Done,
}

pub struct DownloadTask {
    pub title: String,
    pub video_id: String,
    pub status: DownloadStatus,
    pub progress: f32,
}
