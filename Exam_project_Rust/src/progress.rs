/// Parses a progress line emitted by yt-dlp with the format `downloaded_bytes:<percent>%`
///
/// Returns `Some(f32)` representing the progress as a fraction (0.0 to 1.0),
/// or `None` if the line does not match the expected pattern.
pub fn parse_progress_from_line(line: &str) -> Option<f32> {
    // Check if the line starts with the expected prefix
    if let Some(rest) = line.strip_prefix("downloaded_bytes:") {
        // Remove leading/trailing whitespace from the remaining text
        let trimmed = rest.trim();
        // Ensure the text ends with a '%' and strip it off
        if let Some(number) = trimmed.strip_suffix('%') {
            // Attempt to parse the numeric part into an f32
            if let Ok(v) = number.trim().parse::<f32>() {
                // Convert percentage value to a fraction and return
                return Some(v / 100.0);
            }
        }
    }
    // Return None if parsing fails at any step
    None
}
