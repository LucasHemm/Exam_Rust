use eframe::egui::ColorImage;

/// Module for downloading and decoding video thumbnails from YouTube.
pub fn fetch_thumbnail(video_id: &str) -> Option<ColorImage> {
    let url = format!("https://img.youtube.com/vi/{}/hqdefault.jpg", video_id);
    let resp = reqwest::blocking::get(&url).ok()?.bytes().ok()?;
    let img = image::load_from_memory(&resp).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, &img))
}
