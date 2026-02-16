//! Screen capture and change detection.

/// Capture the primary monitor as PNG bytes.
pub fn capture_screen() -> Result<Vec<u8>, String> {
    let monitors =
        xcap::Monitor::all().map_err(|e| format!("Failed to enumerate monitors: {}", e))?;

    let monitor = monitors
        .into_iter()
        .find(|m| m.is_primary())
        .or_else(|| xcap::Monitor::all().ok()?.into_iter().next())
        .ok_or_else(|| "No monitors found".to_string())?;

    let img = monitor
        .capture_image()
        .map_err(|e| format!("Screen capture failed: {}", e))?;

    // Convert RGBA → RGB (JPEG doesn't support alpha channel)
    let rgb_img = image::DynamicImage::ImageRgba8(img).to_rgb8();

    // Encode as JPEG (smaller than PNG, faster to encode)
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(rgb_img)
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .map_err(|e| format!("Image encoding failed: {}", e))?;

    Ok(buf.into_inner())
}

/// Compare two images for significant changes.
/// Downscales both images to a small grid and computes RMS pixel difference.
/// Returns true if difference exceeds threshold (0.0–1.0).
pub fn has_significant_change(prev: &[u8], curr: &[u8], threshold: f64) -> bool {
    let prev_img = match image::load_from_memory(prev) {
        Ok(img) => img,
        Err(_) => return true, // Can't decode = treat as changed
    };
    let curr_img = match image::load_from_memory(curr) {
        Ok(img) => img,
        Err(_) => return true,
    };

    // Downscale to 32x32 for fast comparison
    let prev_thumb = prev_img.resize_exact(32, 32, image::imageops::FilterType::Nearest);
    let curr_thumb = curr_img.resize_exact(32, 32, image::imageops::FilterType::Nearest);

    let prev_bytes = prev_thumb.to_rgba8();
    let curr_bytes = curr_thumb.to_rgba8();

    // RMS difference across all pixels
    let mut total_diff: f64 = 0.0;
    let pixel_count = (32 * 32) as f64;

    for (p, c) in prev_bytes.pixels().zip(curr_bytes.pixels()) {
        let dr = (p[0] as f64 - c[0] as f64) / 255.0;
        let dg = (p[1] as f64 - c[1] as f64) / 255.0;
        let db = (p[2] as f64 - c[2] as f64) / 255.0;
        total_diff += (dr * dr + dg * dg + db * db) / 3.0;
    }

    let rms = (total_diff / pixel_count).sqrt();

    println!(
        "[Vision] Change detection: RMS={:.4}, threshold={:.4}",
        rms, threshold
    );
    rms > threshold
}
