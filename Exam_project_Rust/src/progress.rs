pub fn parse_progress_from_line(line: &str) -> Option<f32> {
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
