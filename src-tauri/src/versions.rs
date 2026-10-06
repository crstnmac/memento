//! Eras of state per thread.
//!
//! Mirrors Minimi's `memory_threads -> memory_versions` design: captures are
//! grouped into threads, and each thread keeps an append-only history of
//! hourly snapshots (`hour_bucket`), frozen once the hour rolls over, so we
//! can reconstruct "what was on my screen at 3pm".
//!
//! A background task ticks every minute: it freezes elapsed hour buckets and
//! updates one consolidated version for the current hour when content changes.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tauri::Emitter;

use crate::db::{self, Db};
use crate::state::MonitorState;

const TICK_MS: u64 = 60_000;
const MIN_PACE_MS: i64 = 15_000;
const MAX_ITEMS_PER_VERSION: usize = 400;

/// Stable 64-bit FNV-1a hash (same algorithm as `monitor::content_hash`),
/// used to detect whether a thread's content actually changed.
fn content_hash(content: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in content.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn spawn(app: tauri::AppHandle, state: Arc<MonitorState>) {
    std::thread::spawn(move || {
        let mut last = db::now_ms() - MIN_PACE_MS;
        while state.running.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(TICK_MS.min(15_000)));
            let now = db::now_ms();
            if now - last < MIN_PACE_MS {
                continue;
            }
            last = now;
            let db = state.db();
            match tick(&db, now) {
                Ok(changed_threads) => {
                    if changed_threads > 0 {
                        // Surface that new eras were written (UI can refresh).
                        let _ = app.emit("versions-updated", changed_threads);
                    }
                }
                Err(e) => eprintln!("versions: {e}"),
            }
        }
    });
}

fn tick(db: &Db, now: i64) -> Result<usize, String> {
    let frozen = db.freeze_elapsed_hours(now)?;
    if frozen > 0 {
        log::info!("versions: froze {frozen} hourly snapshot(s)");
    }

    let mut changed = 0usize;
    for root_id in db.capture_root_ids()? {
        let Some((_, members)) = db.get_thread(root_id)? else {
            continue;
        };
        let mut items = Vec::with_capacity(members.len().min(MAX_ITEMS_PER_VERSION));
        let mut words: i64 = 0;
        let mut joined = String::new();
        for m in members.iter().take(MAX_ITEMS_PER_VERSION) {
            words += m.content.split_whitespace().count() as i64;
            joined.push_str(&m.content);
            joined.push('\n');
            items.push(json!({
                "id": m.id,
                "title": m.title,
                "content": m.content,
                "createdAt": m.created_at,
            }));
        }
        let hash = content_hash(&joined);
        match db.latest_version_hash(root_id)? {
            Some(last) if last == hash as i64 => continue,
            _ => {}
        }
        let content = serde_json::to_string(&items).map_err(|e| e.to_string())?;
        db.insert_thread_version(root_id, &content, hash, words)?;
        changed += 1;
    }

    Ok(changed)
}
