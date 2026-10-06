//! Editing and snoozing follow-ups. The database layer only knows how to
//! change status and reminder fields; text edits and the snooze presets live
//! here, with the clock and timezone injectable for tests.

use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::db::{self, Db};
use crate::localtime::{self as lt, Offset, HOUR_MS};
use crate::state::MonitorState;

pub const MAX_TEXT_CHARS: usize = 500;
const MAX_SNOOZE_MS: i64 = 2 * 366 * lt::DAY_MS;

/// Trim, require something to say, and keep it to a sensible length.
pub fn validate_text(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("A follow-up needs some text.".into());
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!("Keep follow-ups under {MAX_TEXT_CHARS} characters."));
    }
    Ok(text.to_string())
}

/// Replace a follow-up's wording. Refuses to create a twin of another
/// follow-up from the same memory, matching the check extraction uses.
pub fn edit_text(db: &Db, id: i64, text: &str) -> Result<String, String> {
    let text = validate_text(text)?;
    let row: Option<(Option<i64>, String)> = db
        .conn
        .query_row(
            "SELECT memory_id, content FROM action_items WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((memory_id, current)) = row else {
        return Err("That follow-up no longer exists.".into());
    };
    if current == text {
        return Ok(text);
    }
    let twin: bool = db
        .conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM action_items
                WHERE id != ?1 AND memory_id IS ?2 AND status = 'open'
                  AND lower(trim(content)) = lower(trim(?3)))",
            params![id, memory_id, text],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if twin {
        return Err("Another follow-up already says this.".into());
    }
    db.conn
        .execute(
            "UPDATE action_items SET content = ?1 WHERE id = ?2",
            params![text, id],
        )
        .map_err(|e| e.to_string())?;
    Ok(text)
}

/// Remind again at `until_ms`; the follow-up stays open and its reminder
/// becomes eligible to show again.
pub fn snooze(db: &Db, id: i64, until_ms: i64, now: i64) -> Result<(), String> {
    if until_ms <= now {
        return Err("Pick a time in the future.".into());
    }
    if until_ms > now + MAX_SNOOZE_MS {
        return Err("That is too far away. Pick a date within two years.".into());
    }
    let status: Option<String> = db
        .conn
        .query_row("SELECT status FROM action_items WHERE id = ?1", params![id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    match status.as_deref() {
        None => return Err("That follow-up no longer exists.".into()),
        Some("open") => {}
        Some(_) => return Err("Only open follow-ups can be snoozed.".into()),
    }
    // Also clears reminder_shown and refreshes the date/slot labels.
    db.update_action_item(id, None, Some(Some(until_ms)), None, None)
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SnoozePreset {
    pub id: &'static str,
    pub label: &'static str,
    pub until_ms: i64,
}

/// "Later today 5pm / Tomorrow 9am / Next Monday 9am" in local time. Later
/// today is left out once it is within half an hour of (or past) 5pm.
pub fn presets(now: i64, off: Offset) -> Vec<SnoozePreset> {
    let today = lt::day_number(now, off);
    let mut out = Vec::new();
    let evening = lt::at_hour(today, 17, off);
    if evening - now >= HOUR_MS / 2 {
        out.push(SnoozePreset { id: "later-today", label: "Later today, 5 pm", until_ms: evening });
    }
    out.push(SnoozePreset {
        id: "tomorrow",
        label: "Tomorrow, 9 am",
        until_ms: lt::at_hour(today + 1, 9, off),
    });
    // The next Monday strictly after today, even when today is Monday.
    let days_ahead = 7 - lt::weekday_mon0(today);
    out.push(SnoozePreset {
        id: "next-monday",
        label: "Next Monday, 9 am",
        until_ms: lt::at_hour(today + days_ahead, 9, off),
    });
    out
}

#[tauri::command]
pub fn edit_action_item_text(
    state: State<'_, Arc<MonitorState>>,
    id: i64,
    text: String,
) -> Result<String, String> {
    edit_text(&state.db(), id, &text)
}

#[tauri::command]
pub fn snooze_action_item(
    state: State<'_, Arc<MonitorState>>,
    id: i64,
    until_ms: i64,
) -> Result<(), String> {
    snooze(&state.db(), id, until_ms, db::now_ms())
}

#[tauri::command]
pub fn get_snooze_presets() -> Vec<SnoozePreset> {
    presets(db::now_ms(), &lt::system_offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::localtime::{days_from_civil as dfc, testutil::dst_zone, DAY_MS};

    fn utc(_: i64) -> i64 {
        0
    }

    fn memory(db: &Db) -> i64 {
        db.conn
            .execute(
                "INSERT INTO memories (source, title, content, created_at) VALUES ('note', 't', 'c', 1)",
                [],
            )
            .unwrap();
        db.conn.last_insert_rowid()
    }

    #[test]
    fn validation() {
        assert_eq!(validate_text("  Call Alex \n").unwrap(), "Call Alex");
        assert!(validate_text("   ").is_err());
        assert!(validate_text("").is_err());
        assert!(validate_text(&"x".repeat(MAX_TEXT_CHARS)).is_ok());
        assert!(validate_text(&"x".repeat(MAX_TEXT_CHARS + 1)).is_err());
        // Counted in characters, not bytes.
        assert!(validate_text(&"é".repeat(MAX_TEXT_CHARS)).is_ok());
    }

    #[test]
    fn edit_updates_and_rejects_twins() {
        let db = db::open_in_memory_for_test().unwrap();
        let m = memory(&db);
        let a = db.insert_action_item_ext(m, "Email Sam", false, 0, None).unwrap();
        let b = db.insert_action_item_ext(m, "Call Sam", false, 0, None).unwrap();
        assert_eq!(edit_text(&db, a, "  Email Sam the deck ").unwrap(), "Email Sam the deck");
        assert_eq!(db.list_action_items("open").unwrap().iter().find(|i| i.id == a).unwrap().content, "Email Sam the deck");
        // Same text as sibling, ignoring case and spacing.
        assert!(edit_text(&db, b, " email sam the deck").is_err());
        // Saving unchanged text is fine.
        assert!(edit_text(&db, a, "Email Sam the deck").is_ok());
        assert!(edit_text(&db, a, "  ").is_err());
        assert!(edit_text(&db, 9999, "x").is_err());
    }

    #[test]
    fn snooze_sets_reminder_and_keeps_open() {
        let db = db::open_in_memory_for_test().unwrap();
        let m = memory(&db);
        let id = db.insert_action_item_ext(m, "Ping", false, 0, Some(5_000)).unwrap();
        db.mark_reminders_shown(&[id]).unwrap();
        let now = 1_000_000;
        snooze(&db, id, now + HOUR_MS, now).unwrap();
        let item = &db.list_action_items("open").unwrap()[0];
        assert_eq!(item.remind_at, Some(now + HOUR_MS));
        assert!(!item.reminder_shown);
        assert_eq!(item.status, "open");
    }

    #[test]
    fn snooze_rejections() {
        let db = db::open_in_memory_for_test().unwrap();
        let m = memory(&db);
        let id = db.insert_action_item_ext(m, "Ping", false, 0, None).unwrap();
        let now = 1_000_000;
        assert!(snooze(&db, id, now, now).is_err());
        assert!(snooze(&db, id, now - 1, now).is_err());
        assert!(snooze(&db, id, now + 10 * 366 * DAY_MS, now).is_err());
        assert!(snooze(&db, 4242, now + HOUR_MS, now).is_err());
        db.set_action_item_status(id, "resolved").unwrap();
        assert!(snooze(&db, id, now + HOUR_MS, now).is_err());
    }

    fn ids(p: &[SnoozePreset]) -> Vec<&'static str> {
        p.iter().map(|p| p.id).collect()
    }

    #[test]
    fn presets_midweek_morning() {
        // Wednesday 2026-10-07 08:00 UTC.
        let wed = dfc(2026, 10, 7);
        let p = presets(wed * DAY_MS + 8 * HOUR_MS, &utc);
        assert_eq!(ids(&p), ["later-today", "tomorrow", "next-monday"]);
        assert_eq!(p[0].until_ms, wed * DAY_MS + 17 * HOUR_MS);
        assert_eq!(p[1].until_ms, (wed + 1) * DAY_MS + 9 * HOUR_MS);
        assert_eq!(p[2].until_ms, dfc(2026, 10, 12) * DAY_MS + 9 * HOUR_MS);
    }

    #[test]
    fn presets_evening_drops_later_today() {
        let wed = dfc(2026, 10, 7);
        assert_eq!(ids(&presets(wed * DAY_MS + 16 * HOUR_MS + 40 * 60_000, &utc)), ["tomorrow", "next-monday"]);
        assert_eq!(ids(&presets(wed * DAY_MS + 20 * HOUR_MS, &utc)), ["tomorrow", "next-monday"]);
        // Exactly half an hour before is still offered.
        assert_eq!(presets(wed * DAY_MS + 16 * HOUR_MS + 30 * 60_000, &utc).len(), 3);
    }

    #[test]
    fn next_monday_from_monday_and_sunday() {
        let mon = dfc(2026, 10, 5);
        let p = presets(mon * DAY_MS + 8 * HOUR_MS, &utc);
        assert_eq!(p.last().unwrap().until_ms, (mon + 7) * DAY_MS + 9 * HOUR_MS);
        let sun = dfc(2026, 10, 11);
        let p = presets(sun * DAY_MS + 8 * HOUR_MS, &utc);
        assert_eq!(p.last().unwrap().until_ms, (sun + 1) * DAY_MS + 9 * HOUR_MS);
        assert_eq!(p[1].until_ms, p[2].until_ms);
    }

    #[test]
    fn presets_use_local_time_across_dst() {
        // New York, spring forward Sunday 2026-03-08 at 07:00 UTC.
        let zone = dst_zone(dfc(2026, 3, 8) * DAY_MS + 7 * HOUR_MS);
        // Saturday 2026-03-07 21:00 local (02:00 UTC Sunday): "tomorrow 9am" is
        // after the change, so 9:00 EDT = 13:00 UTC.
        let now = dfc(2026, 3, 8) * DAY_MS + 2 * HOUR_MS;
        let p = presets(now, &zone);
        assert_eq!(ids(&p), ["tomorrow", "next-monday"]);
        assert_eq!(p[0].until_ms, dfc(2026, 3, 8) * DAY_MS + 13 * HOUR_MS);
        // Next Monday 2026-03-09 9am EDT = 13:00 UTC.
        assert_eq!(p[1].until_ms, dfc(2026, 3, 9) * DAY_MS + 13 * HOUR_MS);
    }
}
