//! Persistent, fully local closing-loops agent.
//!
//! Every memory is fingerprinted exactly once. Historical startup and hourly
//! runs extract missing commitments, suppress semantic duplicates, and
//! reconcile explicit completion language against currently open actions.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::Emitter;

use crate::actions::{self, Speaker};
use crate::db::{self, ActionItemRow, Db, MemoryRow};
use crate::localtime as lt;
use crate::state::MonitorState;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub kind: String,
    pub processed: usize,
    pub created: usize,
    pub resolved: usize,
    pub fingerprint: String,
}

pub fn spawn(app: tauri::AppHandle, state: Arc<MonitorState>) {
    std::thread::spawn(move || {
        {
            let db = state.db();
            if let Ok(summary) = run_cycle(&db, "historical", 5_000) {
                let _ = app.emit("agent-updated", summary);
            }
        }
        let mut last_hour = db::hour_bucket_ms(db::now_ms());
        while state.running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_secs(60));
            let hour = db::hour_bucket_ms(db::now_ms());
            if hour == last_hour {
                continue;
            }
            last_hour = hour;
            let db = state.db();
            match run_cycle(&db, "hourly", 5_000) {
                Ok(summary) => {
                    let _ = app.emit("agent-updated", summary);
                }
                Err(error) => eprintln!("agent hourly run: {error}"),
            }
        }
    });
}

pub fn run_cycle(db: &Db, kind: &str, limit: i64) -> Result<RunSummary, String> {
    let memories = db.list_unprocessed_agent_memories(limit)?;
    let mut existing = all_action_items(db)?;
    let mut created = 0usize;
    let mut resolved = 0usize;
    let mut run_hash: u64 = 0xcbf2_9ce4_8422_2325;

    for memory in &memories {
        let fingerprint = stable_hash(&memory.content);
        run_hash ^= fingerprint;
        run_hash = run_hash.wrapping_mul(0x0000_0100_0000_01b3);

        // Backfill: memories saved before extraction ran (or while it was off).
        created += extract_for_memory(db, memory)?;

        for completion in completion_lines(&memory.content) {
            for item in existing.iter_mut().filter(|item| item.status == "open") {
                if completion_matches(&completion, &item.content) {
                    db.set_action_item_status(item.id, "resolved")?;
                    item.status = "resolved".into();
                    resolved += 1;
                }
            }
        }
        db.mark_agent_memory_processed(memory.id, fingerprint as i64, kind)?;
    }

    let summary = RunSummary {
        kind: kind.to_string(),
        processed: memories.len(),
        created,
        resolved,
        fingerprint: format!("{run_hash:016x}"),
    };
    let result = serde_json::to_string(&summary).map_err(|error| error.to_string())?;
    db.insert_agent_run(kind, None, Some(&result))?;
    Ok(summary)
}

fn all_action_items(db: &Db) -> Result<Vec<ActionItemRow>, String> {
    let mut items = Vec::new();
    for status in ["open", "resolved", "dismissed"] {
        items.extend(db.list_action_items(status)?);
    }
    Ok(items)
}

/// Find follow-ups in a memory with the local rules and store the new ones.
/// Called as soon as a memory is saved, and again by the hourly pass as a
/// backfill; duplicates (including ones the user already resolved or
/// dismissed) are filtered when inserting.
pub fn extract_for_memory(db: &Db, memory: &MemoryRow) -> Result<usize, String> {
    // A captured screen is mostly other people's words; notes, voice notes and
    // meetings are the user's own.
    let speaker = if memory.source == "capture" {
        Speaker::Mixed
    } else {
        Speaker::Own
    };
    let mut created = 0;
    for action in actions::detect(&memory.content, speaker, db::now_ms(), &lt::system_offset) {
        if db
            .insert_action_item_if_missing(memory.id, &action.content, action.urgent, action.remind_at)?
            .is_some()
        {
            created += 1;
        }
    }
    Ok(created)
}

fn completion_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| {
            let lower = line.to_lowercase();
            !lower.contains("not done")
                && [
                    "done",
                    "completed",
                    "finished",
                    "resolved",
                    "sent",
                    "shipped",
                ]
                .iter()
                .any(|marker| lower.contains(marker))
        })
        .map(str::to_string)
        .collect()
}

fn completion_matches(completion: &str, action: &str) -> bool {
    let completion = actions::normalized_tokens(completion);
    let action = actions::normalized_tokens(action);
    if action.is_empty() {
        return false;
    }
    let overlap = completion.intersection(&action).count();
    overlap >= 2 && overlap as f32 / action.len() as f32 >= 0.4
}

fn stable_hash(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(db: &Db, source: &str, content: &str) -> MemoryRow {
        db.insert_memory(source, Some("App"), "t", content, source == "meeting", false, None)
            .unwrap()
    }

    #[test]
    fn a_meeting_becomes_follow_ups_once() -> Result<(), String> {
        let db = db::open_in_memory_for_test()?;
        let meeting = memory(
            &db,
            "meeting",
            "Karen, can you follow up with the infrastructure team by Thursday? \
             I will email them this afternoon. I'll draft the runbook by Wednesday.",
        );
        assert_eq!(extract_for_memory(&db, &meeting)?, 3);
        let open = db.list_action_items("open")?;
        assert_eq!(open.len(), 3);
        assert!(open.iter().any(|a| a.content.starts_with("Follow up with the infrastructure team")));
        // Saving the same text again (a re-capture, the hourly backfill) adds nothing.
        assert_eq!(extract_for_memory(&db, &meeting)?, 0);
        let again = memory(&db, "meeting", "I'll draft the runbook by Wednesday morning.");
        assert_eq!(extract_for_memory(&db, &again)?, 0);
        assert_eq!(db.list_action_items("open")?.len(), 3);
        Ok(())
    }

    #[test]
    fn resolved_and_dismissed_follow_ups_do_not_come_back() -> Result<(), String> {
        let db = db::open_in_memory_for_test()?;
        let first = memory(&db, "note", "I need to renew the parking permit.");
        assert_eq!(extract_for_memory(&db, &first)?, 1);
        let id = db.list_action_items("open")?[0].id;
        db.set_action_item_status(id, "dismissed")?;
        // The same sentence on the screen again must not resurrect it.
        let screen = memory(&db, "capture", "Reminder to renew the parking permit");
        assert_eq!(extract_for_memory(&db, &screen)?, 0);
        assert!(db.list_action_items("open")?.is_empty());
        Ok(())
    }

    #[test]
    fn a_captured_screen_only_yields_requests_not_other_peoples_promises() -> Result<(), String> {
        let db = db::open_in_memory_for_test()?;
        let screen = memory(
            &db,
            "capture",
            "Priya: I'll update the checklist tomorrow.\nPriya: can you send the revised mockups by Friday?",
        );
        assert_eq!(extract_for_memory(&db, &screen)?, 1);
        assert_eq!(db.list_action_items("open")?[0].content, "Send the revised mockups by Friday");
        Ok(())
    }

    #[test]
    fn the_model_rephrasing_a_rule_found_follow_up_is_not_a_second_one() -> Result<(), String> {
        let db = db::open_in_memory_for_test()?;
        let note = memory(&db, "note", "I will send the launch report by Friday.");
        assert_eq!(extract_for_memory(&db, &note)?, 1);
        // What the LLM path would add for the same memory, worded differently.
        assert!(db
            .insert_action_item_if_missing(note.id, "Send the launch report to the board", false, None)?
            .is_none());
        // A genuinely different follow-up still goes through.
        assert!(db
            .insert_action_item_if_missing(note.id, "Book the offsite venue", false, None)?
            .is_some());
        Ok(())
    }

    #[test]
    fn a_switched_off_source_gets_no_rule_found_follow_ups() -> Result<(), String> {
        let db = db::open_in_memory_for_test()?;
        let screen = memory(&db, "capture", "Can you send the revised mockups by Friday?");
        db.set_memory_service(screen.id, "slack")?;
        db.set_follow_up_source_enabled("slack", false)?;
        assert_eq!(extract_for_memory(&db, &screen)?, 0);
        db.set_follow_up_source_enabled("slack", true)?;
        assert_eq!(extract_for_memory(&db, &screen)?, 1);
        Ok(())
    }

    #[test]
    fn the_hourly_pass_backfills_memories_saved_before_extraction() -> Result<(), String> {
        let db = db::open_in_memory_for_test()?;
        memory(&db, "note", "Remember to call the dentist on Friday.");
        let summary = run_cycle(&db, "historical", 100)?;
        assert_eq!(summary.processed, 1);
        assert_eq!(summary.created, 1);
        assert_eq!(db.list_action_items("open")?.len(), 1);
        assert_eq!(run_cycle(&db, "hourly", 100)?.created, 0);
        Ok(())
    }

    #[test]
    fn duplicate_actions_and_completions_are_reconciled() {
        assert!(completion_matches(
            "Done — sent the launch report",
            "Send the launch report"
        ));
        assert!(!completion_matches(
            "Done with lunch",
            "Send the launch report"
        ));
        assert!(completion_lines("not done with report").is_empty());
    }

    #[test]
    fn cycles_are_fingerprinted_and_historical_actions_reconciled() -> Result<(), String> {
        let db = db::open_in_memory_for_test()?;
        db.insert_memory(
            "note",
            None,
            "commitment",
            "I will send the launch report by Friday",
            false,
            false,
            None,
        )?;
        db.insert_action_item_if_missing(
            db.conn.last_insert_rowid(),
            "Send the launch report",
            false,
            None,
        )?;
        let first = run_cycle(&db, "historical", 100)?;
        assert_eq!(first.processed, 1);
        assert_eq!(first.created, 0);
        let repeated = run_cycle(&db, "historical", 100)?;
        assert_eq!(repeated.processed, 0);
        assert_eq!(repeated.created, 0);

        db.insert_memory(
            "note",
            None,
            "completion",
            "Done — sent the launch report",
            false,
            false,
            None,
        )?;
        let completion = run_cycle(&db, "hourly", 100)?;
        assert_eq!(completion.resolved, 1);
        assert!(db.list_action_items("open")?.is_empty());
        assert_eq!(db.list_action_items("resolved")?.len(), 1);
        Ok(())
    }
}
