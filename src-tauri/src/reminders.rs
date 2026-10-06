//! The reminder queue ("closing loops" nudge).
//!
//! Mirrors Minimi's ReminderQueue: action items with a scheduled `remind_at`
//! that have gone due are batched into a single nudge (missed-reminder
//! catch-up included) instead of one notification per item. A "calculate DND
//! state" hook reads the OS Do Not Disturb / Focus state to decide whether to
//! surface the nudge at all.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::db::{self, ActionItemRow, Db};
use crate::state::MonitorState;

/// How close (in ms) two reminders can be and still land in the same cluster,
/// so a pile of "same-time" deadlines becomes one "nudge".
const CLUSTER_WINDOW_MS: i64 = 15 * 60 * 1000;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReminderBatch {
    pub id: String,
    pub label: String,
    pub items: Vec<ActionItemRow>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DueReminders {
    pub dnd: bool,
    pub batches: Vec<ReminderBatch>,
    pub total: usize,
}

pub fn due_reminders(db: &Db) -> Result<DueReminders, String> {
    let now = db::now_ms();
    let items = db.due_reminder_items(now)?;
    let batches = cluster_items(items, now);
    let total: usize = batches.iter().map(|b| b.items.len()).sum();
    Ok(DueReminders {
        dnd: dnd_state(),
        batches,
        total,
    })
}

/// Bucket due items into "same-time" clusters: all reminders landing within
/// `CLUSTER_WINDOW_MS` of one another share one nudge.
fn cluster_items(items: Vec<ActionItemRow>, now: i64) -> Vec<ReminderBatch> {
    if items.is_empty() {
        return Vec::new();
    }
    let mut clusters: Vec<ReminderBatch> = Vec::new();
    let mut cur: Vec<ActionItemRow> = Vec::new();
    let mut anchor: i64 = items[0].remind_at.unwrap_or(now);
    for item in items {
        let t = item.remind_at.unwrap_or(now);
        if cur.is_empty() || (t - anchor).abs() <= CLUSTER_WINDOW_MS {
            if cur.is_empty() {
                anchor = t;
            }
            cur.push(item);
        } else {
            clusters.push(build_batch(cur, anchor, now));
            cur = vec![item];
            anchor = t;
        }
    }
    if !cur.is_empty() {
        clusters.push(build_batch(cur, anchor, now));
    }
    clusters
}

fn build_batch(items: Vec<ActionItemRow>, anchor: i64, now: i64) -> ReminderBatch {
    let minutes_late = (now - anchor) / 60_000;
    let (id, label) = if minutes_late > 15 {
        ("overdue".to_string(), "Overdue".to_string())
    } else if minutes_late > 0 {
        (
            "due-now".to_string(),
            format!("Due now · {}m ago", minutes_late),
        )
    } else {
        let bucket = db::hour_bucket_ms(anchor);
        (format!("due-{bucket}"), "Upcoming".to_string())
    };
    ReminderBatch { id, label, items }
}

/// The "calculate DND state" hook: best-effort read of the OS Do Not Disturb
/// / Focus state. macOS keeps this behind no public API, so we probe a couple
/// of stable surface points and default to being permissive (nudge anyway).
pub fn dnd_state() -> bool {
    if probe_dnd_plist() {
        return true;
    }
    if let Some(assertions) = dnd_assertions_path() {
        if let Ok(content) = std::fs::read_to_string(assertions) {
            // Sonoma+ stores enabled Focus modes in Assertions.json.
            if content.contains("\"enabled\" : true") || content.contains("\"enabled\": true") {
                return true;
            }
        }
    }
    false
}

fn dnd_assertions_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| {
        h.join("Library")
            .join("DoNotDisturb")
            .join("DB")
            .join("Assertions.json")
    })
}

/// Older macOS shipped a plain `doNotDisturb` key in the notification center
/// preferences.
fn probe_dnd_plist() -> bool {
    let Ok(mut child) = std::process::Command::new("defaults")
        .args(["read", "com.apple.notificationcenterui", "doNotDisturb"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let mut out = String::new();
    if child
        .stdout
        .as_mut()
        .map(|s| s.read_to_string(&mut out))
        .is_none()
    {
        return false;
    }
    let _ = child.wait();
    let v = out.trim().to_lowercase();
    v == "1" || v == "true" || v == "yes"
}

/// Background nudge worker: every 60s, batch all due reminders into ONE
/// notification (missed-reminder catch-up), unless Do Not Disturb is on —
/// in which case the items stay queued and are caught up later.
pub fn spawn_nudge_worker(app: AppHandle, state: Arc<MonitorState>) {
    std::thread::spawn(move || {
        let mut last = db::now_ms() - 60_000;
        while state.running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(15_000));
            if state.paused.load(Ordering::Relaxed) {
                continue;
            }
            let now = db::now_ms();
            if now - last < 15_000 {
                continue;
            }
            last = now;

            let reminders = {
                let db = state.db();
                match due_reminders(&db) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("reminders: {e}");
                        continue;
                    }
                }
            };
            if reminders.batches.is_empty() {
                continue;
            }

            // Always let the UI know (the banner renders whether or not a
            // system notification fires).
            let _ = app.emit("reminders-due", &reminders);

            if reminders.dnd {
                log::info!("reminders: nudges deferred (Do Not Disturb on)");
                continue;
            }

            let ids: Vec<i64> = reminders
                .batches
                .iter()
                .flat_map(|b| b.items.iter())
                .map(|i| i.id)
                .collect();
            let count = ids.len();
            let mut body = format!("{count} item(s) summarized into one nudge:\n");
            for batch in &reminders.batches {
                let mut lines: Vec<&str> = batch.items.iter().map(|i| i.content.as_str()).collect();
                if lines.len() > 3 {
                    lines.truncate(3);
                    lines.push("…");
                }
                body.push_str(&format!("· {} — {}\n", batch.label, lines.join(" | ")));
            }

            use tauri_plugin_notification::NotificationExt;
            let _ = app
                .notification()
                .builder()
                .title("A few open loops need you")
                .body(body.trim_end())
                .show();

            {
                let db = state.db();
                let _ = db.mark_reminders_shown(&ids);
            }
        }
    });
}
