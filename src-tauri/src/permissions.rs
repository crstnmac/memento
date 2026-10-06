//! App-owned macOS permission state and request flow.
//!
//! We intentionally avoid the permission plugin's microphone request because
//! version 2.3.0 passes a null AVFoundation completion handler.

use serde::Serialize;
use tauri::AppHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionState {
    Granted,
    Denied,
    NotDetermined,
    Restricted,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    pub accessibility: PermissionState,
    pub microphone: PermissionState,
    pub screen_recording: PermissionState,
}

#[cfg(target_os = "macos")]
fn microphone_state() -> PermissionState {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
    let Some(media_type) = (unsafe { AVMediaTypeAudio }) else {
        return PermissionState::Restricted;
    };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) } {
        AVAuthorizationStatus::Authorized => PermissionState::Granted,
        AVAuthorizationStatus::Denied => PermissionState::Denied,
        AVAuthorizationStatus::Restricted => PermissionState::Restricted,
        _ => PermissionState::NotDetermined,
    }
}

#[cfg(not(target_os = "macos"))]
fn microphone_state() -> PermissionState {
    PermissionState::Granted
}

pub async fn get() -> Permissions {
    Permissions {
        accessibility: if tauri_plugin_macos_permissions::check_accessibility_permission().await {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        },
        microphone: microphone_state(),
        screen_recording: if tauri_plugin_macos_permissions::check_screen_recording_permission()
            .await
        {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        },
    }
}

#[cfg(target_os = "macos")]
fn request_microphone() -> Result<(), String> {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};
    let media_type =
        (unsafe { AVMediaTypeAudio }).ok_or("AVFoundation audio media type unavailable")?;
    let completion = RcBlock::new(|_granted: Bool| {});
    unsafe {
        AVCaptureDevice::requestAccessForMediaType_completionHandler(media_type, &completion)
    };
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn request_microphone() -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_privacy_pane(pane: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{pane}"
        ))
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub async fn request(_app: AppHandle, kind: &str) -> Result<(), String> {
    match kind {
        "accessibility" => {
            tauri_plugin_macos_permissions::request_accessibility_permission().await;
            Ok(())
        }
        "microphone" => match microphone_state() {
            PermissionState::NotDetermined => request_microphone(),
            PermissionState::Granted => Ok(()),
            _ => {
                #[cfg(target_os = "macos")]
                return open_privacy_pane("Privacy_Microphone");
                #[cfg(not(target_os = "macos"))]
                return Ok(());
            }
        },
        "screen-recording" => {
            tauri_plugin_macos_permissions::request_screen_recording_permission().await;
            #[cfg(target_os = "macos")]
            open_privacy_pane("Privacy_ScreenCapture")?;
            Ok(())
        }
        _ => Err(format!("unknown permission kind: {kind}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_states_serialize_without_collapsing() {
        assert_eq!(
            serde_json::to_string(&PermissionState::Granted).unwrap(),
            "\"granted\""
        );
        assert_eq!(
            serde_json::to_string(&PermissionState::Denied).unwrap(),
            "\"denied\""
        );
        assert_eq!(
            serde_json::to_string(&PermissionState::NotDetermined).unwrap(),
            "\"notDetermined\""
        );
        assert_eq!(
            serde_json::to_string(&PermissionState::Restricted).unwrap(),
            "\"restricted\""
        );
    }
}
