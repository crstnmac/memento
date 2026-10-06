//! Hourly check-in — one quiet notification summarizing the past hour,
//! Minimi-style ("this hour" digest), honoring Do Not Disturb and capture
//! pause. Skips hours where nothing happened.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::params;
use tauri::AppHandle;

use crate::db;
use crate::state::MonitorState;

pub fn spawn(app: AppHandle, state: Arc<MonitorState>) {
    std::thread::spawn(move || {
        // First check-in after an hour of running, not at launch.
        loop {
            std::thread::sleep(Duration::from_secs(3_600));
            if !state.running.load(Ordering::Relaxed) {
                break;
            }
            if state.is_paused() {
                continue;
            }
            let summary = match summarize(&state) {
                Ok(summary) => summary,
                Err(error) => {
                    log::warn!("check-in: {error}");
                    continue;
                }
            };
            let Some(summary) = summary else {
                continue;
            };
            if crate::reminders::dnd_state() {
                continue;
            }
            let _ = crate::send_notification(
                app.clone(),
                "Memento — the last hour".to_string(),
                summary,
            );
        }
    });
}

/// Returns None for an uneventful hour.
fn summarize(state: &Arc<MonitorState>) -> Result<Option<String>, String> {
    let db = state.db();
    let since = db::now_ms() - 3_600_000;
    let count = |sql: &str| -> Result<i64, String> {
        db.conn
            .query_row(sql, params![since], |row| row.get(0))
            .map_err(|e| e.to_string())
    };
    let memories =
        count("SELECT COUNT(*) FROM memories WHERE created_at >= ?1")?;
    let new_actions =
        count("SELECT COUNT(*) FROM action_items WHERE created_at >= ?1")?;
    if memories == 0 && new_actions == 0 {
        return Ok(None);
    }
    let open: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM action_items WHERE status = 'open'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(Some(format!(
        "{memories} new memories · {new_actions} follow-ups captured · {open} still open"
    )))
}
