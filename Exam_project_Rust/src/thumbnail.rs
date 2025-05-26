use eframe::egui::ColorImage;

/// Module for downloading and decoding video thumbnails from YouTube.
pub fn fetch_thumbnail(video_id: &str) -> Option<ColorImage> {
    // Construct URL for the YouTube thumbnail (standard high-quality default)
    let url = format!("https://img.youtube.com/vi/{}/hqdefault.jpg", video_id);
    // Perform a blocking HTTP GET request, returning None on any error
    let resp = reqwest::blocking::get(&url).ok()?.bytes().ok()?;
    // Load image data into an image::DynamicImage and convert to RGBA8
    let img = image::load_from_memory(&resp).ok()?.to_rgba8();
    // Determine the image dimensions for egui
    let size = [img.width() as usize, img.height() as usize];
    // Create a ColorImage from the raw RGBA bytes without premultiplying alpha
    Some(ColorImage::from_rgba_unmultiplied(size, &img))
}
