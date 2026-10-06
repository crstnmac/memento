//! App icons for the capture/follow-up settings lists.
//!
//! Icons are rendered through `NSWorkspace`, so every app works — including
//! ones whose icon lives in an asset catalog rather than an `.icns` — and are
//! cached on disk as small PNGs so the lists open instantly after the first
//! load. Only bundle ids discovered by the installed-app scan are accepted;
//! the webview never supplies a filesystem path.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::db;

const ICON_PX: usize = 64;
const MAX_REQUEST: usize = 400;

static APP_PATHS: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

fn paths() -> std::sync::MutexGuard<'static, HashMap<String, PathBuf>> {
    APP_PATHS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Called by the installed-app scan for every app it finds.
pub fn remember(bundle_id: &str, path: PathBuf) {
    paths().insert(bundle_id.to_lowercase(), path);
}

pub fn known_apps() -> usize {
    paths().len()
}

fn cache_file(bundle_id: &str) -> Result<PathBuf, String> {
    let dir = db::app_data_dir()?.join("icon-cache");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name: String = bundle_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    Ok(dir.join(format!("{name}.png")))
}

/// `data:image/png;base64,…` for each requested bundle id that has an icon.
/// Unknown ids and apps that fail to render are simply omitted.
pub fn data_urls(bundle_ids: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for id in bundle_ids.iter().take(MAX_REQUEST) {
        let key = id.to_lowercase();
        let Some(app_path) = paths().get(&key).cloned() else {
            continue;
        };
        let Ok(cache) = cache_file(&key) else {
            continue;
        };
        let app_modified = std::fs::metadata(&app_path)
            .and_then(|meta| meta.modified())
            .ok();
        let fresh = std::fs::metadata(&cache)
            .and_then(|meta| meta.modified())
            .ok()
            .zip(app_modified)
            .is_some_and(|(cached, app)| cached >= app);
        let png = if fresh {
            std::fs::read(&cache).ok()
        } else {
            render_png(&app_path).inspect(|bytes| {
                let _ = std::fs::write(&cache, bytes);
            })
        };
        if let Some(png) = png.filter(|bytes| !bytes.is_empty()) {
            out.insert(id.clone(), format!("data:image/png;base64,{}", base64(&png)));
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn render_png(app_path: &std::path::Path) -> Option<Vec<u8>> {
    use objc2::AnyThread;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext,
        NSWorkspace,
    };
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

    let workspace = NSWorkspace::sharedWorkspace();
    let image = workspace.iconForFile(&NSString::from_str(&app_path.to_string_lossy()));
    let side = ICON_PX as isize;
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            side,
            side,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    image.drawInRect(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(ICON_PX as f64, ICON_PX as f64),
    ));
    NSGraphicsContext::restoreGraphicsState_class();
    let data = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(data.to_vec())
}

#[cfg(not(target_os = "macos"))]
fn render_png(_app_path: &std::path::Path) -> Option<Vec<u8>> {
    None
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn renders_a_real_app_icon_as_png() {
        let app = std::path::Path::new("/System/Applications/Mail.app");
        if !app.exists() {
            return;
        }
        let png = render_png(app).expect("icon renders");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
        assert!(png.len() > 500, "suspiciously small icon: {} bytes", png.len());
        remember("com.apple.mail.test", app.to_path_buf());
        let urls = data_urls(&["com.apple.mail.test".to_string(), "no.such.app".to_string()]);
        assert!(urls["com.apple.mail.test"].starts_with("data:image/png;base64,"));
        assert!(!urls.contains_key("no.such.app"));
        if let Ok(path) = cache_file("com.apple.mail.test") {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
