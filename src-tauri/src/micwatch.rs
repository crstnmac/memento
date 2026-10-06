//! "micwatch" — automatic meeting recording supervision.
//!
//! Watches the frontmost app and the active recording to:
//!   1. auto-stop once the meeting window ends (gone back to a non-meeting app),
//!   2. clamp recording to `maxCaptureDurationSec` so an abandoned recording
//!      can't run forever, and
//!   3. optionally auto-start a recording when a meeting app comes forward.
//!
//! New recordings are only started with the mic permission already granted;
//! if the mic is unavailable, auto-start is skipped (it will retry next tick).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::audio;
use crate::db;
use crate::meeting_detection;
use crate::state::MonitorState;
use crate::transcribe;
use crate::RecordingState;

const TICK_MS: u64 = 5_000;
const AUTO_STOP_GRACE_MS: i64 = 60_000; // wait 1m after leaving a meeting
const START_CONFIRMATION_CHECKS: u8 = 2;

pub fn spawn(app: AppHandle, state: Arc<MonitorState>, recordings: Arc<RecordingState>) {
    std::thread::spawn(move || {
        let mut positive_checks: u8 = 0;
        let mut last_active_at: Option<i64> = None;
        let mut saw_confirmed_meeting = false;
        while state.running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(TICK_MS));
            let now = db::now_ms();
            let signal = meeting_detection::meeting_active();
            positive_checks = if signal {
                positive_checks.saturating_add(1)
            } else {
                0
            };
            let in_meeting = positive_checks >= START_CONFIRMATION_CHECKS;
            if in_meeting {
                last_active_at = Some(now);
                saw_confirmed_meeting = true;
            }

            let settings = load_micwatch_settings(&state);
            let recording_started = recordings
                .active
                .lock()
                .as_ref()
                .map(|r| (r.conversation_id, r.started_ms));

            match recording_started {
                None => {
                    // Require two consecutive multi-signal detections before
                    // starting; pausing memory capture also suppresses starts.
                    if settings.auto_start && !state.paused.load(Ordering::Relaxed) && in_meeting {
                        // Respect the configured sources; auto-start falls back
                        // to whichever track actually opens (see audio::start).
                        let mode = match (settings.record_mic, settings.record_system) {
                            (true, true) => audio::RecordingMode::Both,
                            (false, true) => audio::RecordingMode::System,
                            (true, false) => audio::RecordingMode::Mic,
                            (false, false) => {
                                // Recording is fully disabled; don't retry every tick.
                                continue;
                            }
                        };
                        // Claim the start slot without holding the recording
                        // lock while devices open (may block on a permission
                        // prompt); a manual start is excluded by the claim.
                        let start = match recordings.begin_start() {
                            Err(_) => None,
                            Ok(_claim) => match audio::start(&app, &state, mode, "meeting", true) {
                                Ok(active) => {
                                    let start = crate::RecordingStart {
                                        conversation_id: active.conversation_id,
                                        started_ms: active.started_ms,
                                    };
                                    *recordings.active.lock() = Some(active);
                                    Some(start)
                                }
                                Err(e) => {
                                    log::debug!("micwatch: auto-start skipped ({e})");
                                    None
                                }
                            },
                        };
                        if let Some(start) = start {
                            let _ = app.emit("recording-started", &start);
                            log::info!(
                                "micwatch: auto-started recording #{}",
                                start.conversation_id
                            );
                        }
                    } else if !signal {
                        saw_confirmed_meeting = false;
                        last_active_at = None;
                    }
                }
                Some((conversation_id, started_ms)) => {
                    let elapsed = (now - started_ms).max(0);
                    let overdue =
                        settings.max_duration_ms > 0 && elapsed >= settings.max_duration_ms;
                    let left_meeting = settings.auto_stop
                        && saw_confirmed_meeting
                        && !signal
                        && last_active_at
                            .map(|last| now - last > AUTO_STOP_GRACE_MS)
                            .unwrap_or(false);

                    if overdue || left_meeting {
                        // Detach the active recording and stop it.
                        let active = match recordings.active.lock().take() {
                            Some(a) => a,
                            None => continue,
                        };
                        let reason = if overdue {
                            format!(
                                "max duration reached ({}s)",
                                settings.max_duration_ms / 1000
                            )
                        } else {
                            "meeting ended".to_string()
                        };
                        log::info!("micwatch: auto-stopping #{conversation_id} — {reason}");
                        let result = audio::stop(&state, active);
                        match result {
                            Ok(result) => {
                                let conversation_id = result.conversation_id;
                                let payload =
                                    crate::RecordingAutoStopped { result, reason };
                                let _ = app.emit("recording-auto-stopped", &payload);
                                // Transcribe in the background; this works even
                                // when the main window is closed.
                                transcribe::spawn_for(
                                    app.clone(),
                                    state.clone(),
                                    conversation_id,
                                );
                            }
                            Err(e) => log::error!("micwatch: stop failed: {e}"),
                        }
                        positive_checks = 0;
                        last_active_at = None;
                        saw_confirmed_meeting = false;
                    }
                }
            }
        }
    });
}

struct MicwatchSettings {
    auto_start: bool,
    auto_stop: bool,
    max_duration_ms: i64,
    record_mic: bool,
    record_system: bool,
}

fn load_micwatch_settings(state: &MonitorState) -> MicwatchSettings {
    let db = state.db();
    let get = |key: &str, default: &str| -> String {
        db.get_setting(key)
            .ok()
            .flatten()
            .unwrap_or_else(|| default.to_string())
    };
    MicwatchSettings {
        auto_start: get("autoStartMeetings", "false") == "true",
        auto_stop: get("autoStopMeetings", "true") == "true",
        max_duration_ms: get("maxCaptureDurationSec", "0")
            .parse::<i64>()
            .unwrap_or(0)
            * 1000,
        // Defaults must mirror load_settings() in lib.rs.
        record_mic: get("recordMic", "true") == "true",
        record_system: get("recordSystem", "true") == "true",
    }
}
