//! Screen capture and change detection.

use crate::vision::config::NormalizedRect;

#[derive(Debug, Clone, Default)]
pub struct CaptureOptions {
    pub display_id: Option<String>,
    pub region: Option<NormalizedRect>,
}

#[derive(Debug, Clone)]
pub struct CapturedImage {
    pub jpeg_bytes: Vec<u8>,
    pub display_id: Option<String>,
    pub region: Option<NormalizedRect>,
    pub image_hash: String,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScreenInfo {
    pub display_id: String,
    pub label: String,
    pub is_primary: bool,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

/// Capture the primary monitor as JPEG bytes.
pub fn capture_screen() -> Result<Vec<u8>, String> {
    capture_screen_with_options(&CaptureOptions::default()).map(|image| image.jpeg_bytes)
}

pub fn capture_screen_with_options(options: &CaptureOptions) -> Result<CapturedImage, String> {
    let screens =
        screenshots::Screen::all().map_err(|e| format!("Failed to enumerate screens: {}", e))?;

    let requested_display_id = options
        .display_id
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let descriptors = screen_descriptors(&screens);
    let selection = select_screen_index(&descriptors, requested_display_id)?;
    let warning = selection.warning;
    let screen = screens
        .get(selection.index)
        .ok_or_else(|| "Selected screen index was out of bounds".to_string())?;

    let img = screen
        .capture()
        .map_err(|e| format!("Screen capture failed: {}", e))?;

    let rgba_img = image::RgbaImage::from_raw(img.width(), img.height(), img.as_raw().to_vec())
        .ok_or_else(|| "Failed to construct image from raw bytes".to_string())?;
    let mut dynamic = image::DynamicImage::ImageRgba8(rgba_img);
    let region = options.region.and_then(NormalizedRect::clamped);
    if let Some(region) = region {
        let x = ((region.x * dynamic.width() as f64).round() as u32)
            .min(dynamic.width().saturating_sub(1));
        let y = ((region.y * dynamic.height() as f64).round() as u32)
            .min(dynamic.height().saturating_sub(1));
        let max_width = dynamic.width().saturating_sub(x).max(1);
        let max_height = dynamic.height().saturating_sub(y).max(1);
        let width = ((region.width * dynamic.width() as f64).round() as u32).clamp(1, max_width);
        let height =
            ((region.height * dynamic.height() as f64).round() as u32).clamp(1, max_height);
        dynamic = dynamic.crop_imm(x, y, width, height);
    }

    let image_hash = image_fingerprint(&dynamic);
    let rgb_img = dynamic.to_rgb8();
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(rgb_img)
        .write_to(&mut buf, image::ImageFormat::Jpeg)
        .map_err(|e| format!("Image encoding failed: {}", e))?;

    Ok(CapturedImage {
        jpeg_bytes: buf.into_inner(),
        display_id: Some(display_id_for_screen(screen)),
        region,
        image_hash,
        warning,
    })
}

pub fn list_screens() -> Result<Vec<ScreenInfo>, String> {
    let screens =
        screenshots::Screen::all().map_err(|e| format!("Failed to enumerate screens: {}", e))?;

    Ok(screens
        .iter()
        .enumerate()
        .map(|(index, screen)| {
            let info = &screen.display_info;
            ScreenInfo {
                display_id: display_id_for_screen(screen),
                label: format!(
                    "Display {} · {}×{}{}",
                    index + 1,
                    info.width,
                    info.height,
                    if info.is_primary { " · Primary" } else { "" }
                ),
                is_primary: info.is_primary,
                x: info.x,
                y: info.y,
                width: info.width,
                height: info.height,
                scale_factor: info.scale_factor,
            }
        })
        .collect())
}

fn display_id_for_screen(screen: &screenshots::Screen) -> String {
    let info = &screen.display_info;
    format!("{}:{}:{}x{}", info.id, info.x, info.width, info.height)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScreenDescriptor {
    display_id: String,
    is_primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScreenSelection {
    index: usize,
    warning: Option<String>,
}

fn screen_descriptors(screens: &[screenshots::Screen]) -> Vec<ScreenDescriptor> {
    screens
        .iter()
        .map(|screen| ScreenDescriptor {
            display_id: display_id_for_screen(screen),
            is_primary: screen.display_info.is_primary,
        })
        .collect()
}

fn select_screen_index(
    screens: &[ScreenDescriptor],
    requested_display_id: Option<&str>,
) -> Result<ScreenSelection, String> {
    if screens.is_empty() {
        return Err("No screens found".to_string());
    }

    if let Some(display_id) = requested_display_id {
        if let Some(index) = screens
            .iter()
            .position(|screen| screen.display_id == display_id)
        {
            return Ok(ScreenSelection {
                index,
                warning: None,
            });
        }

        let (index, fallback_label) = primary_or_first_screen_index(screens);
        return Ok(ScreenSelection {
            index,
            warning: Some(format!(
                "Display '{}' was not found; captured {} instead",
                display_id, fallback_label
            )),
        });
    }

    let (index, fallback_label) = primary_or_first_screen_index(screens);
    Ok(ScreenSelection {
        index,
        warning: if screens[index].is_primary {
            None
        } else {
            Some(format!(
                "No primary screen was reported; captured {} instead",
                fallback_label
            ))
        },
    })
}

fn primary_or_first_screen_index(screens: &[ScreenDescriptor]) -> (usize, &'static str) {
    screens
        .iter()
        .position(|screen| screen.is_primary)
        .map(|index| (index, "primary display"))
        .unwrap_or((0, "first available display"))
}

fn image_fingerprint(image: &image::DynamicImage) -> String {
    let thumb = image
        .resize_exact(16, 16, image::imageops::FilterType::Nearest)
        .to_rgba8();
    let mut hash: u64 = 0xcbf29ce484222325;
    for pixel in thumb.pixels() {
        for byte in &pixel.0[..3] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("{:016x}", hash)
}

/// Compare two images for significant changes.
/// Downscales both images to a small grid and computes RMS pixel difference.
/// Returns true if difference exceeds threshold (0.0–1.0).
pub fn has_significant_change(prev: &[u8], curr: &[u8], threshold: f64) -> bool {
    image_change_score(prev, curr)
        .map(|score| score > threshold)
        .unwrap_or(true)
}

pub fn image_change_score(prev: &[u8], curr: &[u8]) -> Option<f64> {
    let prev_img = image::load_from_memory(prev).ok()?;
    let curr_img = image::load_from_memory(curr).ok()?;

    let prev_thumb = prev_img.resize_exact(32, 32, image::imageops::FilterType::Nearest);
    let curr_thumb = curr_img.resize_exact(32, 32, image::imageops::FilterType::Nearest);

    let prev_bytes = prev_thumb.to_rgba8();
    let curr_bytes = curr_thumb.to_rgba8();

    let mut total_diff: f64 = 0.0;
    let pixel_count = (32 * 32) as f64;

    for (p, c) in prev_bytes.pixels().zip(curr_bytes.pixels()) {
        let dr = (p[0] as f64 - c[0] as f64) / 255.0;
        let dg = (p[1] as f64 - c[1] as f64) / 255.0;
        let db = (p[2] as f64 - c[2] as f64) / 255.0;
        total_diff += (dr * dr + dg * dg + db * db) / 3.0;
    }

    let rms = (total_diff / pixel_count).sqrt();
    tracing::info!(
        target: "vision",
        "[Vision] Change detection: RMS={:.4}",
        rms
    );
    Some(rms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(color: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb(color));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut buf, image::ImageFormat::Jpeg)
            .unwrap();
        buf.into_inner()
    }

    #[test]
    fn change_detection_skips_below_threshold() {
        let a = jpeg([20, 20, 20]);
        let b = jpeg([22, 20, 20]);
        assert!(!has_significant_change(&a, &b, 0.05));
    }

    #[test]
    fn change_detection_detects_large_delta() {
        let a = jpeg([0, 0, 0]);
        let b = jpeg([255, 255, 255]);
        assert!(has_significant_change(&a, &b, 0.05));
    }

    #[test]
    fn select_screen_falls_back_to_first_when_no_primary_is_reported() {
        let screens = vec![
            ScreenDescriptor {
                display_id: "display-a".to_string(),
                is_primary: false,
            },
            ScreenDescriptor {
                display_id: "display-b".to_string(),
                is_primary: false,
            },
        ];

        let selected = select_screen_index(&screens, None).unwrap();

        assert_eq!(selected.index, 0);
        assert_eq!(
            selected.warning.as_deref(),
            Some("No primary screen was reported; captured first available display instead")
        );
    }

    #[test]
    fn select_screen_falls_back_to_first_when_requested_display_is_missing_without_primary() {
        let screens = vec![ScreenDescriptor {
            display_id: "display-a".to_string(),
            is_primary: false,
        }];

        let selected = select_screen_index(&screens, Some("missing-display")).unwrap();

        assert_eq!(selected.index, 0);
        assert_eq!(
            selected.warning.as_deref(),
            Some(
                "Display 'missing-display' was not found; captured first available display instead"
            )
        );
    }
}
