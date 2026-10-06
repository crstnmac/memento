use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRow {
    pub id: i64,
    pub source: String,
    pub app: Option<String>,
    pub title: String,
    pub content: String,
    pub created_at: i64,
    pub is_meeting: bool,
    pub open_loop: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEnrichmentRow {
    pub memory_id: i64,
    pub provider: String,
    pub model: String,
    pub structured: serde_json::Value,
    pub enriched_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppVisitRow {
    pub id: i64,
    pub app: String,
    pub window_title: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemRow {
    pub id: i64,
    pub memory_id: Option<i64>,
    pub content: String,
    pub status: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub is_urgent: bool,
    pub is_unread: bool,
    pub sort_order: i64,
    pub remind_at: Option<i64>,
    pub remind_date: Option<String>,
    pub remind_slot: Option<String>,
    pub reminder_shown: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRow {
    pub id: i64,
    pub title: String,
    pub transcript: Option<String>,
    pub audio_path: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub kind: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ActivityConversationRow {
    pub id: i64,
    pub app: String,
    pub title: String,
    pub started_at: i64,
    pub updated_at: i64,
    pub before_chars: i64,
    pub new_chars: i64,
    pub message_count: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConversationEventRow {
    pub id: i64,
    pub conversation_id: i64,
    pub memory_id: Option<i64>,
    pub captured_at: i64,
    pub before_chars: i64,
    pub new_chars: i64,
    pub new_content: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunRow {
    pub id: i64,
    pub kind: String,
    pub prompt: Option<String>,
    pub result: Option<String>,
    pub created_at: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MemoryVersionRow {
    pub id: i64,
    pub thread_id: i64,
    pub parent_version_id: Option<i64>,
    pub created_at: i64,
    pub is_frozen: bool,
    pub word_count: i64,
    pub content: String,
    pub item_count: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRow {
    pub id: i64,
    pub source: String,
    pub app: Option<String>,
    pub title: String,
    pub derivative: Option<String>,
    pub created_at: i64,
    pub item_count: i64,
    pub open_count: i64,
    pub last_activity: i64,
}

pub struct Db {
    pub conn: Connection,
}

pub fn app_data_dir() -> Result<PathBuf, String> {
    // Must match the bundle identifier (dev.poppy.memento).
    dirs::data_local_dir()
        .map(|base| base.join("dev.poppy.memento"))
        .ok_or_else(|| "Could not resolve the app data directory".to_string())
}

pub fn recordings_dir() -> Result<PathBuf, String> {
    let dir = app_data_dir()?.join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    restrict_dir(&dir)?;
    Ok(dir)
}

pub fn open() -> Result<Db, String> {
    let dir = app_data_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    restrict_dir(&dir)?;
    let db_path = dir.join("memento.db");
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    restrict_file(&db_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    // WAL + NORMAL is durable across app crashes and far cheaper per commit;
    // the busy timeout keeps the rare cross-connection write from erroring.
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "temp_store", "MEMORY")
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", db_path.display()));
        if sidecar.exists() {
            restrict_file(&sidecar)?;
        }
    }
    let db = Db { conn };
    db.migrate()?;
    Ok(db)
}

#[cfg(test)]
pub(crate) fn open_in_memory_for_test() -> Result<Db, String> {
    let db = Db {
        conn: Connection::open_in_memory().map_err(|error| error.to_string())?,
    };
    db.conn
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| error.to_string())?;
    db.migrate()?;
    Ok(db)
}

#[cfg(unix)]
fn restrict_dir(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn restrict_dir(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn restrict_file(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

/// Turn free text into a safe FTS5 expression: every whitespace-separated term
/// becomes a quoted phrase (AND-ed), and the last term also matches as a prefix.
fn fts_match_expression(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.replace('"', ""))
        .filter(|term| term.chars().any(char::is_alphanumeric))
        .map(|term| format!("\"{term}\""))
        .collect();
    let last = terms.len().checked_sub(1)?;
    Some(
        terms
            .iter()
            .enumerate()
            .map(|(index, term)| if index == last { format!("{term}*") } else { term.clone() })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

impl Db {
    fn migrate(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS memories (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source TEXT NOT NULL,
                    app TEXT,
                    title TEXT NOT NULL,
                    content TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    is_meeting INTEGER NOT NULL DEFAULT 0,
                    open_loop INTEGER NOT NULL DEFAULT 1,
                    thread_id INTEGER,
                    derivative TEXT,
                    content_hash INTEGER,
                    activity_key TEXT
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                    title, content,
                    content='memories', content_rowid='id'
                );

                CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                    INSERT INTO memories_fts(rowid, title, content)
                    VALUES (new.id, new.title, new.content);
                END;

                CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                    INSERT INTO memories_fts(memories_fts, rowid, title, content)
                    VALUES ('delete', old.id, old.title, old.content);
                END;

                -- Only re-index when the searchable columns change. Updates to
                -- derivative/thread_id (every capture touches the thread root)
                -- must not rewrite the FTS entry of large memories.
                DROP TRIGGER IF EXISTS memories_au;
                CREATE TRIGGER memories_au AFTER UPDATE OF title, content ON memories BEGIN
                    INSERT INTO memories_fts(memories_fts, rowid, title, content)
                    VALUES ('delete', old.id, old.title, old.content);
                    INSERT INTO memories_fts(rowid, title, content)
                    VALUES (new.id, new.title, new.content);
                END;

                CREATE TABLE IF NOT EXISTS app_visits (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    app TEXT NOT NULL,
                    window_title TEXT,
                    started_at INTEGER NOT NULL,
                    ended_at INTEGER
                );

                CREATE TABLE IF NOT EXISTS conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    title TEXT NOT NULL,
                    transcript TEXT,
                    audio_path TEXT,
                    started_at INTEGER NOT NULL,
                    ended_at INTEGER,
                    duration_ms INTEGER,
                    kind TEXT NOT NULL DEFAULT 'meeting'
                );

                CREATE TABLE IF NOT EXISTS activity_conversations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    app TEXT NOT NULL,
                    title TEXT NOT NULL,
                    current_content TEXT NOT NULL,
                    current_hash INTEGER NOT NULL,
                    started_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    before_chars INTEGER NOT NULL DEFAULT 0,
                    new_chars INTEGER NOT NULL DEFAULT 0,
                    message_count INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS activity_conversation_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    conversation_id INTEGER NOT NULL REFERENCES activity_conversations(id) ON DELETE CASCADE,
                    memory_id INTEGER REFERENCES memories(id) ON DELETE SET NULL,
                    captured_at INTEGER NOT NULL,
                    before_chars INTEGER NOT NULL,
                    new_chars INTEGER NOT NULL,
                    content_hash INTEGER NOT NULL,
                    new_content TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS activity_conversation_messages (
                    conversation_id INTEGER NOT NULL REFERENCES activity_conversations(id) ON DELETE CASCADE,
                    fingerprint INTEGER NOT NULL,
                    text TEXT NOT NULL,
                    first_seen_at INTEGER NOT NULL,
                    PRIMARY KEY (conversation_id, fingerprint)
                );

                CREATE TABLE IF NOT EXISTS action_items (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    memory_id INTEGER REFERENCES memories(id) ON DELETE CASCADE,
                    content TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'open',
                    created_at INTEGER NOT NULL,
                    completed_at INTEGER,
                    is_urgent INTEGER NOT NULL DEFAULT 0,
                    is_unread INTEGER NOT NULL DEFAULT 1,
                    sort_order INTEGER NOT NULL DEFAULT 0,
                    remind_at INTEGER,
                    remind_date TEXT,
                    remind_slot TEXT,
                    reminder_shown INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS agent_runs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    kind TEXT NOT NULL,
                    prompt TEXT,
                    result TEXT,
                    created_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS agent_memory_fingerprints (
                    memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                    content_hash INTEGER NOT NULL,
                    processed_at INTEGER NOT NULL,
                    run_kind TEXT NOT NULL,
                    PRIMARY KEY (memory_id, content_hash)
                );

                CREATE TABLE IF NOT EXISTS memory_enrichments (
                    memory_id INTEGER PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
                    provider TEXT NOT NULL,
                    model TEXT NOT NULL,
                    content_hash INTEGER NOT NULL,
                    structured_json TEXT NOT NULL,
                    enriched_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS memory_versions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    thread_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                    parent_version_id INTEGER REFERENCES memory_versions(id),
                    hour_bucket INTEGER NOT NULL,
                    content TEXT NOT NULL,
                    content_hash INTEGER NOT NULL,
                    is_frozen INTEGER NOT NULL DEFAULT 0,
                    word_count INTEGER NOT NULL DEFAULT 0,
                    created_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS vault_documents (
                    relative_path TEXT PRIMARY KEY,
                    memory_id INTEGER REFERENCES memories(id) ON DELETE SET NULL,
                    content_hash TEXT NOT NULL,
                    synced_at INTEGER NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_mem_thread ON memories(thread_id);
                CREATE INDEX IF NOT EXISTS idx_mem_thread_app ON memories(thread_id, app);
                CREATE INDEX IF NOT EXISTS idx_mem_activity_key ON memories(app, activity_key, created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_mem_capture_hash ON memories(app, content_hash);
                CREATE INDEX IF NOT EXISTS idx_actions_status ON action_items(status);
                CREATE INDEX IF NOT EXISTS idx_actions_remind ON action_items(remind_at);
                CREATE INDEX IF NOT EXISTS idx_visits_started ON app_visits(started_at DESC);
                CREATE INDEX IF NOT EXISTS idx_conversations_started ON conversations(started_at DESC);
                CREATE INDEX IF NOT EXISTS idx_activity_conversations_updated ON activity_conversations(updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_conversation_events_timeline ON activity_conversation_events(conversation_id, captured_at DESC);
                CREATE INDEX IF NOT EXISTS idx_versions_thread ON memory_versions(thread_id, created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_versions_hour ON memory_versions(thread_id, hour_bucket);
                CREATE INDEX IF NOT EXISTS idx_agent_fingerprints_processed ON agent_memory_fingerprints(processed_at DESC);
                CREATE INDEX IF NOT EXISTS idx_enrichments_updated ON memory_enrichments(enriched_at DESC);
                CREATE INDEX IF NOT EXISTS idx_vault_documents_memory ON vault_documents(memory_id);
                "#,
            )
                .map_err(|e| e.to_string())?;
        // Additive migrations for databases created by earlier versions.
        let has_service = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('memories') WHERE name = 'service'")
            .and_then(|mut stmt| stmt.exists([]))
            .map_err(|e| e.to_string())?;
        if !has_service {
            self.conn
                .execute("ALTER TABLE memories ADD COLUMN service TEXT", [])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    // ---- follow-up sources ----

    /// Tag a capture with the follow-up source (app/service) it came from.
    pub fn set_memory_service(&self, memory_id: i64, service: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE memories SET service = ?1 WHERE id = ?2",
                params![service, memory_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Source ids the user switched off for follow-up detection.
    pub fn disabled_follow_up_sources(&self) -> Result<Vec<String>, String> {
        Ok(self
            .get_setting("followUpDisabledSources")?
            .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
            .unwrap_or_default())
    }

    pub fn set_follow_up_source_enabled(
        &self,
        id: &str,
        enabled: bool,
    ) -> Result<Vec<String>, String> {
        let mut disabled = self.disabled_follow_up_sources()?;
        disabled.retain(|existing| existing != id);
        if !enabled {
            disabled.push(id.to_string());
        }
        disabled.sort();
        let json = serde_json::to_string(&disabled).map_err(|e| e.to_string())?;
        self.set_setting("followUpDisabledSources", &json)?;
        Ok(disabled)
    }

    /// Whether follow-ups should be skipped for this memory because the user
    /// switched its source off. Memories with no known source (user notes,
    /// rows captured before sources were tracked) are never skipped.
    fn follow_up_source_disabled(&self, memory_id: i64) -> Result<bool, String> {
        let source: Option<String> = self
            .conn
            .query_row(
                "SELECT COALESCE(service, CASE source WHEN 'meeting' THEN 'meeting'
                                                     WHEN 'voice-note' THEN 'voice-note' END)
                 FROM memories WHERE id = ?1",
                params![memory_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .flatten();
        let Some(source) = source else {
            return Ok(false);
        };
        Ok(self.disabled_follow_up_sources()?.contains(&source))
    }

    // ---- settings ----

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn memories_needing_enrichment(&self, limit: i64) -> Result<Vec<MemoryRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"SELECT m.id, m.source, m.app, m.title, m.content, m.created_at,
                      m.is_meeting, m.open_loop
               FROM memories m
               LEFT JOIN memory_enrichments x ON x.memory_id = m.id
               WHERE x.memory_id IS NULL
                  OR json_type(x.structured_json, '$.actionItems') IS NULL
               ORDER BY m.created_at ASC LIMIT ?1"#,
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 100)], memory_from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn save_memory_enrichment(
        &self,
        memory_id: i64,
        provider: &str,
        model: &str,
        content_hash: i64,
        structured_json: &str,
        derivative: &str,
    ) -> Result<(), String> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO memory_enrichments
             (memory_id, provider, model, content_hash, structured_json, enriched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(memory_id) DO UPDATE SET provider=excluded.provider,
             model=excluded.model, content_hash=excluded.content_hash,
             structured_json=excluded.structured_json, enriched_at=excluded.enriched_at",
            params![
                memory_id,
                provider,
                model,
                content_hash,
                structured_json,
                now_ms()
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE memories SET derivative=?1 WHERE id=?2",
            params![derivative, memory_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn list_memory_enrichments(&self, ids: &[i64]) -> Result<Vec<MemoryEnrichmentRow>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT memory_id, provider, model, structured_json, enriched_at
                           FROM memory_enrichments WHERE memory_id IN ({placeholders})"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(ids), |row| {
                let raw: String = row.get(3)?;
                Ok(MemoryEnrichmentRow {
                    memory_id: row.get(0)?,
                    provider: row.get(1)?,
                    model: row.get(2)?,
                    structured: serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null),
                    enriched_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    // ---- memories ----

    #[allow(clippy::too_many_arguments)]
    pub fn insert_memory(
        &self,
        source: &str,
        app: Option<&str>,
        title: &str,
        content: &str,
        is_meeting: bool,
        open_loop: bool,
        content_hash: Option<u64>,
    ) -> Result<MemoryRow, String> {
        let now = now_ms();
        let mut thread_id: Option<i64> = None;
        let activity_key =
            (source == "capture").then(|| capture_activity_key(app.unwrap_or(""), title, content));
        // Activity captures group by a stable app + topic identity. Returning
        // to the same channel, message, or meeting reconnects its thread even
        // after visiting another app; unrelated topics in one app stay apart.
        if source == "capture" {
            thread_id =
                self.recent_capture_root(app.unwrap_or(""), activity_key.as_deref().unwrap_or(""))?;
        }
        self.conn
            .execute(
                "INSERT INTO memories (source, app, title, content, created_at, is_meeting, open_loop, thread_id, content_hash, activity_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    source,
                    app,
                    title,
                    content,
                    now,
                    is_meeting as i64,
                    open_loop as i64,
                    thread_id,
                    content_hash.map(|h| h as i64),
                    activity_key,
                ],
            )
            .map_err(|e| e.to_string())?;
        let id = self.conn.last_insert_rowid();
        if source == "capture" {
            if thread_id.is_none() {
                self.set_derivative(id, &make_derivative(title, content))?;
            } else if let Some(root) = thread_id {
                self.touch_derivative(root, title)?;
            }
        }
        self.get_memory(id)?
            .ok_or_else(|| "memory disappeared".to_string())
    }

    /// `true` if a capture already exists for this app with the same content
    /// hash, i.e. the same screen/content was already indexed. Used to avoid
    /// storing duplicate versions of unchanged content.
    pub fn capture_hash_exists(&self, app: &str, content_hash: u64) -> Result<bool, String> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories
                 WHERE source = 'capture' AND app = ?1 AND content_hash = ?2",
                params![app, content_hash as i64],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(count > 0)
    }

    /// Find the most recently active root for this exact activity/topic. A
    /// 24-hour horizon reconnects interrupted work without merging unrelated
    /// windows merely because they belong to the same application.
    fn recent_capture_root(&self, app: &str, activity_key: &str) -> Result<Option<i64>, String> {
        let cutoff = now_ms() - 24 * 60 * 60 * 1000;
        self.conn
            .query_row(
                "SELECT root.id FROM memories root
                 WHERE root.source = 'capture' AND root.app = ?1
                   AND root.activity_key = ?2 AND root.thread_id IS NULL
                   AND COALESCE((SELECT MAX(child.created_at) FROM memories child
                                 WHERE child.thread_id = root.id), root.created_at) > ?3
                 ORDER BY COALESCE((SELECT MAX(child.created_at) FROM memories child
                                    WHERE child.thread_id = root.id), root.created_at) DESC
                 LIMIT 1",
                params![app, activity_key, cutoff],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    fn set_derivative(&self, id: i64, derivative: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE memories SET derivative = ?1 WHERE id = ?2",
                params![derivative, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn touch_derivative(&self, root: i64, latest_title: &str) -> Result<(), String> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE thread_id = ?1",
                params![root],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        self.set_derivative(root, &format!("{count} captured moments · {latest_title}"))
    }

    pub fn list_threads(&self, limit: i64) -> Result<Vec<ThreadRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"SELECT root.id, root.source, root.app, root.title, root.derivative,
                          root.created_at,
                          (SELECT COUNT(*) FROM memories m WHERE m.thread_id = root.id) AS item_count,
                          (SELECT COUNT(*) FROM memories m WHERE m.thread_id = root.id AND m.open_loop = 1) AS open_count,
                          COALESCE((SELECT MAX(m.created_at) FROM memories m WHERE m.thread_id = root.id), root.created_at) AS last_activity
                   FROM memories root
                   WHERE root.thread_id IS NULL
                   ORDER BY COALESCE(last_activity, root.created_at) DESC
                   LIMIT ?1"#,
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(ThreadRow {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    app: row.get(2)?,
                    title: row.get(3)?,
                    derivative: row.get(4)?,
                    created_at: row.get(5)?,
                    item_count: row.get(6)?,
                    open_count: row.get(7)?,
                    last_activity: row.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn get_thread(
        &self,
        thread_id: i64,
    ) -> Result<Option<(ThreadRow, Vec<MemoryRow>)>, String> {
        let root = self
            .conn
            .query_row(
                "SELECT id, source, app, title, derivative, created_at,
                        (SELECT COUNT(*) FROM memories m WHERE m.thread_id = memories.id) AS item_count,
                        (SELECT COUNT(*) FROM memories m WHERE m.thread_id = memories.id AND m.open_loop = 1) AS open_count,
                        created_at AS last_activity
                 FROM memories WHERE id = ?1",
                params![thread_id],
                |row| {
                    Ok(ThreadRow {
                        id: row.get(0)?,
                        source: row.get(1)?,
                        app: row.get(2)?,
                        title: row.get(3)?,
                        derivative: row.get(4)?,
                        created_at: row.get(5)?,
                        item_count: row.get(6)?,
                        open_count: row.get(7)?,
                        last_activity: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(root) = root else {
            return Ok(None);
        };
        let members = self.list_thread_items(thread_id)?;
        Ok(Some((root, members)))
    }

    fn list_thread_items(&self, thread_id: i64) -> Result<Vec<MemoryRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"SELECT m.id, m.source, m.app, m.title, m.content, m.created_at,
                          m.is_meeting, m.open_loop
                   FROM memories m
                   WHERE (m.id = ?1 AND m.thread_id IS NULL) OR m.thread_id = ?1
                   ORDER BY m.created_at ASC"#,
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![thread_id], memory_from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    // ---- memory versions (eras of state per thread) ----

    /// Root capture memories — each becomes a "thread" with version history.
    pub fn capture_root_ids(&self) -> Result<Vec<i64>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id FROM memories WHERE source = 'capture' AND thread_id IS NULL
                 ORDER BY created_at DESC LIMIT 2000",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// The latest (highest id) version for a thread, if any.
    pub fn latest_version(&self, thread_id: i64) -> Result<Option<MemoryVersionRow>, String> {
        self.conn
            .query_row(
                "SELECT id, thread_id, parent_version_id, created_at, is_frozen, word_count, content
                 FROM memory_versions WHERE thread_id = ?1 ORDER BY id DESC LIMIT 1",
                params![thread_id],
                |row| {
                    let item_count: i64 = serde_json::from_str::<Vec<serde_json::Value>>(row.get::<_, String>(6).unwrap_or_else(|_| "[]".into()).as_str()).map(|v| v.len() as i64).unwrap_or(0);
                    Ok(MemoryVersionRow {
                        id: row.get(0)?,
                        thread_id: row.get(1)?,
                        parent_version_id: row.get(2)?,
                        created_at: row.get(3)?,
                        is_frozen: row.get::<_, i64>(4)? != 0,
                        word_count: row.get(5)?,
                        content: row.get(6)?,
                        item_count,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    /// The content_hash of the latest version for a thread (None if none).
    pub fn latest_version_hash(&self, thread_id: i64) -> Result<Option<i64>, String> {
        self.conn
            .query_row(
                "SELECT content_hash FROM memory_versions
                 WHERE thread_id = ?1 ORDER BY id DESC LIMIT 1",
                params![thread_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Maintain one consolidated current version per thread/hour. Changes in
    /// the active hour update that row; the next hour creates a child version
    /// after the previous bucket has been frozen.
    pub fn insert_thread_version(
        &self,
        thread_id: i64,
        content: &str,
        hash: u64,
        word_count: i64,
    ) -> Result<i64, String> {
        let now = now_ms();
        let bucket = hour_bucket_ms(now);
        if let Some((id, frozen)) = self
            .conn
            .query_row(
                "SELECT id, is_frozen FROM memory_versions
             WHERE thread_id = ?1 AND hour_bucket = ?2 ORDER BY id DESC LIMIT 1",
                params![thread_id, bucket],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .optional()
            .map_err(|e| e.to_string())?
        {
            if !frozen {
                self.conn
                    .execute(
                        "UPDATE memory_versions
                     SET content = ?1, content_hash = ?2, word_count = ?3, created_at = ?4
                     WHERE id = ?5",
                        params![content, hash as i64, word_count, now, id],
                    )
                    .map_err(|e| e.to_string())?;
                return Ok(id);
            }
        }
        let parent = self.latest_version(thread_id)?.map(|v| v.id);
        self.conn
            .execute(
                "INSERT INTO memory_versions
                    (thread_id, parent_version_id, hour_bucket, content, content_hash, is_frozen, word_count, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7)",
                params![thread_id, parent, bucket, content, hash as i64, word_count, now],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Freeze every non-frozen version whose hour bucket has fully elapsed,
    /// so past hours is "locked" while the current hour can still grow.
    pub fn freeze_elapsed_hours(&self, now: i64) -> Result<i64, String> {
        let current_bucket = hour_bucket_ms(now);
        let frozen = self
            .conn
            .execute(
                "UPDATE memory_versions SET is_frozen = 1
                 WHERE is_frozen = 0 AND hour_bucket < ?1",
                params![current_bucket],
            )
            .map_err(|e| e.to_string())?;
        Ok(frozen as i64)
    }

    pub fn list_thread_versions(
        &self,
        thread_id: i64,
        limit: i64,
    ) -> Result<Vec<MemoryVersionRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, thread_id, parent_version_id, created_at, is_frozen, word_count, content
                 FROM memory_versions WHERE thread_id = ?1 ORDER BY id DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![thread_id, limit], |row| {
                let content: String = row.get(6)?;
                let item_count: i64 = serde_json::from_str::<Vec<serde_json::Value>>(&content)
                    .map(|v| v.len() as i64)
                    .unwrap_or(0);
                Ok(MemoryVersionRow {
                    id: row.get(0)?,
                    thread_id: row.get(1)?,
                    parent_version_id: row.get(2)?,
                    created_at: row.get(3)?,
                    is_frozen: row.get::<_, i64>(4)? != 0,
                    word_count: row.get(5)?,
                    content,
                    item_count,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn get_memory(&self, id: i64) -> Result<Option<MemoryRow>, String> {
        self.conn
            .query_row(
                r#"SELECT m.id, m.source, m.app, m.title, m.content, m.created_at,
                          m.is_meeting, m.open_loop
                   FROM memories m
                   WHERE m.id = ?1"#,
                params![id],
                memory_from_row,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn list_memories(&self, limit: i64, offset: i64) -> Result<Vec<MemoryRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"SELECT m.id, m.source, m.app, m.title, m.content, m.created_at,
                          m.is_meeting, m.open_loop
                   FROM memories m
                   ORDER BY m.created_at DESC
                   LIMIT ?1 OFFSET ?2"#,
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit, offset], memory_from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn delete_memory(&self, id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn search_text(&self, query: &str, limit: i64) -> Result<Vec<MemoryRow>, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        // Raw user text is not valid FTS5 syntax ("-", ":", quotes, AND/OR…),
        // so quote each term and prefix-match the last one for type-ahead.
        if let Some(match_expr) = fts_match_expression(query) {
            let ranked = (|| -> Result<Vec<MemoryRow>, rusqlite::Error> {
                let mut stmt = self.conn.prepare_cached(
                    r#"SELECT m.id, m.source, m.app, m.title, m.content, m.created_at,
                              m.is_meeting, m.open_loop
                       FROM memories_fts
                       JOIN memories m ON m.id = memories_fts.rowid
                       WHERE memories_fts MATCH ?1
                       ORDER BY bm25(memories_fts, 1.0, 1.5)
                       LIMIT ?2"#,
                )?;
                let rows = stmt
                    .query_map(params![match_expr, limit], memory_from_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })();
            if let Ok(rows) = ranked {
                if !rows.is_empty() {
                    return Ok(rows);
                }
            }
        }
        // Fall back to a substring scan (handles punctuation the tokenizer drops).
        let like = format!(
            "%{}%",
            query
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut stmt = self
            .conn
            .prepare_cached(
                r#"SELECT m.id, m.source, m.app, m.title, m.content, m.created_at,
                          m.is_meeting, m.open_loop
                   FROM memories m
                   WHERE m.title LIKE ?1 ESCAPE '\' OR m.content LIKE ?1 ESCAPE '\'
                   ORDER BY m.created_at DESC
                   LIMIT ?2"#,
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![like, limit], memory_from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// A title for a new meeting/voice-note memory that won't collide with an
    /// existing one: if `base` is already used by a meeting memory, we
    /// append " (2)", " (3)", … (extract-meeting title-collision handling).
    pub fn unique_meeting_title(&self, base: &str) -> Result<String, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM memories WHERE source = 'meeting' AND title = ?1")
            .map_err(|e| e.to_string())?;
        let existing: i64 = stmt
            .query_row(params![base], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        if existing == 0 {
            return Ok(base.to_string());
        }
        let mut n = existing + 1;
        loop {
            let candidate = format!("{base} ({n})");
            let count: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM memories WHERE source = 'meeting' AND title = ?1",
                    params![candidate],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if count == 0 {
                return Ok(candidate);
            }
            n += 1;
        }
    }

    // ---- app visits ----

    pub fn open_visit(&self, app: &str, window_title: Option<&str>) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO app_visits (app, window_title, started_at) VALUES (?1, ?2, ?3)",
                params![app, window_title, now_ms()],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn close_visit(&self, id: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE app_visits SET ended_at = ?1 WHERE id = ?2 AND ended_at IS NULL",
                params![now_ms(), id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update_visit_title(&self, id: i64, window_title: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE app_visits SET window_title = ?1 WHERE id = ?2",
                params![window_title, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_visits(&self, limit: i64) -> Result<Vec<AppVisitRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, app, window_title, started_at, ended_at
                 FROM app_visits ORDER BY started_at DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(AppVisitRow {
                    id: row.get(0)?,
                    app: row.get(1)?,
                    window_title: row.get(2)?,
                    started_at: row.get(3)?,
                    ended_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    // ---- action items ----

    /// The canonical non-open status values. The legacy "done" status is
    /// normalized to "resolved" wherever it is written.
    pub const STATUS_OPEN: &'static str = "open";
    pub const STATUS_RESOLVED: &'static str = "resolved";
    pub const STATUS_DISMISSED: &'static str = "dismissed";

    pub fn normalize_status(status: &str) -> &str {
        match status {
            "done" | "resolved" => Self::STATUS_RESOLVED,
            "dismissed" => Self::STATUS_DISMISSED,
            _ => Self::STATUS_OPEN,
        }
    }

    /// Insert an action item with the richer model: urgency, unread flag,
    /// sort order, and an optional scheduled reminder.
    pub fn insert_action_item_ext(
        &self,
        memory_id: i64,
        content: &str,
        urgent: bool,
        sort_order: i64,
        remind_at: Option<i64>,
    ) -> Result<i64, String> {
        let (remind_date, remind_slot) = if remind_at.is_some() {
            split_schedule_long(remind_at)
        } else {
            (None, None)
        };
        self.conn
            .execute(
                "INSERT INTO action_items
                    (memory_id, content, status, created_at, is_urgent, is_unread, sort_order, remind_at, remind_date, remind_slot, reminder_shown)
                 VALUES (?1, ?2, 'open', ?3, ?4, 1, ?5, ?6, ?7, ?8, 0)",
                params![
                    memory_id,
                    content,
                    now_ms(),
                    urgent as i64,
                    sort_order,
                    remind_at,
                    remind_date,
                    remind_slot
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert an LLM-extracted action unless an equivalent action from the
    /// same memory already exists. Enrichment retries are therefore idempotent.
    pub fn insert_action_item_if_missing(
        &self,
        memory_id: i64,
        content: &str,
        urgent: bool,
        remind_at: Option<i64>,
    ) -> Result<Option<i64>, String> {
        if content.trim().is_empty() || self.follow_up_source_disabled(memory_id)? {
            return Ok(None);
        }
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM action_items WHERE memory_id=?1 AND lower(trim(content))=lower(trim(?2)))",
            params![memory_id, content],
            |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if exists {
            return Ok(None);
        }
        // The same commitment re-captured, rephrased by the model, or already
        // resolved/dismissed must not come back as a new follow-up.
        let cutoff = now_ms() - 60 * 86_400_000;
        let recent: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare_cached(
                    "SELECT content FROM action_items WHERE created_at > ?1 ORDER BY id DESC LIMIT 1000",
                )
                .map_err(|error| error.to_string())?;
            let rows = stmt
                .query_map(params![cutoff], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            rows
        };
        if recent.iter().any(|existing| crate::actions::is_similar(existing, content)) {
            return Ok(None);
        }
        let id = self.insert_action_item_ext(memory_id, content, urgent, 0, remind_at)?;
        self.conn
            .execute(
                "UPDATE memories SET open_loop=1 WHERE id=?1",
                params![memory_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(Some(id))
    }

    pub fn list_action_items(&self, status: &str) -> Result<Vec<ActionItemRow>, String> {
        let status = Self::normalize_status(status);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, memory_id, content, status, created_at, completed_at,
                        is_urgent, is_unread, sort_order, remind_at, remind_date, remind_slot, reminder_shown
                 FROM action_items
                 WHERE status = ?1
                 ORDER BY is_urgent DESC, sort_order ASC, created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![status], |row| {
                Ok(ActionItemRow {
                    id: row.get(0)?,
                    memory_id: row.get(1)?,
                    content: row.get(2)?,
                    status: row.get(3)?,
                    created_at: row.get(4)?,
                    completed_at: row.get(5)?,
                    is_urgent: row.get::<_, i64>(6)? != 0,
                    is_unread: row.get::<_, i64>(7)? != 0,
                    sort_order: row.get(8)?,
                    remind_at: row.get(9)?,
                    remind_date: row.get(10)?,
                    remind_slot: row.get(11)?,
                    reminder_shown: row.get::<_, i64>(12)? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// Update arbitrary scheduling/priority fields on an action item.
    pub fn update_action_item(
        &self,
        id: i64,
        urgent: Option<bool>,
        remind_at: Option<Option<i64>>,
        unread: Option<bool>,
        sort_order: Option<i64>,
    ) -> Result<(), String> {
        if let Some(urgent) = urgent {
            self.conn
                .execute(
                    "UPDATE action_items SET is_urgent = ?1 WHERE id = ?2",
                    params![urgent as i64, id],
                )
                .map_err(|e| e.to_string())?;
        }
        if let Some(unread) = unread {
            self.conn
                .execute(
                    "UPDATE action_items SET is_unread = ?1 WHERE id = ?2",
                    params![unread as i64, id],
                )
                .map_err(|e| e.to_string())?;
        }
        if let Some(sort_order) = sort_order {
            self.conn
                .execute(
                    "UPDATE action_items SET sort_order = ?1 WHERE id = ?2",
                    params![sort_order, id],
                )
                .map_err(|e| e.to_string())?;
        }
        if let Some(remind_at) = remind_at {
            let (remind_date, remind_slot) = if remind_at.is_some() {
                split_schedule_long(remind_at)
            } else {
                (None, None)
            };
            self.conn
                .execute(
                    "UPDATE action_items SET remind_at = ?1, remind_date = ?2, remind_slot = ?3, reminder_shown = 0 WHERE id = ?4",
                    params![remind_at, remind_date, remind_slot, id],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn set_action_item_status(&self, id: i64, status: &str) -> Result<(), String> {
        let status = Self::normalize_status(status);
        let completed_at = if status == Self::STATUS_RESOLVED {
            Some(now_ms())
        } else {
            None
        };
        self.conn
            .execute(
                "UPDATE action_items SET status = ?1, completed_at = ?2 WHERE id = ?3",
                params![status, completed_at, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Mark a set of action items' reminder as surfaced (used by the nudge).
    pub fn mark_reminders_shown(&self, ids: &[i64]) -> Result<(), String> {
        for id in ids {
            self.conn
                .execute(
                    "UPDATE action_items SET reminder_shown = 1 WHERE id = ?1",
                    params![id],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Items that are due for a reminder: open, not yet nudged, timed.
    pub fn due_reminder_items(&self, now: i64) -> Result<Vec<ActionItemRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, memory_id, content, status, created_at, completed_at,
                        is_urgent, is_unread, sort_order, remind_at, remind_date, remind_slot, reminder_shown
                 FROM action_items
                 WHERE status = 'open' AND reminder_shown = 0
                       AND remind_at IS NOT NULL AND remind_at <= ?1
                 ORDER BY remind_at ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![now], |row| {
                Ok(ActionItemRow {
                    id: row.get(0)?,
                    memory_id: row.get(1)?,
                    content: row.get(2)?,
                    status: row.get(3)?,
                    created_at: row.get(4)?,
                    completed_at: row.get(5)?,
                    is_urgent: row.get::<_, i64>(6)? != 0,
                    is_unread: row.get::<_, i64>(7)? != 0,
                    sort_order: row.get(8)?,
                    remind_at: row.get(9)?,
                    remind_date: row.get(10)?,
                    remind_slot: row.get(11)?,
                    reminder_shown: row.get::<_, i64>(12)? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    // ---- incremental activity conversations ----

    pub fn record_conversation_snapshot(
        &self,
        app: &str,
        title: &str,
        content: &str,
        memory_id: i64,
    ) -> Result<Option<i64>, String> {
        let now = now_ms();
        let cutoff = now - 2 * 60 * 60 * 1000;
        let existing = self
            .conn
            .query_row(
                "SELECT id, current_content, current_hash FROM activity_conversations
                 WHERE app = ?1 AND title = ?2 AND updated_at >= ?3
                 ORDER BY updated_at DESC LIMIT 1",
                params![app, title, cutoff],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let content_hash = stable_text_hash(content) as i64;
        if existing.as_ref().map(|(_, _, hash)| *hash) == Some(content_hash) {
            return Ok(None);
        }

        let (conversation_id, previous) = if let Some((id, previous, _)) = existing {
            (id, previous)
        } else {
            self.conn
                .execute(
                    "INSERT INTO activity_conversations
                        (app, title, current_content, current_hash, started_at, updated_at)
                     VALUES (?1, ?2, '', 0, ?3, ?3)",
                    params![app, title, now],
                )
                .map_err(|e| e.to_string())?;
            (self.conn.last_insert_rowid(), String::new())
        };

        let mut new_lines = Vec::new();
        for line in content
            .lines()
            .map(str::trim)
            .filter(|line| line.chars().count() > 1)
        {
            let fingerprint = stable_text_hash(line) as i64;
            let inserted = self
                .conn
                .execute(
                    "INSERT OR IGNORE INTO activity_conversation_messages
                        (conversation_id, fingerprint, text, first_seen_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![conversation_id, fingerprint, line, now],
                )
                .map_err(|e| e.to_string())?;
            if inserted > 0 {
                new_lines.push(line);
            }
        }
        let new_content = new_lines.join("\n");
        let before_chars = previous.chars().count() as i64;
        let new_chars = new_content.chars().count() as i64;
        self.conn
            .execute(
                "INSERT INTO activity_conversation_events
                    (conversation_id, memory_id, captured_at, before_chars, new_chars, content_hash, new_content)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![conversation_id, memory_id, now, before_chars, new_chars, content_hash, new_content],
            )
            .map_err(|e| e.to_string())?;
        let message_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM activity_conversation_messages WHERE conversation_id = ?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "UPDATE activity_conversations SET current_content = ?1, current_hash = ?2,
                    updated_at = ?3, before_chars = ?4, new_chars = ?5, message_count = ?6
                 WHERE id = ?7",
                params![
                    content,
                    content_hash,
                    now,
                    before_chars,
                    new_chars,
                    message_count,
                    conversation_id
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(Some(conversation_id))
    }

    pub fn list_activity_conversations(
        &self,
        limit: i64,
    ) -> Result<Vec<ActivityConversationRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, app, title, started_at, updated_at, before_chars, new_chars, message_count
                 FROM activity_conversations ORDER BY updated_at DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 500)], |row| {
                Ok(ActivityConversationRow {
                    id: row.get(0)?,
                    app: row.get(1)?,
                    title: row.get(2)?,
                    started_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    before_chars: row.get(5)?,
                    new_chars: row.get(6)?,
                    message_count: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn list_conversation_events(
        &self,
        conversation_id: i64,
        limit: i64,
    ) -> Result<Vec<ConversationEventRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, conversation_id, memory_id, captured_at, before_chars, new_chars, new_content
                 FROM activity_conversation_events WHERE conversation_id = ?1
                 ORDER BY captured_at DESC, id DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![conversation_id, limit.clamp(1, 1000)], |row| {
                Ok(ConversationEventRow {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    memory_id: row.get(2)?,
                    captured_at: row.get(3)?,
                    before_chars: row.get(4)?,
                    new_chars: row.get(5)?,
                    new_content: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    // ---- conversations / recordings ----

    pub fn insert_conversation(
        &self,
        title: &str,
        started_at: i64,
        audio_path: Option<&str>,
        kind: &str,
    ) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO conversations (title, started_at, audio_path, kind) VALUES (?1, ?2, ?3, ?4)",
                params![title, started_at, audio_path, kind],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_conversations(&self, limit: i64) -> Result<Vec<ConversationRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, title, transcript, audio_path, started_at, ended_at, duration_ms, kind
                 FROM conversations ORDER BY started_at DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(ConversationRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    transcript: row.get(2)?,
                    audio_path: row.get(3)?,
                    started_at: row.get(4)?,
                    ended_at: row.get(5)?,
                    duration_ms: row.get(6)?,
                    kind: row.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn get_conversation(&self, id: i64) -> Result<Option<ConversationRow>, String> {
        self.conn
            .query_row(
                "SELECT id, title, transcript, audio_path, started_at, ended_at, duration_ms, kind
                 FROM conversations WHERE id = ?1",
                params![id],
                |row| {
                    Ok(ConversationRow {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        transcript: row.get(2)?,
                        audio_path: row.get(3)?,
                        started_at: row.get(4)?,
                        ended_at: row.get(5)?,
                        duration_ms: row.get(6)?,
                        kind: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub fn delete_conversation(&self, id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM conversations WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Crash recovery: a recording whose row never got an `ended_at` was
    /// interrupted (crash, force-quit, power loss). Its WAVs were checkpointed
    /// while recording, so adopt whatever audio exists — the user can then
    /// transcribe it — and drop rows that captured nothing. Call once at
    /// startup, before any new recording can begin.
    pub fn recover_interrupted_recordings(&self, dir: &std::path::Path) -> Result<usize, String> {
        let pending: Vec<(i64, i64)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id, started_at FROM conversations WHERE ended_at IS NULL AND audio_path IS NULL AND transcript IS NULL")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            rows
        };
        let mut recovered = 0;
        for (id, started_at) in pending {
            let mut paths = Vec::new();
            let mut last_modified = started_at;
            for name in [format!("mic-{id}.wav"), format!("system-{id}.wav")] {
                let path = dir.join(name);
                // A bare 44-byte WAV header holds no audio.
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue;
                };
                if meta.len() <= 44 {
                    let _ = std::fs::remove_file(&path);
                    continue;
                }
                if let Ok(modified) = meta.modified() {
                    if let Ok(since_epoch) = modified.duration_since(std::time::UNIX_EPOCH) {
                        last_modified = last_modified.max(since_epoch.as_millis() as i64);
                    }
                }
                paths.push(path.to_string_lossy().to_string());
            }
            if paths.is_empty() {
                self.delete_conversation(id)?;
            } else {
                self.finish_recording(
                    id,
                    Some(&paths.join(",")),
                    last_modified,
                    (last_modified - started_at).max(0),
                )?;
                recovered += 1;
            }
        }
        Ok(recovered)
    }

    pub fn finish_recording(
        &self,
        id: i64,
        audio_path: Option<&str>,
        ended_at: i64,
        duration_ms: i64,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE conversations SET audio_path = ?1, ended_at = ?2, duration_ms = ?3
                 WHERE id = ?4",
                params![audio_path, ended_at, duration_ms, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn set_conversation_transcript(
        &self,
        id: i64,
        transcript: &str,
        fallback_ended_at: i64,
        fallback_duration_ms: i64,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE conversations SET transcript = ?1,
                    ended_at = COALESCE(ended_at, ?2),
                    duration_ms = COALESCE(duration_ms, ?3)
                 WHERE id = ?4",
                params![transcript, fallback_ended_at, fallback_duration_ms, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---- autonomous agent input ledger ----

    pub fn list_unprocessed_agent_memories(&self, limit: i64) -> Result<Vec<MemoryRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                r#"SELECT m.id, m.source, m.app, m.title, m.content, m.created_at,
                          m.is_meeting, m.open_loop
                   FROM memories m
                   LEFT JOIN agent_memory_fingerprints f
                     ON f.memory_id = m.id
                   WHERE f.memory_id IS NULL
                   ORDER BY m.created_at ASC
                   LIMIT ?1"#,
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit.clamp(1, 5000)], memory_from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn mark_agent_memory_processed(
        &self,
        memory_id: i64,
        content_hash: i64,
        run_kind: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO agent_memory_fingerprints
                    (memory_id, content_hash, processed_at, run_kind)
                 VALUES (?1, ?2, ?3, ?4)",
                params![memory_id, content_hash, now_ms(), run_kind],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ---- agent runs ----

    pub fn insert_agent_run(
        &self,
        kind: &str,
        prompt: Option<&str>,
        result: Option<&str>,
    ) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO agent_runs (kind, prompt, result, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![kind, prompt, result, now_ms()],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_agent_runs(&self, limit: i64) -> Result<Vec<AgentRunRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, prompt, result, created_at
                 FROM agent_runs ORDER BY created_at DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(AgentRunRow {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    prompt: row.get(2)?,
                    result: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }
}

fn memory_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRow> {
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
}

fn stable_text_hash(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Start-of-hour epoch ms (UTC). Used as the `hour_bucket` for version
/// freezing. Hour buckets are UTC-anchored so they're stable and comparable.
pub fn hour_bucket_ms(now: i64) -> i64 {
    now - now.rem_euclid(3_600_000)
}

/// Stable, readable identity for a captured activity. Window/conversation
/// titles carry the strongest topic signal; volatile counters, punctuation,
/// browser suffixes, and generic UI words are discarded.
fn capture_activity_key(app: &str, title: &str, content: &str) -> String {
    use std::collections::BTreeSet;

    const NOISE: &[&str] = &[
        "and",
        "app",
        "browser",
        "chat",
        "discord",
        "edge",
        "firefox",
        "gmail",
        "google",
        "mail",
        "message",
        "messages",
        "microsoft",
        "outlook",
        "safari",
        "slack",
        "spark",
        "teams",
        "the",
        "unread",
        "whatsapp",
        "window",
    ];
    let normalize = |value: &str| {
        value
            .to_lowercase()
            .chars()
            .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .filter(|token| token.len() > 1 && !token.chars().all(|ch| ch.is_ascii_digit()))
            .filter(|token| !NOISE.contains(token))
            .take(12)
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
    };
    let mut topic = normalize(title);
    if topic.is_empty() {
        topic = normalize(content.lines().next().unwrap_or(""));
    }
    let app = app
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    let topic = if topic.is_empty() {
        "general".to_string()
    } else {
        topic.into_iter().collect::<Vec<_>>().join("-")
    };
    format!("{app}|{topic}")
}

/// Local calendar date for an epoch-ms timestamp, as `YYYY-MM-DD`.
/// Uses Howard Hinnant's civil-from-days algorithm against the local
/// timezone offset, so no external tz database dependency is required.
pub fn local_date(ms: i64) -> String {
    civil_date(ms + local_utc_offset())
}

/// Best-effort local UTC offset in ms via libc (glibc/BSD `tm_gmtoff`).
fn local_utc_offset() -> i64 {
    #[cfg(target_os = "macos")]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        unsafe {
            let t: libc::time_t = now;
            let mut tm: libc::tm = std::mem::zeroed();
            libc::localtime_r(&t, &mut tm);
            tm.tm_gmtoff as i64 * 1000
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

/// Civil (local) calendar parts from an epoch-ms timestamp (already offset).
fn civil_date(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Day-of-week of an epoch-ms local timestamp, 0 = Sunday .. 6 = Saturday.
pub fn local_weekday(ms: i64) -> u32 {
    // 1970-01-01 was a Thursday (4).
    let days = ms.div_euclid(86_400_000);
    (days.rem_euclid(7) as u32 + 4).rem_euclid(7)
}

/// Howard Hinnant's civil_from_days — converts days since epoch to (y,m,d).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Sunday/date/slot labels derived from a planned reminder timestamp — stored
/// alongside `remind_at` so the UI can show a human-readable schedule without
/// recomputing timezone math.
fn split_schedule_long(remind_at: Option<i64>) -> (Option<String>, Option<String>) {
    let Some(ms) = remind_at else {
        return (None, None);
    };
    let date = local_date(ms);
    let wd = local_weekday(ms);
    let slot = match wd {
        0 => "sunday",
        6 => "saturday",
        _ => "weekday",
    };
    (Some(date), Some(slot.to_string()))
}

/// Local "rewritten for display" derivative: a readable consolidation of a
/// captured moment (Mirrors Minimi's derivative step without an LLM).
fn make_derivative(_title: &str, content: &str) -> String {
    let compact: String = content
        .split_whitespace()
        .take(70)
        .collect::<Vec<_>>()
        .join(" ");
    if compact.chars().count() > 160 {
        compact.chars().take(157).collect::<String>() + "…"
    } else {
        compact
    }
}

#[cfg(test)]
mod tests {
    use super::fts_match_expression;

    #[test]
    fn switched_off_sources_produce_no_follow_ups() {
        let db = open_in_memory_for_test().unwrap();
        let slack = db.insert_memory("capture", Some("Slack"), "t", "please send the report", false, false, None).unwrap();
        db.set_memory_service(slack.id, "slack").unwrap();
        let note = db.insert_memory("note", Some("Memento"), "n", "call Sam", false, true, None).unwrap();
        let meeting = db.insert_memory("meeting", Some("Meeting"), "m", "ship it", true, false, None).unwrap();

        assert!(db.insert_action_item_if_missing(slack.id, "Send the report", false, None).unwrap().is_some());
        db.set_follow_up_source_enabled("slack", false).unwrap();
        db.set_follow_up_source_enabled("meeting", false).unwrap();
        assert!(db.insert_action_item_if_missing(slack.id, "Reply to Dana", false, None).unwrap().is_none());
        assert!(db.insert_action_item_if_missing(meeting.id, "Ship it", false, None).unwrap().is_none());
        // Notes have no switchable source, so they always keep working.
        assert!(db.insert_action_item_if_missing(note.id, "Call Sam", false, None).unwrap().is_some());
        // Switching back on resumes detection.
        db.set_follow_up_source_enabled("slack", true).unwrap();
        assert!(db.insert_action_item_if_missing(slack.id, "Reply to Dana", false, None).unwrap().is_some());
        assert_eq!(db.disabled_follow_up_sources().unwrap(), vec!["meeting".to_string()]);
    }

    #[test]
    fn interrupted_recordings_are_recovered_or_discarded() {
        let db = open_in_memory_for_test().unwrap();
        let dir = std::env::temp_dir().join(format!("memento-recover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let kept = db.insert_conversation("kept", 1_000, None, "meeting").unwrap();
        let empty = db.insert_conversation("empty", 2_000, None, "meeting").unwrap();
        std::fs::write(dir.join(format!("mic-{kept}.wav")), vec![0u8; 4_096]).unwrap();
        std::fs::write(dir.join(format!("mic-{empty}.wav")), vec![0u8; 44]).unwrap();
        assert_eq!(db.recover_interrupted_recordings(&dir).unwrap(), 1);
        let row = db.get_conversation(kept).unwrap().unwrap();
        assert!(row.audio_path.unwrap().ends_with(&format!("mic-{kept}.wav")));
        assert!(row.ended_at.is_some());
        assert!(db.get_conversation(empty).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_survives_punctuation_and_matches_prefixes() {
        let db = open_in_memory_for_test().unwrap();
        db.insert_memory("note", Some("Memento"), "Quarterly planning", "Ship the C++ editor -- refresh by Friday (draft)", false, true, None).unwrap();
        // Characters that are FTS5 syntax must not turn into errors.
        for query in ["-", "\"", "c++", "Friday AND", "plan:", "(draft", "100%", "a_b"] {
            assert!(db.search_text(query, 10).is_ok(), "query {query:?} errored");
        }
        // Type-ahead: a partial last word still finds the memory.
        assert_eq!(db.search_text("quarterly plan", 10).unwrap().len(), 1);
        assert_eq!(db.search_text("c++", 10).unwrap().len(), 1);
        assert!(db.search_text("   ", 10).unwrap().is_empty());
        assert_eq!(fts_match_expression("a \"b\" ").as_deref(), Some("\"a\" \"b\"*"));
        assert_eq!(fts_match_expression("--"), None);
    }

    use super::*;

    #[test]
    fn hour_bucket_truncates_to_epoch_hour() {
        assert_eq!(hour_bucket_ms(3_600_123), 3_600_000);
        assert_eq!(hour_bucket_ms(-3_600_123), -7_200_000);
        assert_eq!(hour_bucket_ms(-3_599_999), -3_600_000);
        assert_eq!(hour_bucket_ms(0), 0);
    }

    #[test]
    fn weekday_is_epoch_accurate() {
        assert_eq!(local_weekday(4 * 86_400_000), 1);
        assert_eq!(local_weekday(0), 4);
    }

    #[test]
    fn local_date_formats() {
        let d = local_date(1_710_501_296_000);
        assert_eq!(d.len(), 10);
    }

    #[test]
    fn activity_identity_ignores_ui_noise_and_separates_topics() {
        assert_eq!(
            capture_activity_key("Slack", "# launch - Slack (3 unread)", "changed"),
            capture_activity_key("Slack", "#launch | Slack (9 unread)", "other")
        );
        assert_ne!(
            capture_activity_key("Slack", "# launch - Slack", "same"),
            capture_activity_key("Slack", "# support - Slack", "same")
        );
    }

    #[test]
    fn current_hour_version_is_consolidated_in_place() -> Result<(), String> {
            let db = Db {
            conn: Connection::open_in_memory().map_err(|e| e.to_string())?,
        };
        db.conn
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| e.to_string())?;
        db.migrate()?;
        let root = db.insert_memory(
            "capture",
            Some("Slack"),
            "# launch",
            "first",
            false,
            false,
            None,
        )?;
        let first_version = db.insert_thread_version(root.id, "[1]", 1, 1)?;
        let second_version = db.insert_thread_version(root.id, "[1,2]", 2, 2)?;
        assert_eq!(first_version, second_version);
        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memory_versions WHERE thread_id = ?1",
                params![root.id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        assert_eq!(count, 1);
        assert_eq!(db.latest_version_hash(root.id)?, Some(2));
        Ok(())
    }

    #[test]
    fn conversation_timeline_records_only_new_fingerprints() -> Result<(), String> {
        let db = Db {
            conn: Connection::open_in_memory().map_err(|e| e.to_string())?,
        };
        db.conn
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| e.to_string())?;
        db.migrate()?;
        let first = db.insert_memory(
            "capture",
            Some("Slack"),
            "project",
            "Alice\nhello",
            false,
            false,
            None,
        )?;
        let conversation_id = db
            .record_conversation_snapshot("Slack", "project", "Alice\nhello", first.id)?
            .ok_or("missing first event")?;
        let second = db.insert_memory(
            "capture",
            Some("Slack"),
            "project",
            "Alice\nhello\nworld",
            false,
            false,
            None,
        )?;
        db.record_conversation_snapshot("Slack", "project", "Alice\nhello\nworld", second.id)?;
        assert!(db
            .record_conversation_snapshot("Slack", "project", "Alice\nhello\nworld", second.id)?
            .is_none());

        let conversations = db.list_activity_conversations(10)?;
        assert_eq!(conversations[0].message_count, 3);
        let events = db.list_conversation_events(conversation_id, 10)?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].new_content, "world");
        assert_eq!(
            events[0].before_chars,
            "Alice\nhello".chars().count() as i64
        );
        assert_eq!(events[0].new_chars, 5);
        Ok(())
    }

    #[test]
    fn recording_paths_survive_database_reopen_and_transcription() -> Result<(), String> {
            let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("poppy-recording-persistence-{unique}.db"));
        let conversation_id;
        {
            let db = Db {
                conn: Connection::open(&path).map_err(|e| e.to_string())?,
            };
            db.conn
                .pragma_update(None, "foreign_keys", "ON")
                .map_err(|e| e.to_string())?;
            db.migrate()?;
            conversation_id = db.insert_conversation("Meeting", 1_000, None, "meeting")?;
            db.finish_recording(
                conversation_id,
                Some("/tmp/mic.wav,/tmp/system.wav"),
                5_000,
                4_000,
            )?;
        }
        {
            let db = Db {
                conn: Connection::open(&path).map_err(|e| e.to_string())?,
            };
            db.conn
                .pragma_update(None, "foreign_keys", "ON")
                .map_err(|e| e.to_string())?;
            db.migrate()?;
            let saved = db
                .get_conversation(conversation_id)?
                .ok_or("conversation missing")?;
            assert_eq!(
                saved.audio_path.as_deref(),
                Some("/tmp/mic.wav,/tmp/system.wav")
            );
            assert_eq!(saved.duration_ms, Some(4_000));
            assert_eq!(saved.kind, "meeting");
            db.set_conversation_transcript(conversation_id, "hello", 9_000, 8_000)?;
            let transcribed = db
                .get_conversation(conversation_id)?
                .ok_or("conversation missing")?;
            assert_eq!(transcribed.audio_path, saved.audio_path);
            assert_eq!(transcribed.duration_ms, Some(4_000));
            assert_eq!(transcribed.transcript.as_deref(), Some("hello"));
        }
        for candidate in [
            path.clone(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ] {
            if candidate.exists() {
                std::fs::remove_file(candidate).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}
