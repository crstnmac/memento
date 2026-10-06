//! Daily and weekly summaries. The numbers are always computed locally from
//! the database; an optional short narrative is layered on when an LLM is
//! configured. The model call never holds the DB lock, has a hard timeout, and
//! its result is cached so an unchanged day is not regenerated.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use tauri::State;

use crate::db::{self, Db};
use crate::llm;
use crate::localtime::{self as lt, Offset};
use crate::state::MonitorState;

const NARRATIVE_TIMEOUT: Duration = Duration::from_secs(45);
/// An in-progress period keeps showing its last narrative this long even
/// though new captures have changed the inputs.
const ONGOING_REUSE_MS: i64 = 30 * 60_000;
const NARRATIVE_MAX_CHARS: usize = 700;

static NARRATING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppCount {
    pub app: String,
    pub count: i64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TopicActivity {
    pub title: String,
    pub app: Option<String>,
    pub count: i64,
    /// Latest memory in the period for this thread — what "open" should show.
    pub memory_id: i64,
    pub last_at: i64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MeetingItem {
    pub id: i64,
    pub title: String,
    pub kind: String,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
    pub memory_id: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FollowUpRef {
    pub id: i64,
    pub content: String,
    pub status: String,
    pub memory_id: Option<i64>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FollowUpStats {
    pub created: i64,
    pub resolved: i64,
    /// Created in the period and still open now.
    pub still_open: i64,
    pub items: Vec<FollowUpRef>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Busiest {
    /// Local hour of day, 0-23.
    pub hour: u32,
    pub count: i64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DayCount {
    pub date: String,
    pub count: i64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PeriodSummary {
    /// "day" or "week".
    pub period: String,
    /// Inclusive local dates, YYYY-MM-DD.
    pub start_date: String,
    pub end_date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// True while the period has not finished yet ("so far").
    pub ongoing: bool,
    pub memory_count: i64,
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
    pub apps: Vec<AppCount>,
    pub topics: Vec<TopicActivity>,
    pub meetings: Vec<MeetingItem>,
    pub follow_ups: FollowUpStats,
    pub busiest_hour: Option<Busiest>,
    /// Per-day counts; weeks only.
    pub days: Vec<DayCount>,
    pub narrative: Option<String>,
}

fn sql<T>(r: rusqlite::Result<T>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

/// Deterministic summary of `days` local days starting at `start_day`.
pub fn compute(
    db: &Db,
    period: &str,
    start_day: i64,
    days: i64,
    now: i64,
    off: Offset,
) -> Result<PeriodSummary, String> {
    let start = lt::start_of_day(start_day, off);
    let end = lt::start_of_day(start_day + days, off);
    let conn = &db.conn;

    let stamps: Vec<i64> = {
        let mut stmt = sql(conn.prepare_cached(
            "SELECT created_at FROM memories WHERE created_at >= ?1 AND created_at < ?2 ORDER BY created_at",
        ))?;
        let rows = sql(stmt.query_map(params![start, end], |r| r.get::<_, i64>(0)))?;
        sql(rows.collect::<Result<Vec<_>, _>>())?
    };

    let mut by_hour: HashMap<u32, i64> = HashMap::new();
    let mut by_day: HashMap<i64, i64> = HashMap::new();
    for &t in &stamps {
        *by_hour.entry(lt::local_hour(t, off)).or_default() += 1;
        *by_day.entry(lt::day_number(t, off)).or_default() += 1;
    }
    // Earliest hour wins a tie so the answer is stable.
    let busiest_hour = by_hour
        .iter()
        .max_by_key(|(hour, count)| (**count, std::cmp::Reverse(**hour)))
        .map(|(hour, count)| Busiest { hour: *hour, count: *count });
    let day_counts = if days > 1 {
        (0..days)
            .map(|i| DayCount {
                date: lt::format_ymd(start_day + i),
                count: by_day.get(&(start_day + i)).copied().unwrap_or(0),
            })
            .collect()
    } else {
        Vec::new()
    };

    let apps = {
        let mut stmt = sql(conn.prepare_cached(
            "SELECT COALESCE(app, source), COUNT(*) FROM memories
             WHERE created_at >= ?1 AND created_at < ?2
             GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 8",
        ))?;
        let rows = sql(stmt.query_map(params![start, end], |r| {
            Ok(AppCount { app: r.get(0)?, count: r.get(1)? })
        }))?;
        sql(rows.collect::<Result<Vec<_>, _>>())?
    };

    let topics = {
        let mut stmt = sql(conn.prepare_cached(
            "SELECT COALESCE(m.thread_id, m.id) AS tid, COUNT(*) AS c,
                    MAX(m.created_at), MAX(m.id)
             FROM memories m
             WHERE m.created_at >= ?1 AND m.created_at < ?2
             GROUP BY tid ORDER BY c DESC, MAX(m.created_at) DESC LIMIT 5",
        ))?;
        let rows = sql(stmt.query_map(params![start, end], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        }))?;
        let rows = sql(rows.collect::<Result<Vec<_>, _>>())?;
        let mut out = Vec::new();
        for (tid, count, last_at, memory_id) in rows {
            let root: Option<(String, Option<String>)> = sql(conn
                .query_row(
                    "SELECT title, app FROM memories WHERE id = ?1",
                    params![tid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional())?;
            let Some((title, app)) = root else { continue };
            out.push(TopicActivity { title, app, count, memory_id, last_at });
        }
        out
    };

    let meetings = {
        let mut stmt = sql(conn.prepare_cached(
            "SELECT id, title, kind, started_at, duration_ms FROM conversations
             WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at LIMIT 12",
        ))?;
        let rows = sql(stmt.query_map(params![start, end], |r| {
            Ok(MeetingItem {
                id: r.get(0)?,
                title: r.get(1)?,
                kind: r.get(2)?,
                started_at: r.get(3)?,
                duration_ms: r.get(4)?,
                memory_id: None,
            })
        }))?;
        let mut items = sql(rows.collect::<Result<Vec<_>, _>>())?;
        for item in &mut items {
            item.memory_id = sql(conn
                .query_row(
                    "SELECT id FROM memories WHERE is_meeting = 1 AND title = ?1
                     ORDER BY ABS(created_at - ?2) LIMIT 1",
                    params![item.title, item.started_at],
                    |r| r.get(0),
                )
                .optional())?;
        }
        items
    };

    let count_where = |clause: &str| -> Result<i64, String> {
        sql(conn.query_row(
            &format!("SELECT COUNT(*) FROM action_items WHERE {clause}"),
            params![start, end],
            |r| r.get(0),
        ))
    };
    let created = count_where("created_at >= ?1 AND created_at < ?2")?;
    let resolved =
        count_where("status = 'resolved' AND completed_at >= ?1 AND completed_at < ?2")?;
    let still_open = count_where("status = 'open' AND created_at >= ?1 AND created_at < ?2")?;
    let items = {
        let mut stmt = sql(conn.prepare_cached(
            "SELECT id, content, status, memory_id FROM action_items
             WHERE (created_at >= ?1 AND created_at < ?2)
                OR (status = 'resolved' AND completed_at >= ?1 AND completed_at < ?2)
             ORDER BY created_at DESC LIMIT 8",
        ))?;
        let rows = sql(stmt.query_map(params![start, end], |r| {
            Ok(FollowUpRef {
                id: r.get(0)?,
                content: r.get(1)?,
                status: r.get(2)?,
                memory_id: r.get(3)?,
            })
        }))?;
        sql(rows.collect::<Result<Vec<_>, _>>())?
    };

    Ok(PeriodSummary {
        period: period.to_string(),
        start_date: lt::format_ymd(start_day),
        end_date: lt::format_ymd(start_day + days - 1),
        start_ms: start,
        end_ms: end,
        ongoing: end > now && start <= now,
        memory_count: stamps.len() as i64,
        first_at: stamps.first().copied(),
        last_at: stamps.last().copied(),
        apps,
        topics,
        meetings,
        follow_ups: FollowUpStats { created, resolved, still_open, items },
        busiest_hour,
        days: day_counts,
        narrative: None,
    })
}

/// True when there is nothing to narrate.
fn is_empty(s: &PeriodSummary) -> bool {
    s.memory_count == 0 && s.meetings.is_empty() && s.follow_ups.created == 0
}

fn fnv1a(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Compact, factual description of the period for the model (and the cache key).
fn narrative_input(db: &Db, s: &PeriodSummary, off: Offset) -> Result<String, String> {
    let mut out = format!(
        "Period: {} ({} to {}){}\nMemories captured: {}\n",
        s.period,
        s.start_date,
        s.end_date,
        if s.ongoing { ", still in progress" } else { "" },
        s.memory_count
    );
    if !s.apps.is_empty() {
        out.push_str("Apps: ");
        out.push_str(
            &s.apps.iter().map(|a| format!("{} ({})", a.app, a.count)).collect::<Vec<_>>().join(", "),
        );
        out.push('\n');
    }
    if let Some(b) = &s.busiest_hour {
        out.push_str(&format!("Busiest hour: {:02}:00\n", b.hour));
    }
    for m in &s.meetings {
        let mins = m.duration_ms.map(|d| format!(" {} min", (d / 60_000).max(1))).unwrap_or_default();
        out.push_str(&format!("{}: {}{}\n", m.kind, m.title, mins));
    }
    out.push_str(&format!(
        "Follow-ups: {} created, {} resolved, {} still open\n",
        s.follow_ups.created, s.follow_ups.resolved, s.follow_ups.still_open
    ));
    for f in &s.follow_ups.items {
        out.push_str(&format!("- follow-up [{}]: {}\n", f.status, f.content));
    }
    let mut stmt = sql(db.conn.prepare_cached(
        "SELECT COALESCE(app, source), title, content FROM memories
         WHERE created_at >= ?1 AND created_at < ?2 ORDER BY created_at DESC LIMIT 10",
    ))?;
    let rows = sql(stmt.query_map(params![s.start_ms, s.end_ms], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
    }))?;
    let rows = sql(rows.collect::<Result<Vec<_>, _>>())?;
    let _ = off;
    if !rows.is_empty() {
        out.push_str("Recent memories:\n");
        for (app, title, content) in rows {
            let flat: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
            out.push_str(&format!("- [{app}] {title}: {}\n", flat.chars().take(220).collect::<String>()));
        }
    }
    Ok(out)
}

fn ensure_cache_table(db: &Db) -> Result<(), String> {
    sql(db.conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS summary_narratives (
            period TEXT NOT NULL,
            start_date TEXT NOT NULL,
            input_hash TEXT NOT NULL,
            narrative TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (period, input_hash)
         );
         CREATE INDEX IF NOT EXISTS idx_summary_narratives_start
            ON summary_narratives(period, start_date, created_at DESC);",
    ))
}

fn cached_narrative(
    db: &Db,
    period: &str,
    start_date: &str,
    hash: &str,
    ongoing: bool,
    now: i64,
) -> Result<Option<String>, String> {
    ensure_cache_table(db)?;
    let exact: Option<String> = sql(db
        .conn
        .query_row(
            "SELECT narrative FROM summary_narratives WHERE period = ?1 AND input_hash = ?2",
            params![period, hash],
            |r| r.get(0),
        )
        .optional())?;
    if exact.is_some() || !ongoing {
        return Ok(exact);
    }
    sql(db
        .conn
        .query_row(
            "SELECT narrative FROM summary_narratives
             WHERE period = ?1 AND start_date = ?2 AND created_at >= ?3
             ORDER BY created_at DESC LIMIT 1",
            params![period, start_date, now - ONGOING_REUSE_MS],
            |r| r.get(0),
        )
        .optional())
}

fn store_narrative(
    db: &Db,
    period: &str,
    start_date: &str,
    hash: &str,
    narrative: &str,
    now: i64,
) -> Result<(), String> {
    ensure_cache_table(db)?;
    sql(db.conn.execute(
        "INSERT OR REPLACE INTO summary_narratives (period, start_date, input_hash, narrative, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![period, start_date, hash, narrative, now],
    ))?;
    sql(db.conn.execute(
        "DELETE FROM summary_narratives WHERE created_at < ?1",
        params![now - 90 * lt::DAY_MS],
    ))?;
    Ok(())
}

fn clean_narrative(raw: &str) -> Option<String> {
    let text = raw.trim().trim_matches('"').trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(NARRATIVE_MAX_CHARS).collect())
}

struct NarratingGuard;

impl NarratingGuard {
    fn acquire() -> Option<Self> {
        NARRATING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for NarratingGuard {
    fn drop(&mut self) {
        NARRATING.store(false, Ordering::Release);
    }
}

/// Fill in `narrative`, from cache or the model. Any failure leaves it empty.
fn attach_narrative(state: &MonitorState, summary: &mut PeriodSummary, now: i64, off: Offset) {
    if is_empty(summary) {
        return;
    }
    // Gather under the lock, then release before any model call.
    let prepared = {
        let db = state.db();
        let config = match llm::load_config(&db) {
            Ok(Some(config)) => config,
            _ => return,
        };
        let Ok(input) = narrative_input(&db, summary, off) else { return };
        let hash = fnv1a(&input);
        match cached_narrative(&db, &summary.period, &summary.start_date, &hash, summary.ongoing, now) {
            Ok(Some(text)) => {
                summary.narrative = Some(text);
                return;
            }
            Ok(None) => {}
            Err(_) => return,
        }
        (config, input, hash)
    };
    let (config, input, hash) = prepared;
    // One generation at a time: a second request just shows the numbers.
    let Some(_guard) = NarratingGuard::acquire() else { return };

    let system = if summary.period == "week" {
        "You write a short recap of someone's work week for their private memory app. In 2-4 plain sentences, say what the week was about, using only the facts provided. Calm, plainspoken, no markdown, no bullet points, no advice, never invent details."
    } else {
        "You write a short recap of someone's workday for their private memory app. In 2-3 plain sentences, say what the day was about, using only the facts provided. Calm, plainspoken, no markdown, no bullet points, no advice, never invent details."
    };
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("memento-summary".into())
        .spawn(move || {
            let _ = tx.send(llm::chat_completion_with(&config, system, &input));
        });
    if spawned.is_err() {
        return;
    }
    let Ok(Ok(raw)) = rx.recv_timeout(NARRATIVE_TIMEOUT) else { return };
    let Some(text) = clean_narrative(&raw) else { return };
    {
        let db = state.db();
        let _ = store_narrative(&db, &summary.period, &summary.start_date, &hash, &text, now);
    }
    summary.narrative = Some(text);
}

fn build(
    state: &MonitorState,
    period: &str,
    start: &str,
    days: i64,
    with_narrative: bool,
) -> Result<PeriodSummary, String> {
    let start_day = lt::parse_ymd(start).ok_or_else(|| format!("'{start}' is not a date (YYYY-MM-DD)"))?;
    let now = db::now_ms();
    let off: Offset = &lt::system_offset;
    let mut summary = {
        let db = state.db();
        let mut summary = compute(&db, period, start_day, days, now, off)?;
        // Cheap cache peek so the numbers-only call can already carry a narrative.
        if !with_narrative && !is_empty(&summary) {
            if let Ok(input) = narrative_input(&db, &summary, off) {
                if let Ok(Some(text)) = cached_narrative(
                    &db,
                    period,
                    &summary.start_date,
                    &fnv1a(&input),
                    summary.ongoing,
                    now,
                ) {
                    summary.narrative = Some(text);
                }
            }
        }
        summary
    };
    if with_narrative && summary.narrative.is_none() {
        attach_narrative(state, &mut summary, now, off);
    }
    Ok(summary)
}

/// Local-data summary of one day. `with_narrative = false` returns right away
/// (with a cached narrative if there is one); the UI then asks again with
/// `true` to have one generated.
#[tauri::command]
pub async fn get_day_summary(
    state: State<'_, Arc<MonitorState>>,
    date: String,
    with_narrative: Option<bool>,
) -> Result<PeriodSummary, String> {
    let state = state.inner().clone();
    let with_narrative = with_narrative.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || build(&state, "day", &date, 1, with_narrative))
        .await
        .map_err(|e| format!("summary: {e}"))?
}

#[tauri::command]
pub async fn get_week_summary(
    state: State<'_, Arc<MonitorState>>,
    week_start: String,
    with_narrative: Option<bool>,
) -> Result<PeriodSummary, String> {
    let state = state.inner().clone();
    let with_narrative = with_narrative.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || build(&state, "week", &week_start, 7, with_narrative))
        .await
        .map_err(|e| format!("summary: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::localtime::{days_from_civil as dfc, DAY_MS, HOUR_MS};

    fn utc(_: i64) -> i64 {
        0
    }

    fn mem(db: &Db, title: &str, app: &str, thread: Option<i64>, at: i64, meeting: bool) -> i64 {
        db.conn
            .execute(
                "INSERT INTO memories (source, app, title, content, created_at, thread_id, is_meeting)
                 VALUES (?1, ?2, ?3, 'body', ?4, ?5, ?6)",
                params![if meeting { "meeting" } else { "capture" }, app, title, at, thread, meeting as i64],
            )
            .unwrap();
        db.conn.last_insert_rowid()
    }

    fn day() -> i64 {
        dfc(2026, 10, 5)
    }

    fn at(hour: i64, minute: i64) -> i64 {
        day() * DAY_MS + hour * HOUR_MS + minute * 60_000
    }

    #[test]
    fn empty_day() {
        let db = db::open_in_memory_for_test().unwrap();
        let s = compute(&db, "day", day(), 1, at(12, 0), &utc).unwrap();
        assert_eq!(s.memory_count, 0);
        assert!(s.busiest_hour.is_none() && s.first_at.is_none());
        assert!(is_empty(&s));
        assert_eq!(s.start_date, "2026-10-05");
        assert_eq!(s.end_date, "2026-10-05");
    }

    #[test]
    fn day_summary_counts_and_threads() {
        let db = db::open_in_memory_for_test().unwrap();
        let root = mem(&db, "Roadmap", "Notion", None, at(9, 5), false);
        mem(&db, "Roadmap", "Notion", Some(root), at(9, 40), false);
        let last = mem(&db, "Roadmap", "Notion", Some(root), at(14, 0), false);
        mem(&db, "Chat with Priya", "Slack", None, at(9, 50), false);
        // Yesterday and tomorrow are excluded.
        mem(&db, "Old", "Mail", None, at(23, 0) - DAY_MS, false);
        mem(&db, "Next", "Mail", None, at(0, 0) + DAY_MS, false);

        let s = compute(&db, "day", day(), 1, at(15, 0), &utc).unwrap();
        assert_eq!(s.memory_count, 4);
        assert!(s.ongoing);
        assert_eq!(s.first_at, Some(at(9, 5)));
        assert_eq!(s.last_at, Some(at(14, 0)));
        assert_eq!(s.apps[0].app, "Notion");
        assert_eq!(s.apps[0].count, 3);
        assert_eq!(s.topics[0].title, "Roadmap");
        assert_eq!(s.topics[0].count, 3);
        assert_eq!(s.topics[0].memory_id, last);
        let b = s.busiest_hour.unwrap();
        assert_eq!((b.hour, b.count), (9, 3));
        assert!(s.days.is_empty());
    }

    #[test]
    fn finished_day_is_not_ongoing() {
        let db = db::open_in_memory_for_test().unwrap();
        let s = compute(&db, "day", day() - 1, 1, at(15, 0), &utc).unwrap();
        assert!(!s.ongoing);
    }

    #[test]
    fn meetings_and_followups() {
        let db = db::open_in_memory_for_test().unwrap();
        let m = mem(&db, "Team sync", "Meet", None, at(10, 0), true);
        db.conn
            .execute(
                "INSERT INTO conversations (title, started_at, ended_at, duration_ms, kind)
                 VALUES ('Team sync', ?1, ?2, 1800000, 'meeting')",
                params![at(10, 0), at(10, 30)],
            )
            .unwrap();
        let a = db.insert_action_item_ext(m, "Send notes", false, 0, None).unwrap();
        let b = db.insert_action_item_ext(m, "Book room", false, 0, None).unwrap();
        db.conn
            .execute("UPDATE action_items SET created_at = ?1", params![at(10, 31)])
            .unwrap();
        db.conn
            .execute(
                "UPDATE action_items SET status='resolved', completed_at=?1 WHERE id=?2",
                params![at(16, 0), a],
            )
            .unwrap();
        // Created yesterday, resolved today: counts as resolved only.
        let c = db.insert_action_item_ext(m, "Old thing", false, 0, None).unwrap();
        db.conn
            .execute(
                "UPDATE action_items SET created_at=?1, status='resolved', completed_at=?2 WHERE id=?3",
                params![at(10, 0) - DAY_MS, at(11, 0), c],
            )
            .unwrap();
        let _ = b;

        let s = compute(&db, "day", day(), 1, at(17, 0), &utc).unwrap();
        assert_eq!(s.meetings.len(), 1);
        assert_eq!(s.meetings[0].duration_ms, Some(1_800_000));
        assert_eq!(s.meetings[0].memory_id, Some(m));
        assert_eq!(s.follow_ups.created, 2);
        assert_eq!(s.follow_ups.resolved, 2);
        assert_eq!(s.follow_ups.still_open, 1);
        assert_eq!(s.follow_ups.items.len(), 3);
    }

    #[test]
    fn week_has_seven_day_counts() {
        let db = db::open_in_memory_for_test().unwrap();
        mem(&db, "A", "X", None, at(9, 0), false);
        mem(&db, "B", "X", None, at(9, 0) + 2 * DAY_MS, false);
        mem(&db, "C", "X", None, at(10, 0) + 2 * DAY_MS, false);
        mem(&db, "Outside", "X", None, at(9, 0) + 7 * DAY_MS, false);
        let s = compute(&db, "week", day(), 7, at(12, 0) + 3 * DAY_MS, &utc).unwrap();
        assert_eq!(s.memory_count, 3);
        assert_eq!(s.days.len(), 7);
        assert_eq!(s.days[0].count, 1);
        assert_eq!(s.days[2].count, 2);
        assert_eq!(s.end_date, "2026-10-11");
        // Tie between hour 9 (2) and hour 10 (1): 9 wins on count.
        assert_eq!(s.busiest_hour.unwrap().hour, 9);
    }

    #[test]
    fn narrative_cache_roundtrip_and_ongoing_reuse() {
        let db = db::open_in_memory_for_test().unwrap();
        let now = at(15, 0);
        assert_eq!(cached_narrative(&db, "day", "2026-10-05", "h1", true, now).unwrap(), None);
        store_narrative(&db, "day", "2026-10-05", "h1", "A calm day.", now).unwrap();
        assert_eq!(
            cached_narrative(&db, "day", "2026-10-05", "h1", false, now).unwrap().as_deref(),
            Some("A calm day.")
        );
        // Inputs changed: a finished day regenerates, an ongoing one reuses a fresh narrative.
        assert_eq!(cached_narrative(&db, "day", "2026-10-05", "h2", false, now).unwrap(), None);
        assert_eq!(
            cached_narrative(&db, "day", "2026-10-05", "h2", true, now + 10 * 60_000).unwrap().as_deref(),
            Some("A calm day.")
        );
        // ...but not a stale one.
        assert_eq!(
            cached_narrative(&db, "day", "2026-10-05", "h2", true, now + 2 * HOUR_MS).unwrap(),
            None
        );
    }

    #[test]
    fn narrative_input_is_stable_and_hashes_differ_with_data() {
        let db = db::open_in_memory_for_test().unwrap();
        mem(&db, "Roadmap", "Notion", None, at(9, 0), false);
        let s = compute(&db, "day", day(), 1, at(12, 0), &utc).unwrap();
        let a = narrative_input(&db, &s, &utc).unwrap();
        assert_eq!(a, narrative_input(&db, &s, &utc).unwrap());
        mem(&db, "More", "Notion", None, at(10, 0), false);
        let s2 = compute(&db, "day", day(), 1, at(12, 0), &utc).unwrap();
        assert_ne!(fnv1a(&a), fnv1a(&narrative_input(&db, &s2, &utc).unwrap()));
    }

    #[test]
    fn clean_narrative_trims_and_caps() {
        assert_eq!(clean_narrative("  \"Hello.\" \n").as_deref(), Some("Hello."));
        assert_eq!(clean_narrative("   "), None);
        assert_eq!(clean_narrative(&"a".repeat(2000)).unwrap().chars().count(), NARRATIVE_MAX_CHARS);
    }
}
