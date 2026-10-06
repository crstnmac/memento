//! Human-readable, Obsidian-compatible mirror of Memento's memory database.
//!
//! SQLite remains authoritative for captured activity. The vault is the durable
//! portability seam: generated memories are readable Markdown, while files in
//! `notes/` are editable inputs that are ingested back into SQLite.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, TimeZone};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::db::{self, Db};
use crate::state::MonitorState;

const SYNC_INTERVAL: Duration = Duration::from_secs(60);
const README: &str = r#"# Memento Memory

This folder is a portable, Obsidian-compatible view of your local memory.

- `activity/` contains captured work activity.
- `meetings/` contains meeting and voice-note transcripts.
- `notes/` is yours: add or edit Markdown here and Memento will ingest it.
- `summaries/daily/` links each day's memories into a browsable timeline.

SQLite remains the source of truth for captured data. Generated files outside
`notes/` may be refreshed by Memento. Use `[[wiki links]]`, tags, and normal
Markdown freely inside your notes.
"#;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub path: String,
    pub exists: bool,
    pub generated_files: i64,
    pub imported_notes: i64,
    pub last_synced_at: Option<i64>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub exported: i64,
    pub imported: i64,
    pub updated: i64,
    pub unchanged: i64,
    pub path: String,
}

struct ExportMemory {
    id: i64,
    source: String,
    app: Option<String>,
    title: String,
    content: String,
    created_at: i64,
    is_meeting: bool,
    derivative: Option<String>,
    enrichment: Option<String>,
}

pub fn spawn(state: Arc<MonitorState>) {
    std::thread::spawn(move || {
        // Give startup migrations and UI initialization a moment to settle.
        std::thread::sleep(Duration::from_secs(3));
        while state.running.load(std::sync::atomic::Ordering::Relaxed) {
            if let Err(error) = sync_shared(&state) {
                log::warn!("memory vault sync failed: {error}");
            }
            std::thread::sleep(SYNC_INTERVAL);
        }
    });
}

pub fn status(db: &Db) -> Result<VaultStatus, String> {
    let root = vault_dir(db)?;
    let generated_files = if root.exists() {
        count_markdown(&root, false)?
    } else {
        0
    };
    let imported_notes = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM vault_documents WHERE memory_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let last_synced_at = db
        .get_setting("vaultLastSyncedAt")?
        .and_then(|value| value.parse::<i64>().ok());
    Ok(VaultStatus {
        path: root.to_string_lossy().into_owned(),
        exists: root.exists(),
        generated_files,
        imported_notes,
        last_synced_at,
    })
}

/// Background/manual sync. The shared DB lock is held only to read (and import
/// notes); rendering and the file writes — the slow part for a large vault —
/// happen without it so the UI and capture are never stalled behind disk I/O.
pub fn sync_shared(state: &MonitorState) -> Result<SyncReport, String> {
    let mut report = SyncReport::default();
    // A delete that lands while files are being written would otherwise be
    // undone by this (stale) pass, so redo the pass if one happened.
    for _ in 0..3 {
        let generation = REMOVAL_GENERATION.load(Ordering::SeqCst);
        let (root, memories) = {
            let db = state.db();
            let root = vault_dir(&db)?;
            ensure_layout(&root)?;
            report = SyncReport {
                path: root.to_string_lossy().into_owned(),
                ..Default::default()
            };
            import_notes(&db, &root, &mut report)?;
            let memories = load_memories(&db)?;
            (root, memories)
        };
        export_memories(&root, &memories, &mut report)?;
        if REMOVAL_GENERATION.load(Ordering::SeqCst) == generation {
            break;
        }
    }
    state
        .db()
        .set_setting("vaultLastSyncedAt", &db::now_ms().to_string())?;
    Ok(report)
}

fn export_memories(
    root: &Path,
    memories: &[ExportMemory],
    report: &mut SyncReport,
) -> Result<(), String> {
    let mut days: BTreeMap<String, Vec<(String, String, Option<String>)>> = BTreeMap::new();
    let removed = REMOVED_IDS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_default();
    for memory in memories {
        if removed.contains(&memory.id) {
            continue;
        }
        let relative = export_path(memory);
        let markdown = render_memory(memory);
        if write_if_changed(root, &relative, &markdown)? {
            report.exported += 1;
        } else {
            report.unchanged += 1;
        }
        let day = local_datetime(memory.created_at)
            .format("%Y-%m-%d")
            .to_string();
        days.entry(day).or_default().push((
            relative.trim_end_matches(".md").to_string(),
            memory.title.clone(),
            memory
                .derivative
                .clone()
                .or_else(|| enrichment_summary(memory.enrichment.as_deref())),
        ));
    }
    for (day, entries) in days {
        let relative = format!("summaries/daily/{day}.md");
        let markdown = render_daily(&day, &entries);
        if write_if_changed(root, &relative, &markdown)? {
            report.exported += 1;
        }
    }
    Ok(())
}

pub fn open(db: &Db, in_obsidian: bool) -> Result<(), String> {
    let root = vault_dir(db)?;
    ensure_layout(&root)?;
    let status = if in_obsidian {
        Command::new("open")
            .args(["-a", "Obsidian"])
            .arg(&root)
            .status()
    } else {
        Command::new("open").arg(&root).status()
    }
    .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else if in_obsidian {
        Err("Could not open Obsidian. Install it or use Reveal in Finder instead.".into())
    } else {
        Err("Could not reveal the memory vault in Finder.".into())
    }
}

pub(crate) fn vault_dir(db: &Db) -> Result<PathBuf, String> {
    let configured = db.get_setting("vaultPath")?;
    let root = configured
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or(db::app_data_dir()?.join("Memento Memory"));
    if !root.is_absolute() {
        return Err("The memory vault path must be absolute".into());
    }
    Ok(root)
}

fn ensure_layout(root: &Path) -> Result<(), String> {
    for path in [
        root.to_path_buf(),
        root.join("activity"),
        root.join("meetings"),
        root.join("notes"),
        root.join("summaries/daily"),
    ] {
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    }
    write_if_changed(root, "README.md", README)?;
    Ok(())
}

fn load_memories(db: &Db) -> Result<Vec<ExportMemory>, String> {
    let mut stmt = db
        .conn
        .prepare(
            "SELECT m.id, m.source, m.app, m.title, m.content, m.created_at,
                m.is_meeting, m.derivative, e.structured_json
         FROM memories m
         LEFT JOIN memory_enrichments e ON e.memory_id = m.id
         WHERE m.source != 'wiki-note'
         ORDER BY m.created_at ASC, m.id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ExportMemory {
                id: row.get(0)?,
                source: row.get(1)?,
                app: row.get(2)?,
                title: row.get(3)?,
                content: row.get(4)?,
                created_at: row.get(5)?,
                is_meeting: row.get::<_, i64>(6)? != 0,
                derivative: row.get(7)?,
                enrichment: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn import_notes(db: &Db, root: &Path, report: &mut SyncReport) -> Result<(), String> {
    let notes = root.join("notes");
    for path in markdown_files(&notes)? {
        let relative = path
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let raw = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let hash = content_hash(&raw);
        let previous: Option<(Option<i64>, String)> = db
            .conn
            .query_row(
                "SELECT memory_id, content_hash FROM vault_documents WHERE relative_path = ?1",
                params![relative],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if previous
            .as_ref()
            .is_some_and(|(_, old_hash)| old_hash == &hash)
        {
            continue;
        }
        let (frontmatter, body) = split_frontmatter(&raw);
        let title = frontmatter_value(frontmatter, "title")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| title_from_path(&path));
        let content = body.trim();
        if content.is_empty() {
            continue;
        }
        let memory_id = match previous.and_then(|(id, _)| id) {
            Some(id) => {
                db.conn.execute(
                    "UPDATE memories SET title = ?1, content = ?2 WHERE id = ?3 AND source = 'wiki-note'",
                    params![title, content, id],
                ).map_err(|error| error.to_string())?;
                report.updated += 1;
                id
            }
            None => {
                let memory = db.insert_memory(
                    "wiki-note",
                    Some("Obsidian"),
                    &title,
                    content,
                    false,
                    true,
                    None,
                )?;
                report.imported += 1;
                memory.id
            }
        };
        db.conn
            .execute(
                "INSERT INTO vault_documents (relative_path, memory_id, content_hash, synced_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(relative_path) DO UPDATE SET memory_id=excluded.memory_id,
             content_hash=excluded.content_hash, synced_at=excluded.synced_at",
                params![relative, memory_id, hash, db::now_ms()],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn export_path(memory: &ExportMemory) -> String {
    let date = local_datetime(memory.created_at)
        .format("%Y-%m-%d")
        .to_string();
    let slug = slugify(&memory.title);
    if memory.is_meeting || matches!(memory.source.as_str(), "meeting" | "voice-note") {
        format!("meetings/{date}--{slug}--{}.md", memory.id)
    } else {
        let app = slugify(memory.app.as_deref().unwrap_or(&memory.source));
        format!("activity/{date}/{app}/{}--{slug}.md", memory.id)
    }
}

fn render_memory(memory: &ExportMemory) -> String {
    let timestamp = local_datetime(memory.created_at).to_rfc3339();
    let summary = memory
        .derivative
        .clone()
        .or_else(|| enrichment_summary(memory.enrichment.as_deref()));
    let mut output = format!(
        "---\nmemento_id: {}\ntitle: {}\nsource: {}\napp: {}\ncreated_at: {}\ngenerated: true\n---\n\n# {}\n\n",
        memory.id, yaml_string(&memory.title), yaml_string(&memory.source),
        yaml_string(memory.app.as_deref().unwrap_or("")), yaml_string(&timestamp), memory.title.trim(),
    );
    if let Some(summary) = summary.filter(|value| !value.trim().is_empty()) {
        output.push_str(&format!("> {}\n\n", summary.trim().replace('\n', " ")));
    }
    output.push_str(memory.content.trim());
    output.push('\n');
    output
}

fn render_daily(day: &str, entries: &[(String, String, Option<String>)]) -> String {
    let mut output =
        format!("---\nkind: daily-index\ndate: {day}\ngenerated: true\n---\n\n# {day}\n\n");
    for (path, title, summary) in entries {
        output.push_str(&format!("- [[{path}|{}]]", title.trim()));
        if let Some(summary) = summary.as_deref().filter(|value| !value.trim().is_empty()) {
            output.push_str(&format!(" — {}", summary.trim().replace('\n', " ")));
        }
        output.push('\n');
    }
    output
}

fn enrichment_summary(raw: Option<&str>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw?).ok()?;
    ["summary", "derivative", "description"]
        .iter()
        .find_map(|key| {
            value
                .get(key)
                .and_then(|item| item.as_str())
                .map(str::to_string)
        })
}

/// Last content hash + file mtime we wrote per path. Lets a repeat sync skip
/// re-reading every unchanged file, while an outside edit (new mtime) is still
/// detected and overwritten.
static WRITTEN: std::sync::Mutex<Option<std::collections::HashMap<PathBuf, (u64, std::time::SystemTime)>>> =
    std::sync::Mutex::new(None);

fn content_fingerprint(content: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

fn write_if_changed(
    root: &Path,
    relative: impl AsRef<Path>,
    content: &str,
) -> Result<bool, String> {
    let path = safe_join(root, relative.as_ref())?;
    let fingerprint = content_fingerprint(content);
    let modified = fs::metadata(&path).and_then(|meta| meta.modified()).ok();
    {
        let cache = WRITTEN.lock().unwrap_or_else(|p| p.into_inner());
        if let (Some(modified), Some(entry)) = (
            modified,
            cache.as_ref().and_then(|cache| cache.get(&path)),
        ) {
            if entry.0 == fingerprint && entry.1 == modified {
                return Ok(false);
            }
        }
    }
    let unchanged = modified.is_some() && fs::read_to_string(&path).ok().as_deref() == Some(content);
    if !unchanged {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let temporary = path.with_extension("md.tmp");
        fs::write(&temporary, content).map_err(|error| error.to_string())?;
        fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
    }
    if let Ok(modified) = fs::metadata(&path).and_then(|meta| meta.modified()) {
        WRITTEN
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_or_insert_with(Default::default)
            .insert(path, (fingerprint, modified));
    }
    Ok(!unchanged)
}

fn safe_join(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Unsafe vault path".into());
    }
    Ok(root.join(relative))
}

fn markdown_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("md"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn count_markdown(root: &Path, include_notes: bool) -> Result<i64, String> {
    Ok(markdown_files(root)?
        .into_iter()
        .filter(|path| include_notes || !path.starts_with(root.join("notes")))
        .count() as i64)
}

fn split_frontmatter(raw: &str) -> (&str, &str) {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return ("", raw);
    };
    let Some(end) = rest.find("\n---\n") else {
        return ("", raw);
    };
    (&rest[..end], &rest[end + 5..])
}

fn frontmatter_value(frontmatter: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    frontmatter
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(str::trim)
        .map(|value| value.trim_matches('"').replace("\\\"", "\""))
}

fn title_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled note")
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn slugify(value: &str) -> String {
    let mut slug = String::with_capacity(value.len().min(80));
    let mut separator = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            separator = false;
        } else if !separator && !slug.is_empty() {
            slug.push('-');
            separator = true;
        }
        if slug.len() >= 64 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "memory".into()
    } else {
        slug.into()
    }
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

fn content_hash(content: &str) -> String {
    // Stable FNV-1a is sufficient here: this is a change detector, not a
    // security primitive, and unlike DefaultHasher it is stable across runs.
    let hash = content
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}

fn local_datetime(timestamp_ms: i64) -> DateTime<Local> {
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .unwrap_or_else(Local::now)
}

// ---- removal (data lifecycle) ----

/// Bumped on every deletion so an in-flight `sync_shared` knows its snapshot
/// is stale and runs again.
static REMOVAL_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Memory ids deleted by the user. Ids are never reused (AUTOINCREMENT), so a
/// stale sync snapshot can safely skip them forever instead of re-exporting.
static REMOVED_IDS: std::sync::Mutex<Option<std::collections::HashSet<i64>>> =
    std::sync::Mutex::new(None);

/// Just enough of a deleted memory to find the Markdown file it was exported to.
#[derive(Clone, Debug)]
pub struct ExportRef {
    pub id: i64,
    pub source: String,
    pub app: Option<String>,
    pub title: String,
    pub created_at: i64,
    pub is_meeting: bool,
}

impl ExportRef {
    fn as_export(&self) -> ExportMemory {
        ExportMemory {
            id: self.id,
            source: self.source.clone(),
            app: self.app.clone(),
            title: self.title.clone(),
            content: String::new(),
            created_at: self.created_at,
            is_meeting: self.is_meeting,
            derivative: None,
            enrichment: None,
        }
    }

    fn is_meeting_file(&self) -> bool {
        self.is_meeting || matches!(self.source.as_str(), "meeting" | "voice-note")
    }
}

/// Daily index that must be rewritten (or removed when empty) after a delete.
pub struct DayIndex {
    day: String,
    entries: Vec<(String, String, Option<String>)>,
}

pub struct RemovalJob {
    root: PathBuf,
    refs: Vec<ExportRef>,
    days: Vec<DayIndex>,
}

#[derive(Default, Debug)]
pub struct RemovalReport {
    pub files_removed: i64,
    pub bytes_removed: i64,
    pub failed: i64,
    pub indexes_refreshed: i64,
}

/// Vault-relative path a memory is exported to.
#[cfg(test)]
pub fn relative_path(item: &ExportRef) -> String {
    export_path(&item.as_export())
}

/// Generated files (`notes/` is the user's own input and is never touched).
pub fn is_exported(source: &str) -> bool {
    source != "wiki-note"
}

/// Existing exported files for these memories. Pure filesystem work; call it
/// without the DB lock. Besides the canonical path it finds files left under
/// an older title slug, by id.
pub fn matching_files(root: &Path, refs: &[ExportRef]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut listings: std::collections::HashMap<(PathBuf, bool), std::collections::HashMap<i64, Vec<PathBuf>>> =
        Default::default();
    for item in refs {
        let relative = export_path(&item.as_export());
        let Ok(path) = safe_join(root, Path::new(&relative)) else {
            continue;
        };
        if path.is_file() {
            found.push(path.clone());
        }
        let Some(dir) = path.parent().map(Path::to_path_buf) else {
            continue;
        };
        let by_prefix = !item.is_meeting_file();
        let listing = listings
            .entry((dir.clone(), by_prefix))
            .or_insert_with(|| index_directory(&dir, by_prefix));
        if let Some(paths) = listing.get(&item.id) {
            found.extend(paths.iter().cloned());
        }
    }
    found.sort();
    found.dedup();
    found
}

/// id -> files in `dir`, parsed from `{id}--slug.md` (activity) or
/// `{date}--slug--{id}.md` (meetings). Symlinks are ignored.
fn index_directory(dir: &Path, by_prefix: bool) -> std::collections::HashMap<i64, Vec<PathBuf>> {
    let mut map: std::collections::HashMap<i64, Vec<PathBuf>> = Default::default();
    let Ok(entries) = fs::read_dir(dir) else {
        return map;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".md") else {
            continue;
        };
        let part = if by_prefix {
            stem.split("--").next()
        } else {
            stem.rsplit("--").next()
        };
        if let Some(id) = part.and_then(|value| value.parse::<i64>().ok()) {
            map.entry(id).or_default().push(entry.path());
        }
    }
    map
}

/// Called right after the deleting transaction commits, still under the DB
/// lock: keeps a concurrent sync from resurrecting the files and snapshots the
/// surviving entries of each affected day for the index rewrite. Only SQL and
/// pure computation happen here; the file work is `run_removal`.
pub fn prepare_removal(db: &Db, refs: Vec<ExportRef>) -> Result<Option<RemovalJob>, String> {
    let refs: Vec<ExportRef> = refs.into_iter().filter(|item| is_exported(&item.source)).collect();
    if refs.is_empty() {
        return Ok(None);
    }
    {
        let mut removed = REMOVED_IDS.lock().unwrap_or_else(|p| p.into_inner());
        removed
            .get_or_insert_with(Default::default)
            .extend(refs.iter().map(|item| item.id));
    }
    REMOVAL_GENERATION.fetch_add(1, Ordering::SeqCst);
    let root = vault_dir(db)?;
    if !root.exists() {
        return Ok(None);
    }
    let mut days: Vec<String> = refs
        .iter()
        .map(|item| local_datetime(item.created_at).format("%Y-%m-%d").to_string())
        .collect();
    days.sort();
    days.dedup();
    let mut stmt = db
        .conn
        .prepare(
            "SELECT m.id, m.source, m.app, m.title, m.created_at, m.is_meeting, m.derivative, e.structured_json
             FROM memories m LEFT JOIN memory_enrichments e ON e.memory_id = m.id
             WHERE m.source != 'wiki-note' AND m.created_at >= ?1 AND m.created_at < ?2
             ORDER BY m.created_at ASC, m.id ASC",
        )
        .map_err(|error| error.to_string())?;
    let mut indexes = Vec::with_capacity(days.len());
    for day in days {
        let (start, end) = day_bounds(&day)?;
        let rows = stmt
            .query_map(params![start, end], |row| {
                let memory = ExportMemory {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    app: row.get(2)?,
                    title: row.get(3)?,
                    content: String::new(),
                    created_at: row.get(4)?,
                    is_meeting: row.get::<_, i64>(5)? != 0,
                    derivative: row.get(6)?,
                    enrichment: row.get(7)?,
                };
                Ok(memory)
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let entries = rows
            .iter()
            .map(|memory| {
                (
                    export_path(memory).trim_end_matches(".md").to_string(),
                    memory.title.clone(),
                    memory
                        .derivative
                        .clone()
                        .or_else(|| enrichment_summary(memory.enrichment.as_deref())),
                )
            })
            .collect();
        indexes.push(DayIndex { day, entries });
    }
    Ok(Some(RemovalJob { root, refs, days: indexes }))
}

fn day_bounds(day: &str) -> Result<(i64, i64), String> {
    let date = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").map_err(|error| error.to_string())?;
    let start = |date: chrono::NaiveDate| -> Result<i64, String> {
        let midnight = date.and_hms_opt(0, 0, 0).ok_or("invalid date")?;
        Ok(Local
            .from_local_datetime(&midnight)
            .earliest()
            .map(|value| value.timestamp_millis())
            .unwrap_or_else(|| midnight.and_utc().timestamp_millis()))
    };
    let next = date.succ_opt().ok_or("invalid date")?;
    Ok((start(date)?, start(next)?))
}

/// Delete the exported files and refresh the daily indexes. No DB access.
pub fn run_removal(job: RemovalJob) -> RemovalReport {
    let mut report = RemovalReport::default();
    for path in matching_files(&job.root, &job.refs) {
        let size = fs::metadata(&path).map(|meta| meta.len() as i64).unwrap_or(0);
        match fs::remove_file(&path) {
            Ok(()) => {
                report.files_removed += 1;
                report.bytes_removed += size;
                forget_written(&path);
                prune_empty_dirs(&job.root, &path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => report.failed += 1,
        }
    }
    for index in job.days {
        let relative = format!("summaries/daily/{}.md", index.day);
        let Ok(path) = safe_join(&job.root, Path::new(&relative)) else {
            continue;
        };
        if index.entries.is_empty() {
            if fs::remove_file(&path).is_ok() {
                forget_written(&path);
                report.indexes_refreshed += 1;
            }
        } else if write_if_changed(&job.root, &relative, &render_daily(&index.day, &index.entries))
            .unwrap_or(false)
        {
            report.indexes_refreshed += 1;
        }
    }
    report
}

fn forget_written(path: &Path) {
    if let Some(cache) = WRITTEN.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        cache.remove(path);
    }
}

/// Remove now-empty `activity/<date>/<app>` folders; never the top-level ones.
fn prune_empty_dirs(root: &Path, file: &Path) {
    let keep = [root.join("activity"), root.join("meetings")];
    let mut dir = file.parent();
    while let Some(current) = dir {
        if !current.starts_with(root) || current == root || keep.iter().any(|path| path == current) {
            break;
        }
        if fs::remove_dir(current).is_err() {
            break;
        }
        dir = current.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_stable_and_safe() {
        let memory = ExportMemory {
            id: 42,
            source: "capture".into(),
            app: Some("Visual Studio Code".into()),
            title: "Plan: Memory / Vault".into(),
            content: "x".into(),
            created_at: 1_700_000_000_000,
            is_meeting: false,
            derivative: None,
            enrichment: None,
        };
        let path = export_path(&memory);
        assert!(path.ends_with("/visual-studio-code/42--plan-memory-vault.md"));
        assert!(safe_join(Path::new("/tmp/vault"), Path::new(&path)).is_ok());
        assert!(safe_join(Path::new("/tmp/vault"), Path::new("../escape.md")).is_err());
    }

    #[test]
    fn frontmatter_is_removed_from_ingested_notes() {
        let raw = "---\ntitle: \"Board call\"\ntags: work\n---\n\nDecided to ship.";
        let (frontmatter, body) = split_frontmatter(raw);
        assert_eq!(
            frontmatter_value(frontmatter, "title").as_deref(),
            Some("Board call")
        );
        assert_eq!(body.trim(), "Decided to ship.");
    }

    #[test]
    fn removal_deletes_files_refreshes_index_and_blocks_stale_exports() {
        let root = std::env::temp_dir().join(format!("memento-vault-removal-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("notes")).unwrap();
        let db = db::open_in_memory_for_test().unwrap();
        db.set_setting("vaultPath", &root.to_string_lossy()).unwrap();
        let day = 1_700_000_000_000_i64;
        let make = |id: i64, title: &str| ExportMemory {
            id,
            source: "capture".into(),
            app: Some("Slack".into()),
            title: title.into(),
            content: "body".into(),
            created_at: day,
            is_meeting: false,
            derivative: None,
            enrichment: None,
        };
        let stale = vec![make(9_000_001, "Gone"), make(9_000_002, "Kept")];
        let mut report = SyncReport::default();
        export_memories(&root, &stale, &mut report).unwrap();
        let gone = root.join(export_path(&stale[0]));
        let kept = root.join(export_path(&stale[1]));
        assert!(gone.is_file() && kept.is_file());
        db.conn
            .execute(
                "INSERT INTO memories (id, source, app, title, content, created_at) VALUES (9000002, 'capture', 'Slack', 'Kept', 'body', ?1)",
                [day],
            )
            .unwrap();

        let refs = vec![ExportRef {
            id: 9_000_001,
            source: "capture".into(),
            app: Some("Slack".into()),
            title: "Gone".into(),
            created_at: day,
            is_meeting: false,
        }];
        let job = prepare_removal(&db, refs).unwrap().unwrap();
        let removal = run_removal(job);
        assert_eq!(removal.files_removed, 1);
        assert!(!gone.exists() && kept.is_file());
        let daily = fs::read_to_string(root.join(format!(
            "summaries/daily/{}.md",
            local_datetime(day).format("%Y-%m-%d")
        )))
        .unwrap();
        assert!(daily.contains("Kept") && !daily.contains("Gone"));

        // A sync that loaded its snapshot before the delete must not bring it back.
        export_memories(&root, &stale, &mut report).unwrap();
        assert!(!gone.exists());
        let _ = fs::remove_dir_all(&root);
    }
}
