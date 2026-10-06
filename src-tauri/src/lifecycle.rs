//! Data lifecycle: what Memento has captured per source, and deleting it by
//! source and/or time range, including everything derived from it (versions,
//! enrichments, follow-ups, activity conversations, vault Markdown, recordings).
//!
//! Deletion is two-phase so the shared DB lock is never held across file I/O:
//! the SQL runs in one transaction under the lock (and snapshots which files
//! belong to the deleted rows); the files are removed afterwards without it.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use rusqlite::{params_from_iter, types::Value, Connection};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::adapters::FOLLOW_UP_SOURCES;
use crate::db::{self, Db, MemoryRow};
use crate::state::MonitorState;
use crate::vault::{self, ExportRef};

/// Source id of everything the user wrote themselves (quick notes and notes
/// imported from the vault's `notes/` folder).
pub const NOTES_ID: &str = "notes";
/// Display name used for captures that have no source tag and no app name.
const UNKNOWN_APP: &str = "Unknown app";

/// The source a memory belongs to: the follow-up source tag when present,
/// meetings and voice notes by their `source`, user notes as `notes`, and
/// NULL for untagged captures (those are grouped by app name instead).
const SOURCE_SQL: &str = "CASE WHEN m.source IN ('note','wiki-note') THEN 'notes' \
    ELSE COALESCE(m.service, CASE m.source WHEN 'meeting' THEN 'meeting' \
    WHEN 'voice-note' THEN 'voice-note' END) END";

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFilter {
    pub source_ids: Option<Vec<String>>,
    /// App names of captures that have no source tag (see `CaptureSource`).
    pub apps: Option<Vec<String>>,
    /// Inclusive lower bound on creation time.
    pub since_ms: Option<i64>,
    /// Exclusive upper bound on creation time.
    pub until_ms: Option<i64>,
    /// User-written notes are only deleted when this is explicitly true.
    pub include_notes: Option<bool>,
    /// Also delete recordings (conversations and their audio files) of the
    /// selected meeting / voice-note sources and time range.
    pub include_recordings: Option<bool>,
    /// Required to delete without any source or time restriction.
    pub all: Option<bool>,
}

impl DeleteFilter {
    fn ids(&self) -> &[String] {
        self.source_ids.as_deref().unwrap_or(&[])
    }
    fn apps(&self) -> &[String] {
        self.apps.as_deref().unwrap_or(&[])
    }
    fn validate(&self) -> Result<(), String> {
        let restricted = !self.ids().is_empty()
            || !self.apps().is_empty()
            || self.since_ms.is_some()
            || self.until_ms.is_some();
        if !restricted && self.all != Some(true) {
            return Err("Choose a source or a time range first. Deleting everything needs an explicit confirmation.".into());
        }
        if let (Some(since), Some(until)) = (self.since_ms, self.until_ms) {
            if since >= until {
                return Err("The start of the time range must be before its end.".into());
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BytesFreed {
    pub database: i64,
    pub recordings: i64,
    pub vault: i64,
    pub total: i64,
}

/// Returned by both `preview_delete` and `delete_memories`; for the same data
/// the two are identical.
#[derive(Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSummary {
    pub memories: i64,
    /// Of `memories`, how many are notes the user wrote.
    pub notes: i64,
    /// Conversation threads (more than one capture) removed entirely.
    pub threads: i64,
    pub follow_ups: i64,
    pub activity_conversations: i64,
    pub recordings: i64,
    pub recording_files: i64,
    pub vault_files: i64,
    /// Files that could not be removed (always 0 in a preview).
    pub failed_files: i64,
    pub bytes_freed: BytesFreed,
}

// ---------- filtering ----------

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(",")
}

/// WHERE clause (alias `m` = memories) and its bound values, in order.
fn scope_sql(
    ids: &[String],
    apps: &[String],
    since: Option<i64>,
    until: Option<i64>,
    include_notes: bool,
) -> (String, Vec<Value>) {
    let mut conditions = vec!["1=1".to_string()];
    let mut values: Vec<Value> = Vec::new();
    if !include_notes {
        conditions.push("m.source NOT IN ('note','wiki-note')".into());
    }
    let mut either = Vec::new();
    if !ids.is_empty() {
        either.push(format!("({SOURCE_SQL}) IN ({})", placeholders(ids.len())));
        values.extend(ids.iter().cloned().map(Value::Text));
    }
    if !apps.is_empty() {
        // App names only select captures without a source tag, matching how
        // the summary groups them, so a row's count is what its delete removes.
        let mut app_match = format!("m.app IN ({})", placeholders(apps.len()));
        values.extend(apps.iter().cloned().map(Value::Text));
        if apps.iter().any(|app| app == UNKNOWN_APP) {
            app_match.push_str(" OR TRIM(COALESCE(m.app, '')) = ''");
        }
        either.push(format!("(({SOURCE_SQL}) IS NULL AND ({app_match}))"));
    }
    if !either.is_empty() {
        conditions.push(format!("({})", either.join(" OR ")));
    }
    if let Some(since) = since {
        conditions.push("m.created_at >= ?".into());
        values.push(Value::Integer(since));
    }
    if let Some(until) = until {
        conditions.push("m.created_at < ?".into());
        values.push(Value::Integer(until));
    }
    (conditions.join(" AND "), values)
}

fn sql_err(error: rusqlite::Error) -> String {
    error.to_string()
}

fn scalar(conn: &Connection, sql: &str) -> Result<i64, String> {
    conn.query_row(sql, [], |row| row.get::<_, Option<i64>>(0))
        .map(|value| value.unwrap_or(0))
        .map_err(sql_err)
}

// ---------- plan ----------

struct Plan {
    summary: DeleteSummary,
    refs: Vec<ExportRef>,
    /// (conversation id, audio files inside the recordings folder)
    recordings: Vec<(i64, Vec<PathBuf>)>,
}

fn is_inside(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
}

fn recording_kinds(filter: &DeleteFilter) -> Vec<&'static str> {
    if filter.include_recordings != Some(true) {
        return Vec::new();
    }
    let ids = filter.ids();
    if ids.is_empty() {
        // Selecting by app only never reaches recordings.
        return if filter.apps().is_empty() { vec!["meeting", "voice-note"] } else { Vec::new() };
    }
    ["meeting", "voice-note"]
        .into_iter()
        .filter(|kind| ids.iter().any(|id| id == kind))
        .collect()
}

/// Works out exactly what the filter selects, inside the caller's transaction,
/// leaving the selection in temp tables for `apply`. Nothing is modified except
/// those temp tables.
fn build_plan(
    conn: &Connection,
    filter: &DeleteFilter,
    recordings_root: Option<&Path>,
) -> Result<Plan, String> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS lifecycle_del;
         DROP TABLE IF EXISTS lifecycle_gone;
         DROP TABLE IF EXISTS lifecycle_rerooted;
         DROP TABLE IF EXISTS lifecycle_acts;
         CREATE TEMP TABLE lifecycle_del (id INTEGER PRIMARY KEY);
         CREATE TEMP TABLE lifecycle_gone (id INTEGER PRIMARY KEY);
         CREATE TEMP TABLE lifecycle_rerooted (old_id INTEGER PRIMARY KEY, new_id INTEGER NOT NULL);
         CREATE TEMP TABLE lifecycle_acts (id INTEGER PRIMARY KEY);",
    )
    .map_err(sql_err)?;

    let (condition, values) = scope_sql(
        filter.ids(),
        filter.apps(),
        filter.since_ms,
        filter.until_ms,
        filter.include_notes == Some(true),
    );
    conn.execute(
        &format!("INSERT INTO lifecycle_del SELECT m.id FROM memories m WHERE {condition}"),
        params_from_iter(values.iter()),
    )
    .map_err(sql_err)?;

    let mut summary = DeleteSummary {
        memories: scalar(conn, "SELECT COUNT(*) FROM lifecycle_del")?,
        notes: scalar(
            conn,
            "SELECT COUNT(*) FROM memories m JOIN lifecycle_del d ON d.id = m.id
             WHERE m.source IN ('note','wiki-note')",
        )?,
        follow_ups: scalar(
            conn,
            "SELECT COUNT(*) FROM action_items WHERE memory_id IN (SELECT id FROM lifecycle_del)",
        )?,
        ..Default::default()
    };

    // Threads: a deleted thread root with surviving children hands the thread
    // to its earliest surviving child (re-root) rather than dropping captures
    // outside the selection; a thread whose members are all selected is gone.
    let roots: Vec<i64> = {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT COALESCE(m.thread_id, m.id) FROM memories m
                 JOIN lifecycle_del d ON d.id = m.id",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(sql_err)?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(sql_err)?;
        rows
    };
    {
        let mut survivors = conn
            .prepare(
                "SELECT id FROM memories
                 WHERE (id = ?1 OR thread_id = ?1) AND id NOT IN (SELECT id FROM lifecycle_del)
                 ORDER BY created_at ASC, id ASC LIMIT 1",
            )
            .map_err(sql_err)?;
        let mut members = conn
            .prepare("SELECT COUNT(*) FROM memories WHERE id = ?1 OR thread_id = ?1")
            .map_err(sql_err)?;
        let mut root_deleted = conn
            .prepare("SELECT 1 FROM lifecycle_del WHERE id = ?1")
            .map_err(sql_err)?;
        let mut mark_gone = conn
            .prepare("INSERT OR IGNORE INTO lifecycle_gone (id) VALUES (?1)")
            .map_err(sql_err)?;
        let mut mark_reroot = conn
            .prepare("INSERT OR IGNORE INTO lifecycle_rerooted (old_id, new_id) VALUES (?1, ?2)")
            .map_err(sql_err)?;
        for root in roots {
            let first: Option<i64> = match survivors.query_row([root], |row| row.get(0)) {
                Ok(id) => Some(id),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(error) => return Err(error.to_string()),
            };
            match first {
                None => {
                    mark_gone.execute([root]).map_err(sql_err)?;
                    let count: i64 = members.query_row([root], |row| row.get(0)).map_err(sql_err)?;
                    if count > 1 {
                        summary.threads += 1;
                    }
                }
                Some(next) => {
                    let deleted = root_deleted.exists([root]).map_err(sql_err)?;
                    if deleted && next != root {
                        mark_reroot.execute([root, next]).map_err(sql_err)?;
                    }
                }
            }
        }
    }

    // An activity conversation goes when every capture it recorded is deleted.
    conn.execute_batch(
        "INSERT INTO lifecycle_acts
         SELECT DISTINCT e.conversation_id FROM activity_conversation_events e
         WHERE e.memory_id IN (SELECT id FROM lifecycle_del)
           AND NOT EXISTS (
             SELECT 1 FROM activity_conversation_events other
             WHERE other.conversation_id = e.conversation_id
               AND (other.memory_id IS NULL
                    OR other.memory_id NOT IN (SELECT id FROM lifecycle_del)));",
    )
    .map_err(sql_err)?;
    summary.activity_conversations = scalar(conn, "SELECT COUNT(*) FROM lifecycle_acts")?;

    summary.bytes_freed.database = scalar(
        conn,
        "SELECT
           COALESCE((SELECT SUM(octet_length(m.title) + octet_length(m.content)
                                + COALESCE(octet_length(m.derivative), 0))
                     FROM memories m JOIN lifecycle_del d ON d.id = m.id), 0)
         + COALESCE((SELECT SUM(octet_length(structured_json)) FROM memory_enrichments
                     WHERE memory_id IN (SELECT id FROM lifecycle_del)), 0)
         + COALESCE((SELECT SUM(octet_length(content)) FROM memory_versions
                     WHERE thread_id IN (SELECT id FROM lifecycle_gone)), 0)
         + COALESCE((SELECT SUM(octet_length(content)) FROM action_items
                     WHERE memory_id IN (SELECT id FROM lifecycle_del)), 0)
         + COALESCE((SELECT SUM(octet_length(current_content)) FROM activity_conversations
                     WHERE id IN (SELECT id FROM lifecycle_acts)), 0)
         + COALESCE((SELECT SUM(octet_length(new_content)) FROM activity_conversation_events
                     WHERE conversation_id IN (SELECT id FROM lifecycle_acts)), 0)
         + COALESCE((SELECT SUM(octet_length(text)) FROM activity_conversation_messages
                     WHERE conversation_id IN (SELECT id FROM lifecycle_acts)), 0)",
    )?;

    let refs = {
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.source, m.app, m.title, m.created_at, m.is_meeting
                 FROM memories m JOIN lifecycle_del d ON d.id = m.id
                 WHERE m.source != 'wiki-note'",
            )
            .map_err(sql_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ExportRef {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    app: row.get(2)?,
                    title: row.get(3)?,
                    created_at: row.get(4)?,
                    is_meeting: row.get::<_, i64>(5)? != 0,
                })
            })
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        rows
    };

    let mut recordings = Vec::new();
    let kinds = recording_kinds(filter);
    if !kinds.is_empty() {
        let mut sql = format!(
            "SELECT id, audio_path FROM conversations WHERE kind IN ({})",
            placeholders(kinds.len())
        );
        let mut values: Vec<Value> = kinds.iter().map(|kind| Value::Text((*kind).into())).collect();
        if let Some(since) = filter.since_ms {
            sql.push_str(" AND started_at >= ?");
            values.push(Value::Integer(since));
        }
        if let Some(until) = filter.until_ms {
            sql.push_str(" AND started_at < ?");
            values.push(Value::Integer(until));
        }
        let mut stmt = conn.prepare(&sql).map_err(sql_err)?;
        let rows = stmt
            .query_map(params_from_iter(values.iter()), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(sql_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        for (id, audio) in rows {
            // Only files inside the recordings folder are ever removed.
            let files = audio
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .filter(|path| recordings_root.is_some_and(|root| is_inside(root, path)))
                .collect();
            recordings.push((id, files));
        }
        summary.recordings = recordings.len() as i64;
    }

    Ok(Plan { summary, refs, recordings })
}

/// Performs the deletion for a plan built in the same transaction.
fn apply(conn: &Connection, plan: &Plan) -> Result<(), String> {
    // Hand threads over before their root disappears (versions belong to the
    // root, and would otherwise be cascaded away with it).
    conn.execute_batch(
        "UPDATE memories SET thread_id = NULL,
            derivative = COALESCE(derivative,
                (SELECT old.derivative FROM memories old
                 WHERE old.id = (SELECT old_id FROM lifecycle_rerooted WHERE new_id = memories.id)))
         WHERE id IN (SELECT new_id FROM lifecycle_rerooted);
         UPDATE memories SET thread_id = (SELECT new_id FROM lifecycle_rerooted WHERE old_id = memories.thread_id)
         WHERE thread_id IN (SELECT old_id FROM lifecycle_rerooted)
           AND id NOT IN (SELECT id FROM lifecycle_del)
           AND id NOT IN (SELECT new_id FROM lifecycle_rerooted);
         UPDATE memory_versions SET thread_id = (SELECT new_id FROM lifecycle_rerooted WHERE old_id = memory_versions.thread_id)
         WHERE thread_id IN (SELECT old_id FROM lifecycle_rerooted);

         DELETE FROM memory_versions WHERE thread_id IN (SELECT id FROM lifecycle_del);
         DELETE FROM action_items WHERE memory_id IN (SELECT id FROM lifecycle_del);
         DELETE FROM memory_enrichments WHERE memory_id IN (SELECT id FROM lifecycle_del);
         DELETE FROM agent_memory_fingerprints WHERE memory_id IN (SELECT id FROM lifecycle_del);
         DELETE FROM activity_conversation_events WHERE conversation_id IN (SELECT id FROM lifecycle_acts);
         DELETE FROM activity_conversation_messages WHERE conversation_id IN (SELECT id FROM lifecycle_acts);
         DELETE FROM activity_conversations WHERE id IN (SELECT id FROM lifecycle_acts);
         DELETE FROM memories WHERE id IN (SELECT id FROM lifecycle_del);",
    )
    .map_err(sql_err)?;
    for (id, _) in &plan.recordings {
        conn.execute("DELETE FROM conversations WHERE id = ?1", [id])
            .map_err(sql_err)?;
    }
    conn.execute_batch(
        "DROP TABLE lifecycle_del; DROP TABLE lifecycle_gone;
         DROP TABLE lifecycle_rerooted; DROP TABLE lifecycle_acts;",
    )
    .map_err(sql_err)
}

// ---------- preview / delete ----------

fn file_len(path: &Path) -> Option<i64> {
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len() as i64)
}

/// SQL part of a preview (lock held by the caller). Rolls everything back.
fn preview_sql(
    db: &Db,
    filter: &DeleteFilter,
    recordings_root: Option<&Path>,
) -> Result<(Plan, Option<PathBuf>), String> {
    filter.validate()?;
    let tx = db.conn.unchecked_transaction().map_err(sql_err)?;
    let plan = build_plan(&tx, filter, recordings_root)?;
    drop(tx);
    Ok((plan, vault::vault_dir(db).ok()))
}

/// Filesystem part of a preview (no lock).
fn preview_files(plan: Plan, vault_root: Option<PathBuf>) -> DeleteSummary {
    let mut summary = plan.summary;
    for path in plan.recordings.iter().flat_map(|(_, files)| files) {
        if let Some(size) = file_len(path) {
            summary.recording_files += 1;
            summary.bytes_freed.recordings += size;
        }
    }
    if let Some(root) = vault_root {
        for path in vault::matching_files(&root, &plan.refs) {
            if let Some(size) = file_len(&path) {
                summary.vault_files += 1;
                summary.bytes_freed.vault += size;
            }
        }
    }
    finish_bytes(&mut summary);
    summary
}

fn finish_bytes(summary: &mut DeleteSummary) {
    let bytes = &mut summary.bytes_freed;
    bytes.total = bytes.database + bytes.recordings + bytes.vault;
}

struct Committed {
    plan: Plan,
    job: Option<vault::RemovalJob>,
}

/// SQL part of a delete (lock held by the caller): one transaction.
fn delete_sql(
    db: &Db,
    filter: &DeleteFilter,
    recordings_root: Option<&Path>,
) -> Result<Committed, String> {
    filter.validate()?;
    let tx = db.conn.unchecked_transaction().map_err(sql_err)?;
    let plan = build_plan(&tx, filter, recordings_root)?;
    apply(&tx, &plan)?;
    tx.commit().map_err(sql_err)?;
    // The rows are gone for good; a failure to plan the file cleanup must not
    // be reported as a failed delete.
    let job = vault::prepare_removal(db, plan.refs.clone()).unwrap_or_else(|error| {
        log::warn!("vault cleanup after delete could not be prepared: {error}");
        None
    });
    Ok(Committed { plan, job })
}

/// Filesystem part of a delete (no lock).
fn delete_files(committed: Committed) -> DeleteSummary {
    let Committed { plan, job } = committed;
    let mut summary = plan.summary;
    for path in plan.recordings.iter().flat_map(|(_, files)| files) {
        let Some(size) = file_len(path) else { continue };
        match std::fs::remove_file(path) {
            Ok(()) => {
                summary.recording_files += 1;
                summary.bytes_freed.recordings += size;
            }
            Err(_) => summary.failed_files += 1,
        }
    }
    if let Some(job) = job {
        let report = vault::run_removal(job);
        summary.vault_files = report.files_removed;
        summary.bytes_freed.vault = report.bytes_removed;
        summary.failed_files += report.failed;
    }
    finish_bytes(&mut summary);
    summary
}

pub fn preview(state: &MonitorState, filter: &DeleteFilter) -> Result<DeleteSummary, String> {
    let root = db::recordings_dir().ok();
    let (plan, vault_root) = preview_sql(&state.db(), filter, root.as_deref())?;
    Ok(preview_files(plan, vault_root))
}

pub fn delete(state: &MonitorState, filter: &DeleteFilter) -> Result<DeleteSummary, String> {
    let root = db::recordings_dir().ok();
    let committed = delete_sql(&state.db(), filter, root.as_deref())?;
    Ok(delete_files(committed))
}

// ---------- capture summary ----------

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSource {
    /// Follow-up source id, `notes`, or `app:<name>` for untagged captures.
    pub id: String,
    /// "source" | "notes" | "app"
    pub kind: String,
    pub name: String,
    pub category: String,
    /// Set for `kind == "app"`: pass as `apps` / `app` to delete or browse.
    pub app: Option<String>,
    pub memories: i64,
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
    pub bytes: i64,
}

#[derive(Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTotals {
    pub memories: i64,
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
    pub bytes: i64,
    pub recordings: i64,
}

#[derive(Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    pub database_bytes: i64,
    pub recordings_bytes: i64,
    pub vault_bytes: i64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSummary {
    pub sources: Vec<CaptureSource>,
    pub totals: CaptureTotals,
    pub disk: DiskUsage,
}

fn summary_sql(conn: &Connection) -> Result<(Vec<CaptureSource>, CaptureTotals), String> {
    // One pass over `memories` with aggregates; only the column lengths are
    // read from the large `content` column, never the text itself.
    let sql = format!(
        "SELECT src, app, COUNT(*), MIN(created_at), MAX(created_at), SUM(size) FROM (
           SELECT ({SOURCE_SQL}) AS src,
                  CASE WHEN ({SOURCE_SQL}) IS NULL THEN COALESCE(NULLIF(TRIM(m.app), ''), '{UNKNOWN_APP}') END AS app,
                  m.created_at AS created_at,
                  octet_length(m.title) + octet_length(m.content) AS size
           FROM memories m)
         GROUP BY src, app"
    );
    let mut stmt = conn.prepare(&sql).map_err(sql_err)?;
    let mut sources: Vec<CaptureSource> = stmt
        .query_map([], |row| {
            let src: Option<String> = row.get(0)?;
            let app: Option<String> = row.get(1)?;
            let (id, kind, name, category, app) = match (src, app) {
                (Some(id), _) if id == NOTES_ID => (id, "notes", "Your notes".to_string(), "notes".to_string(), None),
                (Some(id), _) => {
                    let known = FOLLOW_UP_SOURCES.iter().find(|source| source.id == id);
                    let name = known.map_or_else(|| id.clone(), |source| source.name.to_string());
                    let category = known.map_or("other", |source| source.category).to_string();
                    (id, "source", name, category, None)
                }
                (None, app) => {
                    let app = app.unwrap_or_else(|| UNKNOWN_APP.into());
                    (format!("app:{app}"), "app", app.clone(), "other".to_string(), Some(app))
                }
            };
            Ok(CaptureSource {
                id,
                kind: kind.into(),
                name,
                category,
                app,
                memories: row.get(2)?,
                first_at: row.get(3)?,
                last_at: row.get(4)?,
                bytes: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            })
        })
        .map_err(sql_err)?
        .collect::<Result<_, _>>()
        .map_err(sql_err)?;
    sources.sort_by(|a, b| b.memories.cmp(&a.memories).then_with(|| a.name.cmp(&b.name)));
    let mut totals = CaptureTotals::default();
    for source in &sources {
        totals.memories += source.memories;
        totals.bytes += source.bytes;
        totals.first_at = match (totals.first_at, source.first_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        totals.last_at = match (totals.last_at, source.last_at) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }
    totals.recordings = scalar(conn, "SELECT COUNT(*) FROM conversations")?;
    Ok((sources, totals))
}

fn dir_size(path: &Path) -> i64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(entry.path()),
                Ok(kind) if kind.is_file() => {
                    total += entry.metadata().map(|meta| meta.len() as i64).unwrap_or(0);
                }
                _ => {}
            }
        }
    }
    total
}

pub fn capture_summary(state: &MonitorState) -> Result<CaptureSummary, String> {
    let (sources, totals, vault_root) = {
        let db = state.db();
        let (sources, totals) = summary_sql(&db.conn)?;
        (sources, totals, vault::vault_dir(&db).ok())
    };
    let mut disk = DiskUsage::default();
    if let Ok(data_dir) = db::app_data_dir() {
        for name in ["memento.db", "memento.db-wal", "memento.db-shm"] {
            disk.database_bytes += file_len(&data_dir.join(name)).unwrap_or(0);
        }
        disk.recordings_bytes = dir_size(&data_dir.join("recordings"));
    }
    if let Some(root) = vault_root {
        disk.vault_bytes = dir_size(&root);
    }
    Ok(CaptureSummary { sources, totals, disk })
}

// ---------- browsing ----------

fn list_filtered(
    conn: &Connection,
    source_id: Option<&str>,
    app: Option<&str>,
    since: Option<i64>,
    until: Option<i64>,
    limit: i64,
    offset: i64,
) -> Result<Vec<MemoryRow>, String> {
    let ids: Vec<String> = source_id.map(|id| vec![id.to_string()]).unwrap_or_default();
    let apps: Vec<String> = app.map(|name| vec![name.to_string()]).unwrap_or_default();
    let (condition, mut values) = scope_sql(&ids, &apps, since, until, true);
    values.push(Value::Integer(limit.clamp(1, 1000)));
    values.push(Value::Integer(offset.max(0)));
    let sql = format!(
        "SELECT m.id, m.source, m.app, m.title, m.content, m.created_at, m.is_meeting, m.open_loop
         FROM memories m WHERE {condition}
         ORDER BY m.created_at DESC, m.id DESC LIMIT ? OFFSET ?"
    );
    let mut stmt = conn.prepare(&sql).map_err(sql_err)?;
    let rows = stmt
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(MemoryRow {
                id: row.get(0)?,
                source: row.get(1)?,
                app: row.get(2)?,
                title: row.get(3)?,
                content: row.get(4)?,
                created_at: row.get(5)?,
                is_meeting: row.get::<_, i64>(6)? != 0,
                open_loop: row.get::<_, i64>(7)? != 0,
            })
        })
        .map_err(sql_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_err)?;
    Ok(rows)
}

// ---------- commands ----------

type Shared<'a> = State<'a, Arc<MonitorState>>;

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn get_capture_summary(state: Shared<'_>) -> Result<CaptureSummary, String> {
    let state = state.inner().clone();
    blocking(move || capture_summary(&state)).await
}

#[tauri::command]
pub async fn preview_delete(state: Shared<'_>, filter: DeleteFilter) -> Result<DeleteSummary, String> {
    let state = state.inner().clone();
    blocking(move || preview(&state, &filter)).await
}

#[tauri::command]
pub async fn delete_memories(state: Shared<'_>, filter: DeleteFilter) -> Result<DeleteSummary, String> {
    let state = state.inner().clone();
    blocking(move || delete(&state, &filter)).await
}

#[tauri::command]
pub fn list_memories_filtered(
    state: Shared<'_>,
    source_id: Option<String>,
    app: Option<String>,
    since_ms: Option<i64>,
    until_ms: Option<i64>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<MemoryRow>, String> {
    list_filtered(
        &state.db().conn,
        source_id.as_deref(),
        app.as_deref(),
        since_ms,
        until_ms,
        limit.unwrap_or(200),
        offset.unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    struct Fixture {
        db: Db,
        dir: PathBuf,
        vault: PathBuf,
        recordings: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn capture(db: &Db, app: &str, title: &str, service: Option<&str>, at: i64) -> i64 {
        let row = db.insert_memory("capture", Some(app), title, "some captured text", false, false, None).unwrap();
        if let Some(service) = service {
            db.set_memory_service(row.id, service).unwrap();
        }
        if at != 0 {
            db.conn.execute("UPDATE memories SET created_at = ?1 WHERE id = ?2", rusqlite::params![at, row.id]).unwrap();
        }
        row.id
    }

    fn count(db: &Db, sql: &str) -> i64 {
        db.conn.query_row(sql, [], |row| row.get(0)).unwrap()
    }

    /// slack: thread of three (ids a,b,c; 1000/2000/3000); gmail: two single
    /// threads; one untagged "Foo" capture; one note; one meeting + voice note.
    struct Ids {
        slack: [i64; 3],
        gmail: [i64; 2],
        foo: i64,
        note: i64,
        meeting: i64,
    }

    fn fixture() -> (Fixture, Ids) {
        let dir = std::env::temp_dir().join(format!(
            "memento-lifecycle-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let vault = dir.join("vault");
        let recordings = dir.join("recordings");
        fs::create_dir_all(vault.join("notes")).unwrap();
        fs::create_dir_all(&recordings).unwrap();
        let db = db::open_in_memory_for_test().unwrap();
        db.set_setting("vaultPath", &vault.to_string_lossy()).unwrap();

        // Inserted back to back so they form one thread; back-dated afterwards.
        let slack = [
            capture(&db, "Slack", "#general", Some("slack"), 0),
            capture(&db, "Slack", "#general", Some("slack"), 0),
            capture(&db, "Slack", "#general", Some("slack"), 0),
        ];
        for (id, at) in slack.iter().zip([1_000, 2_000, 3_000]) {
            db.conn.execute("UPDATE memories SET created_at = ?1 WHERE id = ?2", rusqlite::params![at, id]).unwrap();
        }
        let gmail = [
            capture(&db, "Google Chrome", "Inbox", Some("gmail"), 1_500),
            capture(&db, "Google Chrome", "Receipts", Some("gmail"), 2_500),
        ];
        let foo = capture(&db, "Foo", "foo window", None, 4_000);
        let note = db.insert_memory("note", Some("Memento"), "my note", "remember milk", false, true, None).unwrap().id;
        db.conn.execute("UPDATE memories SET created_at = 2200 WHERE id = ?1", [note]).unwrap();
        let meeting = db.insert_memory("meeting", Some("Meeting"), "Standup", "we shipped", true, false, None).unwrap().id;
        db.conn.execute("UPDATE memories SET created_at = 5000 WHERE id = ?1", [meeting]).unwrap();
        // Recordings: a meeting with two files and an old voice note.
        for (name, kind, started) in [("m1", "meeting", 4_900_i64), ("v1", "voice-note", 100)] {
            let files: Vec<String> = ["mic", "sys"]
                .iter()
                .map(|part| {
                    let path = recordings.join(format!("{name}-{part}.wav"));
                    fs::write(&path, vec![0u8; 100]).unwrap();
                    path.to_string_lossy().into_owned()
                })
                .collect();
            db.insert_conversation(name, started, Some(&files.join(",")), kind).unwrap();
        }
        let conn = &db.conn;
        conn.execute("INSERT INTO memory_enrichments VALUES (?1, 'p', 'm', 1, '{\"summary\":\"s\"}', 1)", [slack[0]]).unwrap();
        conn.execute("INSERT INTO memory_enrichments VALUES (?1, 'p', 'm', 1, '{\"summary\":\"s\"}', 1)", [slack[1]]).unwrap();
        conn.execute(
            "INSERT INTO memory_versions (thread_id, hour_bucket, content, content_hash, created_at) VALUES (?1, 0, 'v1 text', 1, 1)",
            [slack[0]],
        ).unwrap();
        for memory in [slack[1], gmail[0]] {
            conn.execute("INSERT INTO action_items (memory_id, content, created_at) VALUES (?1, 'do it', 1)", [memory]).unwrap();
        }
        conn.execute("INSERT INTO agent_memory_fingerprints VALUES (?1, 1, 1, 'k')", [slack[1]]).unwrap();
        // Conversation A only saw slack captures; B also saw a gmail one.
        for (title, memories) in [("A", vec![slack[1], slack[2]]), ("B", vec![slack[2], gmail[0]])] {
            conn.execute(
                "INSERT INTO activity_conversations (app, title, current_content, current_hash, started_at, updated_at) VALUES ('x', ?1, 'text', 1, 1, 1)",
                [title],
            ).unwrap();
            let conversation = conn.last_insert_rowid();
            for memory in memories {
                conn.execute(
                    "INSERT INTO activity_conversation_events (conversation_id, memory_id, captured_at, before_chars, new_chars, content_hash, new_content) VALUES (?1, ?2, 1, 0, 1, 1, 'new')",
                    [conversation, memory],
                ).unwrap();
            }
            conn.execute("INSERT INTO activity_conversation_messages VALUES (?1, 1, 'hello', 1)", [conversation]).unwrap();
        }
        let fixture = Fixture { db, dir, vault, recordings };
        // Exported files for every generated memory, and a daily index.
        let refs = all_refs(&fixture.db);
        for item in &refs {
            let path = fixture.vault.join(vault::relative_path(item));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "exported").unwrap();
        }
        fs::create_dir_all(fixture.vault.join("summaries/daily")).unwrap();
        (fixture, Ids { slack, gmail, foo, note, meeting })
    }

    fn all_refs(db: &Db) -> Vec<ExportRef> {
        let mut stmt = db
            .conn
            .prepare("SELECT id, source, app, title, created_at, is_meeting FROM memories WHERE source != 'wiki-note'")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok(ExportRef {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    app: row.get(2)?,
                    title: row.get(3)?,
                    created_at: row.get(4)?,
                    is_meeting: row.get::<_, i64>(5)? != 0,
                })
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows
    }

    fn exported_files(fixture: &Fixture) -> usize {
        vault::matching_files(&fixture.vault, &all_refs(&fixture.db)).len()
    }

    fn slack_filter() -> DeleteFilter {
        DeleteFilter { source_ids: Some(vec!["slack".into()]), ..Default::default() }
    }

    fn run_preview(f: &Fixture, filter: &DeleteFilter) -> DeleteSummary {
        let (plan, root) = preview_sql(&f.db, filter, Some(&f.recordings)).unwrap();
        preview_files(plan, root)
    }

    fn run_delete(f: &Fixture, filter: &DeleteFilter) -> DeleteSummary {
        let committed = delete_sql(&f.db, filter, Some(&f.recordings)).unwrap();
        delete_files(committed)
    }

    #[test]
    fn empty_filter_is_refused_unless_all_is_explicit() {
        let (f, _) = fixture();
        let before = count(&f.db, "SELECT COUNT(*) FROM memories");
        assert!(delete_sql(&f.db, &DeleteFilter::default(), None).is_err());
        let notes_only = DeleteFilter { include_notes: Some(true), include_recordings: Some(true), ..Default::default() };
        assert!(preview_sql(&f.db, &notes_only, None).is_err());
        assert!(preview_sql(&f.db, &DeleteFilter { source_ids: Some(vec![]), apps: Some(vec![]), ..Default::default() }, None).is_err());
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM memories"), before);

        let everything = run_delete(&f, &DeleteFilter { all: Some(true), ..Default::default() });
        // 8 memories, of which the note is protected.
        assert_eq!(everything.memories, 7);
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM memories"), 1);
    }

    #[test]
    fn preview_matches_what_delete_removes() {
        let (f, ids) = fixture();
        let filter = slack_filter();
        let preview = run_preview(&f, &filter);
        // Nothing changed by previewing.
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM memories"), 8);
        // No scratch tables are left behind.
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM sqlite_temp_master WHERE name LIKE 'lifecycle_%'"), 0);
        assert_eq!(preview.memories, 3);
        assert_eq!(preview.threads, 1);
        assert_eq!(preview.follow_ups, 1);
        assert_eq!(preview.activity_conversations, 1);
        assert_eq!(preview.vault_files, 3);
        assert_eq!(preview.recordings, 0);
        assert!(preview.bytes_freed.database > 0 && preview.bytes_freed.vault == 24);

        let result = run_delete(&f, &filter);
        assert_eq!(serde_json::to_value(&preview).unwrap(), serde_json::to_value(&result).unwrap());
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM memories"), 5);
        for id in ids.slack {
            assert_eq!(count(&f.db, &format!("SELECT COUNT(*) FROM memories WHERE id = {id}")), 0);
        }
        // FTS stays consistent.
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'general'"), 0);
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'Receipts'"), 1);
        // Previewing again now finds nothing.
        assert_eq!(run_preview(&f, &filter).memories, 0);
    }

    #[test]
    fn deleting_cascades_to_derived_rows_and_vault_files() {
        let (f, ids) = fixture();
        let before_files = exported_files(&f);
        let result = run_delete(&f, &slack_filter());
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM memory_enrichments"), 0);
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM memory_versions"), 0);
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM agent_memory_fingerprints"), 0);
        // Only the gmail follow-up survives.
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM action_items"), 1);
        assert_eq!(count(&f.db, "SELECT memory_id FROM action_items"), ids.gmail[0]);
        // Conversation A (all slack) is gone, B (also saw gmail) is kept.
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM activity_conversations"), 1);
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM activity_conversation_messages"), 1);
        assert_eq!(exported_files(&f), before_files - 3);
        assert_eq!(result.vault_files, 3);
        assert_eq!(result.failed_files, 0);
    }

    #[test]
    fn deleted_thread_root_hands_the_thread_to_its_earliest_survivor() {
        let (f, ids) = fixture();
        // Only the root (created at 1000) is in range.
        let filter = DeleteFilter { source_ids: Some(vec!["slack".into()]), until_ms: Some(1_500), ..Default::default() };
        let preview = run_preview(&f, &filter);
        assert_eq!((preview.memories, preview.threads), (1, 0));
        run_delete(&f, &filter);
        let (b, c) = (ids.slack[1], ids.slack[2]);
        let root: Option<i64> = f.db.conn.query_row("SELECT thread_id FROM memories WHERE id = ?1", [b], |r| r.get(0)).unwrap();
        assert_eq!(root, None);
        let child: Option<i64> = f.db.conn.query_row("SELECT thread_id FROM memories WHERE id = ?1", [c], |r| r.get(0)).unwrap();
        assert_eq!(child, Some(b));
        // The version history and summary line follow the thread.
        let version_owner: i64 = f.db.conn.query_row("SELECT thread_id FROM memory_versions", [], |r| r.get(0)).unwrap();
        assert_eq!(version_owner, b);
        let derivative: Option<String> = f.db.conn.query_row("SELECT derivative FROM memories WHERE id = ?1", [b], |r| r.get(0)).unwrap();
        assert!(derivative.is_some());
        // The thread survives, so its vault files other than the root's remain.
        assert_eq!(preview.vault_files, 1);
    }

    #[test]
    fn notes_are_protected_unless_included() {
        let (f, ids) = fixture();
        let by_app = DeleteFilter { apps: Some(vec!["Memento".into()]), ..Default::default() };
        assert_eq!(run_preview(&f, &by_app).memories, 0);
        let by_time = DeleteFilter { since_ms: Some(0), until_ms: Some(10_000), ..Default::default() };
        let without = run_delete(&f, &by_time);
        assert_eq!((without.memories, without.notes), (7, 0));
        assert_eq!(count(&f.db, &format!("SELECT COUNT(*) FROM memories WHERE id = {}", ids.note)), 1);

        let (f, ids) = fixture();
        let with = DeleteFilter { include_notes: Some(true), ..by_time };
        let preview = run_preview(&f, &with);
        assert_eq!((preview.memories, preview.notes), (8, 1));
        run_delete(&f, &with);
        assert_eq!(count(&f.db, &format!("SELECT COUNT(*) FROM memories WHERE id = {}", ids.note)), 0);
    }

    #[test]
    fn time_range_is_inclusive_of_start_and_exclusive_of_end() {
        let (f, _) = fixture();
        let range = |since, until| DeleteFilter {
            source_ids: Some(vec!["slack".into()]),
            since_ms: since,
            until_ms: until,
            ..Default::default()
        };
        assert_eq!(run_preview(&f, &range(Some(2_000), Some(3_000))).memories, 1);
        assert_eq!(run_preview(&f, &range(Some(2_001), Some(3_000))).memories, 0);
        assert_eq!(run_preview(&f, &range(Some(2_000), Some(3_001))).memories, 2);
        assert_eq!(run_preview(&f, &range(Some(3_000), None)).memories, 1);
        assert!(preview_sql(&f.db, &range(Some(3_000), Some(3_000)), None).is_err());
    }

    #[test]
    fn untagged_captures_are_selected_by_app_name_only() {
        let (f, ids) = fixture();
        let filter = DeleteFilter { apps: Some(vec!["Foo".into()]), ..Default::default() };
        assert_eq!(run_preview(&f, &filter).memories, 1);
        // A tagged capture is never swept up by an app-name match.
        let chrome = DeleteFilter { apps: Some(vec!["Google Chrome".into()]), ..Default::default() };
        assert_eq!(run_preview(&f, &chrome).memories, 0);
        run_delete(&f, &filter);
        assert_eq!(count(&f.db, &format!("SELECT COUNT(*) FROM memories WHERE id = {}", ids.foo)), 0);
    }

    #[test]
    fn recordings_are_only_deleted_when_asked_and_only_inside_the_recordings_folder() {
        let (f, ids) = fixture();
        let outside = f.dir.join("outside.wav");
        fs::write(&outside, b"keep").unwrap();
        f.db.conn.execute("UPDATE conversations SET audio_path = audio_path || ?1 WHERE kind = 'meeting'", [format!(",{}", outside.display())]).unwrap();

        let meetings = DeleteFilter { source_ids: Some(vec!["meeting".into()]), ..Default::default() };
        let without = run_delete(&f, &meetings);
        assert_eq!((without.memories, without.recordings), (1, 0));
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM conversations"), 2);
        assert_eq!(count(&f.db, &format!("SELECT COUNT(*) FROM memories WHERE id = {}", ids.meeting)), 0);

        let with = DeleteFilter { include_recordings: Some(true), ..meetings.clone() };
        let preview = run_preview(&f, &with);
        assert_eq!((preview.recordings, preview.recording_files), (1, 2));
        assert_eq!(preview.bytes_freed.recordings, 200);
        let result = run_delete(&f, &with);
        assert_eq!(serde_json::to_value(&preview).unwrap(), serde_json::to_value(&result).unwrap());
        // Voice-note recording untouched (not selected); outside file untouched.
        assert_eq!(count(&f.db, "SELECT COUNT(*) FROM conversations"), 1);
        assert!(outside.exists());
        assert!(!f.recordings.join("m1-mic.wav").exists());
        assert!(f.recordings.join("v1-mic.wav").exists());
    }

    #[test]
    fn summary_groups_by_source_notes_and_app() {
        let (f, _) = fixture();
        let (sources, totals) = summary_sql(&f.db.conn).unwrap();
        let find = |id: &str| sources.iter().find(|source| source.id == id).unwrap();
        assert_eq!(find("slack").memories, 3);
        assert_eq!(find("slack").name, "Slack");
        assert_eq!((find("slack").first_at, find("slack").last_at), (Some(1_000), Some(3_000)));
        assert_eq!(find("gmail").memories, 2);
        assert_eq!(find("meeting").memories, 1);
        assert_eq!(find("notes").memories, 1);
        let foo = find("app:Foo");
        assert_eq!((foo.kind.as_str(), foo.app.as_deref()), ("app", Some("Foo")));
        assert_eq!(totals.memories, 8);
        assert_eq!(totals.recordings, 2);
        assert_eq!((totals.first_at, totals.last_at), (Some(1_000), Some(5_000)));
        assert!(totals.bytes > 0);
    }

    #[test]
    fn filtered_listing_browses_one_source() {
        let (f, ids) = fixture();
        let slack = list_filtered(&f.db.conn, Some("slack"), None, None, None, 10, 0).unwrap();
        assert_eq!(slack.iter().map(|m| m.id).collect::<Vec<_>>(), vec![ids.slack[2], ids.slack[1], ids.slack[0]]);
        let page = list_filtered(&f.db.conn, Some("slack"), None, None, None, 1, 1).unwrap();
        assert_eq!(page[0].id, ids.slack[1]);
        let notes = list_filtered(&f.db.conn, Some("notes"), None, None, None, 10, 0).unwrap();
        assert_eq!(notes[0].id, ids.note);
        let foo = list_filtered(&f.db.conn, None, Some("Foo"), None, None, 10, 0).unwrap();
        assert_eq!(foo.len(), 1);
        let ranged = list_filtered(&f.db.conn, None, None, Some(4_000), None, 10, 0).unwrap();
        assert_eq!(ranged.len(), 2);
    }
}
