//! Memento — a private, local-first memory layer.
//!
//! The Rust side owns OS integration (Accessibility capture, tray, audio
//! recording, SQLite storage). The webview owns search UX and settings, and
//! calls back into these commands. Memory is mirrored to an Obsidian-compatible
//! Markdown vault (the vault module) — the vault, not vectors, is the memory.

mod adapters;
mod agent;
mod audio;
mod ax;
mod checkin;
mod db;
mod icons;
mod untranscribed;
mod decision;
mod health;
mod lifecycle;
mod ask;
mod followups;
mod localtime;
mod summary;
mod actions;
// --- MODULES: add `mod yours;` lines directly above this marker ---
mod llm;
mod local_tools;
mod mcp_http;
mod meeting_detection;
mod micwatch;
mod monitor;
mod overlay;
mod permissions;
mod reminders;
mod state;
mod transcribe;
mod tray;
mod vault;
mod versions;

use std::sync::Arc;

use crate::state::MonitorState;
use serde::Serialize;
use tauri::menu::{MenuBuilder, MenuItem, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, State};

use tauri_plugin_autostart::ManagerExt;

pub struct RecordingState {
    pub active: parking_lot::Mutex<Option<audio::ActiveRecording>>,
    /// Set while a start is in flight (opening audio devices can block on a
    /// permission prompt). Claimed *before* checking `active`, and `active` is
    /// set before the claim is released, so two starts can never both win —
    /// without holding `active` locked for the duration of the prompt.
    starting: std::sync::atomic::AtomicBool,
}

pub struct StartClaim<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for StartClaim<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

impl Default for RecordingState {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingState {
    pub fn new() -> Self {
        Self {
            active: parking_lot::Mutex::new(None),
            starting: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn begin_start(&self) -> Result<StartClaim<'_>, String> {
        use std::sync::atomic::Ordering;
        if self.starting.swap(true, Ordering::AcqRel) {
            return Err("a recording is already starting".into());
        }
        let claim = StartClaim(&self.starting);
        if self.active.lock().is_some() {
            return Err("a recording is already in progress".into());
        }
        Ok(claim)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    trusted: bool,
    paused: bool,
    recording: bool,
    memory_count: i64,
    action_count: i64,
    db_path: String,
    /// When a timed pause ends (epoch ms); None for no pause or an open-ended one.
    pause_until: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FrontmostInfo {
    pid: Option<i32>,
    app: Option<String>,
    window_title: Option<String>,
}

#[tauri::command]
fn vault_status(state: State<'_, Arc<MonitorState>>) -> Result<vault::VaultStatus, String> {
    vault::status(&state.db())
}

#[tauri::command]
async fn vault_sync_now(
    state: State<'_, Arc<MonitorState>>,
) -> Result<vault::SyncReport, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || vault::sync_shared(&state))
        .await
        .map_err(|e| format!("vault sync: {e}"))?
}

#[tauri::command]
async fn vault_open(
    state: State<'_, Arc<MonitorState>>,
    in_obsidian: bool,
) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || vault::open(&state.db(), in_obsidian))
        .await
        .map_err(|e| format!("vault open: {e}"))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingStart {
    conversation_id: i64,
    started_ms: i64,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Settings {
    transcription_model: String,
    record_mic: bool,
    record_system: bool,
    mix_audio: bool,
    capture_interval_ms: i64,
    onboarding_complete: bool,
    auto_start_meetings: bool,
    auto_stop_meetings: bool,
    max_capture_duration_sec: i64,
    reminder_nudges: bool,
    mcp_enabled: bool,
    mcp_port: u16,
    llm_enabled: bool,
    llm_provider: String,
    llm_base_url: String,
    llm_model: String,
    llm_has_api_key: bool,
    vault_path: String,
}

impl Settings {
    const DEFAULT_TRANSCRIPTION: &'static str = transcribe::MODEL_TINY_EN;
}

fn load_settings(state: &MonitorState) -> Settings {
    let db = state.db();
    let get = |key: &str, default: String| db.get_setting(key).ok().flatten().unwrap_or(default);
    Settings {
        transcription_model: transcribe::canonical_model_id(&get(
            "transcriptionModel",
            Settings::DEFAULT_TRANSCRIPTION.to_string(),
        )),
        record_mic: get("recordMic", "true".into()) == "true",
        record_system: get("recordSystem", "true".into()) == "true",
        mix_audio: get("mixAudio", "true".into()) == "true",
        capture_interval_ms: get("captureIntervalMs", "30000".into())
            .parse()
            .unwrap_or(30_000),
        onboarding_complete: get("onboardingComplete", "false".into()) == "true",
        auto_start_meetings: get("autoStartMeetings", "false".into()) == "true",
        auto_stop_meetings: get("autoStopMeetings", "true".into()) == "true",
        max_capture_duration_sec: get("maxCaptureDurationSec", "0".into())
            .parse()
            .unwrap_or(0),
        reminder_nudges: get("reminderNudges", "true".into()) == "true",
        mcp_enabled: get("mcpEnabled", "false".into()) == "true",
        mcp_port: get("mcpPort", "3961".into()).parse().unwrap_or(3961),
        llm_enabled: get("llmEnabled", "false".into()) == "true",
        llm_provider: get("llmProvider", "ollama".into()),
        llm_base_url: get("llmBaseUrl", "http://127.0.0.1:11434/v1".into()),
        llm_model: get("llmModel", "llama3.2".into()),
        llm_has_api_key: !get("llmApiKey", String::new()).is_empty(),
        vault_path: get("vaultPath", String::new()),
    }
}

#[tauri::command]
fn get_status(
    state: State<'_, Arc<MonitorState>>,
    recordings: State<'_, Arc<RecordingState>>,
) -> Result<Status, String> {
    let db = state.db();
    let memory_count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    let action_count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM action_items WHERE status = 'open'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let db_path = db::app_data_dir()?
        .join("memento.db")
        .to_string_lossy()
        .to_string();
    Ok(Status {
        trusted: ax::is_trusted(),
        paused: state.is_paused(),
        recording: recordings.active.lock().is_some(),
        memory_count,
        action_count,
        db_path,
        pause_until: state.pause_until().filter(|_| state.is_paused()),
    })
}

#[tauri::command]
fn get_settings(state: State<'_, Arc<MonitorState>>) -> Settings {
    load_settings(&state)
}

#[tauri::command]
fn set_setting(
    state: State<'_, Arc<MonitorState>>,
    key: String,
    value: String,
) -> Result<(), String> {
    // The MCP access token is generated by the backend; the webview must not
    // be able to read-modify it through the generic settings channel.
    if key == "mcpToken" {
        return Err("this setting is managed by Memento".into());
    }
    state.db().set_setting(&key, &value)
}

const EXCLUDED_APPS_KEY: &str = "excludedApps";
const BLOCKED_DOMAINS_KEY: &str = "blockedDomains";

/// Apps explicitly opted out of ambient capture. Empty means capture all
/// eligible foreground apps.
#[tauri::command]
fn get_excluded_apps(state: State<'_, Arc<MonitorState>>) -> Vec<adapters::CaptureAppSelection> {
    state.excluded_apps()
}

#[tauri::command]
fn set_excluded_apps(
    state: State<'_, Arc<MonitorState>>,
    apps: Vec<adapters::CaptureAppSelection>,
) -> Result<Vec<adapters::CaptureAppSelection>, String> {
    let mut cleaned: Vec<adapters::CaptureAppSelection> = apps
        .into_iter()
        .filter(|app| !app.name.trim().is_empty() && !app.bundle_id.trim().is_empty())
        .collect();
    cleaned.sort_by(|a, b| a.bundle_id.to_lowercase().cmp(&b.bundle_id.to_lowercase()));
    cleaned.dedup_by(|a, b| a.bundle_id.eq_ignore_ascii_case(&b.bundle_id));
    let json = serde_json::to_string(&cleaned).map_err(|e| e.to_string())?;
    state.db().set_setting(EXCLUDED_APPS_KEY, &json)?;
    state.set_excluded_apps(cleaned.clone());
    Ok(cleaned)
}

#[tauri::command]
fn get_blocked_domains(state: State<'_, Arc<MonitorState>>) -> Vec<String> {
    state.blocked_domains()
}

#[tauri::command]
fn set_blocked_domains(
    state: State<'_, Arc<MonitorState>>,
    domains: Vec<String>,
) -> Result<Vec<String>, String> {
    let cleaned = adapters::effective_blocked_domains(Some(
        &serde_json::to_string(&domains).map_err(|e| e.to_string())?,
    ));
    let json = serde_json::to_string(&cleaned).map_err(|e| e.to_string())?;
    state.db().set_setting(BLOCKED_DOMAINS_KEY, &json)?;
    state.set_blocked_domains(cleaned.clone());
    Ok(cleaned)
}

/// Scans /Applications recursively — disk I/O that must not run on the main
/// thread (sync commands do), so the work is moved to the blocking pool.
#[tauri::command]
async fn list_installed_apps() -> Result<Vec<adapters::CaptureAppSelection>, String> {
    tauri::async_runtime::spawn_blocking(scan_installed_apps)
        .await
        .map_err(|e| format!("scan apps: {e}"))
}

fn scan_installed_apps() -> Vec<adapters::CaptureAppSelection> {
    fn scan(
        directory: &std::path::Path,
        depth: usize,
        apps: &mut Vec<adapters::CaptureAppSelection>,
    ) {
        if depth > 3 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
            {
                let Ok(info) = plist::Value::from_file(path.join("Contents/Info.plist")) else {
                    continue;
                };
                let Some(dict) = info.as_dictionary() else {
                    continue;
                };
                if dict.get("LSUIElement").and_then(plist::Value::as_boolean) == Some(true)
                    || dict
                        .get("LSBackgroundOnly")
                        .and_then(plist::Value::as_boolean)
                        == Some(true)
                {
                    continue;
                }
                let Some(bundle_id) = dict
                    .get("CFBundleIdentifier")
                    .and_then(plist::Value::as_string)
                else {
                    continue;
                };
                let name = dict
                    .get("CFBundleDisplayName")
                    .and_then(plist::Value::as_string)
                    .or_else(|| dict.get("CFBundleName").and_then(plist::Value::as_string))
                    .map(str::to_string)
                    .or_else(|| {
                        path.file_stem()
                            .map(|name| name.to_string_lossy().to_string())
                    });
                if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
                    icons::remember(bundle_id, path.clone());
                    apps.push(adapters::CaptureAppSelection {
                        name,
                        bundle_id: bundle_id.to_string(),
                    });
                }
            } else if path.is_dir() {
                scan(&path, depth + 1, apps);
            }
        }
    }

    let mut apps = Vec::new();
    for root in [
        std::path::PathBuf::from("/Applications"),
        std::path::PathBuf::from("/System/Applications"),
        dirs::home_dir().unwrap_or_default().join("Applications"),
    ] {
        scan(&root, 0, &mut apps);
    }
    apps.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    apps.dedup_by(|left, right| left.bundle_id.eq_ignore_ascii_case(&right.bundle_id));
    apps
}

/// Icons (as data URLs) for the given installed-app bundle ids; ids with no
/// installed app or no renderable icon are omitted.
#[tauri::command]
async fn get_app_icons(
    bundle_ids: Vec<String>,
) -> Result<std::collections::HashMap<String, String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if icons::known_apps() == 0 {
            scan_installed_apps();
        }
        icons::data_urls(&bundle_ids)
    })
    .await
    .map_err(|e| format!("icons: {e}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FollowUpSources {
    sources: Vec<adapters::FollowUpSource>,
    disabled: Vec<String>,
}

/// Which apps Memento finds follow-ups (action items) from. The catalog is the
/// set of sources Memento can actually read; `disabled` is the user's opt-outs.
#[tauri::command]
fn get_follow_up_sources(state: State<'_, Arc<MonitorState>>) -> Result<FollowUpSources, String> {
    Ok(FollowUpSources {
        sources: adapters::FOLLOW_UP_SOURCES.to_vec(),
        disabled: state.db().disabled_follow_up_sources()?,
    })
}

#[tauri::command]
fn set_follow_up_source_enabled(
    state: State<'_, Arc<MonitorState>>,
    id: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    if !adapters::FOLLOW_UP_SOURCES.iter().any(|source| source.id == id) {
        return Err(format!("unknown follow-up source: {id}"));
    }
    state.db().set_follow_up_source_enabled(&id, enabled)
}

#[tauri::command]
fn set_paused(
    state: State<'_, Arc<MonitorState>>,
    app: AppHandle,
    paused: bool,
    duration_min: Option<i64>,
) -> Result<(), String> {
    state.set_paused(paused);
    // A manual pause is indefinite; only the tray's "Pause for N minutes"
    // sets a deadline (auto-resumed by the monitor loop).
    state.set_pause_until(match (paused, duration_min) {
        (true, Some(minutes)) if minutes > 0 => {
            Some(db::now_ms() + minutes * 60_000)
        }
        _ => None,
    });
    tray::update_pause_label(&app, paused);
    let _ = app.emit("pause-changed", paused);
    Ok(())
}

#[tauri::command]
async fn get_permissions() -> permissions::Permissions {
    permissions::get().await
}

#[tauri::command]
async fn request_permission(app: AppHandle, kind: String) -> Result<(), String> {
    permissions::request(app, &kind).await
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn send_notification(app: AppHandle, title: String, body: String) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_frontmost() -> Result<FrontmostInfo, String> {
    tauri::async_runtime::spawn_blocking(frontmost_info)
        .await
        .map_err(|e| format!("frontmost: {e}"))
}

fn frontmost_info() -> FrontmostInfo {
    let identity = match ax::frontmost_app() {
        Some(v) => v,
        None => {
            return FrontmostInfo {
                pid: None,
                app: None,
                window_title: None,
            }
        }
    };
    let pid = identity.pid;
    let name = identity.name;
    let window_title = ax::window_title(pid);
    FrontmostInfo {
        pid: Some(pid),
        app: Some(name),
        window_title,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FocusedCapture {
    pid: Option<i32>,
    app: Option<String>,
    window_title: Option<String>,
    focused_text: Option<String>,
    selected_text: Option<String>,
}

#[tauri::command]
async fn get_focused() -> Result<FocusedCapture, String> {
    tauri::async_runtime::spawn_blocking(focused_capture)
        .await
        .map_err(|e| format!("focused: {e}"))
}

fn focused_capture() -> FocusedCapture {
    let identity = match ax::frontmost_app() {
        Some(v) => v,
        None => {
            return FocusedCapture {
                pid: None,
                app: None,
                window_title: None,
                focused_text: None,
                selected_text: None,
            }
        }
    };
    let pid = identity.pid;
    let name = identity.name;
    FocusedCapture {
        pid: Some(pid),
        app: Some(name),
        window_title: ax::window_title(pid),
        focused_text: ax::focused_element_text(pid),
        selected_text: ax::selected_text(pid),
    }
}

// ---- desktop interaction (agent-facing AX actions) ----

#[tauri::command]
fn desktop_activate(app_name: String) -> bool {
    ax::activate_app(&app_name)
}

#[tauri::command]
fn desktop_find(pid: i32, roles: Vec<String>, label: String) -> Result<Option<u64>, String> {
    let role_refs: Vec<&str> = roles.iter().map(|s| s.as_str()).collect();
    Ok(ax::find_element_by_label(pid, &role_refs, &label))
}

#[tauri::command]
fn desktop_set_text(handle: u64, text: String) -> bool {
    ax::set_element_value(handle, &text)
}

#[tauri::command]
fn desktop_press(handle: u64) -> bool {
    ax::press_element(handle)
}

#[tauri::command]
fn desktop_focus(handle: u64) -> bool {
    ax::focus_element(handle)
}

#[tauri::command]
fn desktop_get_bounds(handle: u64) -> Result<Option<(f64, f64, f64, f64)>, String> {
    Ok(ax::element_bounds(handle))
}

#[tauri::command]
async fn desktop_move_cursor(
    x: f64,
    y: f64,
    steps: Option<u32>,
    step_delay_ms: Option<u64>,
    pre_click_delay_ms: Option<u64>,
    hold_ms: Option<u64>,
    do_click: Option<bool>,
) -> Result<(), String> {
    // The glide sleeps between steps; keep that off the main thread.
    tauri::async_runtime::spawn_blocking(move || {
        ax::move_cursor_and_click(
            x,
            y,
            steps.unwrap_or(24),
            step_delay_ms.unwrap_or(4),
            pre_click_delay_ms.unwrap_or(0),
            hold_ms.unwrap_or(50),
            do_click.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| format!("move cursor: {e}"))?
}

// ---- memories ----

#[tauri::command]
fn list_memories(
    state: State<'_, Arc<MonitorState>>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<db::MemoryRow>, String> {
    state
        .db()
        .list_memories(limit.unwrap_or(100), offset.unwrap_or(0))
}

#[tauri::command]
fn get_memory(
    state: State<'_, Arc<MonitorState>>,
    id: i64,
) -> Result<Option<db::MemoryRow>, String> {
    state.db().get_memory(id)
}

#[tauri::command]
fn list_memory_enrichments(
    state: State<'_, Arc<MonitorState>>,
    ids: Vec<i64>,
) -> Result<Vec<db::MemoryEnrichmentRow>, String> {
    state.db().list_memory_enrichments(&ids)
}

#[tauri::command]
async fn enrich_memories_now(
    state: State<'_, Arc<MonitorState>>,
    app: AppHandle,
) -> Result<usize, String> {
    // Keep manual runs responsive. Repeated clicks continue through a backlog,
    // while the single-flight guard prevents overlap with the background pass.
    // Model calls are slow: run them on the blocking pool, off the UI thread
    // and without holding the database lock.
    let state = state.inner().clone();
    let enriched = tauri::async_runtime::spawn_blocking(move || llm::enrich_pending(&state, 5))
        .await
        .map_err(|e| format!("enrich: {e}"))??;
    if enriched > 0 {
        let _ = app.emit("memories-enriched", ());
    }
    Ok(enriched)
}

#[tauri::command]
fn delete_memory(state: State<'_, Arc<MonitorState>>, id: i64) -> Result<(), String> {
    state.db().delete_memory(id)
}

#[tauri::command]
fn insert_note(
    state: State<'_, Arc<MonitorState>>,
    app: AppHandle,
    text: String,
) -> Result<db::MemoryRow, String> {
    let db = state.db();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("note is empty".into());
    }
    let title = first_line(trimmed, 90);
    let memory = db.insert_memory("note", Some("Memento"), &title, trimmed, false, true, None)?;
    let _ = app.emit("memory-new", &memory);
    if agent::extract_for_memory(&db, &memory).unwrap_or(0) > 0 {
        let _ = app.emit("agent-updated", ());
    }
    Ok(memory)
}

#[tauri::command]
fn search_memories(
    state: State<'_, Arc<MonitorState>>,
    query: String,
) -> Result<Vec<db::MemoryRow>, String> {
    state.db().search_text(query.trim(), 50)
}

// ---- threads ----

#[tauri::command]
fn list_threads(
    state: State<'_, Arc<MonitorState>>,
    limit: Option<i64>,
) -> Result<Vec<db::ThreadRow>, String> {
    state.db().list_threads(limit.unwrap_or(100))
}

#[tauri::command]
fn get_thread(
    state: State<'_, Arc<MonitorState>>,
    thread_id: i64,
) -> Result<Option<(db::ThreadRow, Vec<db::MemoryRow>)>, String> {
    state.db().get_thread(thread_id)
}

// ---- local MCP tools ----

// Minimi exposes search_memory / list_active_threads / get_latest_context /
// meeting_memory to external LLMs over MCP. We run the same tools locally and
// return structured JSON; an agent/bridge (e.g. an MCP stdio server) can call
// this `mcp_call` command.

#[tauri::command]
fn mcp_call(
    state: State<'_, Arc<MonitorState>>,
    tool: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let db = state.db();
    crate::local_tools::call_tool(&db, &tool, &args)
}

// ---- overlay: "ask your day" chat ----

#[tauri::command]
fn overlay_expand(app: AppHandle) {
    overlay::expand(&app);
}

#[tauri::command]
fn overlay_collapse(app: AppHandle) {
    overlay::collapse(&app);
}

#[tauri::command]
fn overlay_hide(app: AppHandle) {
    overlay::hide(&app);
}

/// Source of truth for pill/chat/hidden so the webview can sync its view even
/// when it missed an event (e.g. it was still loading when the view changed).
#[tauri::command]
fn overlay_view() -> &'static str {
    overlay::view()
}

/// Answer a question grounded in the current screen and local memory. The
/// heavy LLM call runs on the blocking pool; the DB lock is only held while
/// gathering context.
#[tauri::command]
async fn chat_ask(
    state: State<'_, Arc<MonitorState>>,
    question: String,
) -> Result<String, String> {
    let state = state.inner().clone();
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("the question is empty".into());
    }
    tauri::async_runtime::spawn_blocking(move || ask::answer(&state, &question))
        .await
        .map_err(|e| format!("chat: {e}"))?
}

// ---- visits ----

#[tauri::command]
fn list_visits(
    state: State<'_, Arc<MonitorState>>,
    limit: Option<i64>,
) -> Result<Vec<db::AppVisitRow>, String> {
    state.db().list_visits(limit.unwrap_or(100))
}

// ---- action items ----

#[tauri::command]
fn list_action_items(
    state: State<'_, Arc<MonitorState>>,
    status: String,
) -> Result<Vec<db::ActionItemRow>, String> {
    state.db().list_action_items(&status)
}

#[tauri::command]
fn set_action_item_status(
    state: State<'_, Arc<MonitorState>>,
    id: i64,
    status: String,
) -> Result<(), String> {
    state.db().set_action_item_status(id, &status)
}

// ---- conversations / recording ----

#[tauri::command]
fn list_conversations(
    state: State<'_, Arc<MonitorState>>,
    limit: Option<i64>,
) -> Result<Vec<db::ConversationRow>, String> {
    state.db().list_conversations(limit.unwrap_or(50))
}

#[tauri::command]
fn get_conversation(
    state: State<'_, Arc<MonitorState>>,
    id: i64,
) -> Result<Option<db::ConversationRow>, String> {
    state.db().get_conversation(id)
}

#[tauri::command]
fn list_activity_conversations(
    state: State<'_, Arc<MonitorState>>,
    limit: Option<i64>,
) -> Result<Vec<db::ActivityConversationRow>, String> {
    state.db().list_activity_conversations(limit.unwrap_or(100))
}

#[tauri::command]
fn get_activity_conversation_events(
    state: State<'_, Arc<MonitorState>>,
    conversation_id: i64,
    limit: Option<i64>,
) -> Result<Vec<db::ConversationEventRow>, String> {
    state
        .db()
        .list_conversation_events(conversation_id, limit.unwrap_or(200))
}

/// Delete a meeting/voice-note recording: the conversation row plus its
/// captured audio files on disk.
#[tauri::command]
fn delete_conversation(state: State<'_, Arc<MonitorState>>, id: i64) -> Result<(), String> {
    let db = state.db();
    let row = db.get_conversation(id)?;
    db.delete_conversation(id)?;
    // Best-effort cleanup of the captured audio files.
    if let Some(paths) = row.and_then(|c| c.audio_path) {
        for path in paths.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()) {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

/// Shared by the `start_recording` command and the menu-bar item. Opening
/// audio devices can block on a macOS permission prompt, so call this off the
/// main thread.
pub(crate) fn start_recording_blocking(
    app: &AppHandle,
    state: &Arc<MonitorState>,
    recordings: &Arc<RecordingState>,
    mode: &str,
    kind: &str,
) -> Result<RecordingStart, String> {
    let parsed =
        audio::RecordingMode::parse(mode).ok_or_else(|| "invalid recording mode".to_string())?;
    let _claim = recordings.begin_start()?;
    // Explicit user starts stay strict: if the requested source fails to
    // open, surface the error so the UI can explain the missing permission.
    let active = audio::start(app, state, parsed, kind, false)?;
    let start = RecordingStart {
        conversation_id: active.conversation_id,
        started_ms: active.started_ms,
    };
    *recordings.active.lock() = Some(active);
    Ok(start)
}

/// Stops the active recording, announces it, and starts transcription in the
/// background. WAV finalizing can take seconds: call off the main thread.
pub(crate) fn stop_recording_blocking(
    app: &AppHandle,
    state: &Arc<MonitorState>,
    recordings: &Arc<RecordingState>,
) -> Result<audio::RecordingResult, String> {
    let active = recordings
        .active
        .lock()
        .take()
        .ok_or_else(|| "no recording in progress".to_string())?;
    let result = audio::stop(state, active)?;
    let _ = app.emit("recording-stopped", &result);
    transcribe::spawn_for(app.clone(), state.clone(), result.conversation_id);
    Ok(result)
}

#[tauri::command]
async fn start_recording(
    app: AppHandle,
    state: State<'_, Arc<MonitorState>>,
    recordings: State<'_, Arc<RecordingState>>,
    mode: String,
    kind: Option<String>,
) -> Result<RecordingStart, String> {
    let state = state.inner().clone();
    let recordings = recordings.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        start_recording_blocking(&app, &state, &recordings, &mode, kind.as_deref().unwrap_or("meeting"))
    })
    .await
    .map_err(|e| format!("start recording: {e}"))?
}

#[tauri::command]
async fn stop_recording(
    recordings: State<'_, Arc<RecordingState>>,
    state: State<'_, Arc<MonitorState>>,
    app: AppHandle,
) -> Result<audio::RecordingResult, String> {
    let recordings = recordings.inner().clone();
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || stop_recording_blocking(&app, &state, &recordings))
        .await
        .map_err(|e| format!("stop recording: {e}"))?
}

#[tauri::command]
fn recording_status(
    recordings: State<'_, Arc<RecordingState>>,
) -> Result<Option<RecordingStart>, String> {
    let guard = recordings.active.lock();
    Ok(guard.as_ref().map(|active| RecordingStart {
        conversation_id: active.conversation_id,
        started_ms: active.started_ms,
    }))
}

/// Collapse a transcript into a titled memory. The persisted conversation kind
/// selects the meeting pipeline or the lighter voice-note pipeline:
/// a meeting becomes a summarized meeting memory with action items and
/// title-collision handling; a voice note becomes a lighter, non-meeting
/// memory.
pub(crate) fn persist_transcript(
    db: &db::Db,
    app: &AppHandle,
    conversation_id: i64,
    transcript: &str,
) -> Result<db::MemoryRow, String> {
    let trimmed = transcript.trim();
    if trimmed.is_empty() {
        return Err("transcript is empty".into());
    }
    let ended = db::now_ms();
    let conversation = db
        .get_conversation(conversation_id)?
        .ok_or_else(|| format!("conversation {conversation_id} was not found"))?;
    let is_meeting = conversation.kind != "voice-note";
    let started = conversation.started_at;
    db.set_conversation_transcript(conversation_id, trimmed, ended, ended - started)?;

    let base_title = first_line(trimmed, 90);
    let (source, app_name, title, meeting) = if is_meeting {
        // extract-meeting → titled, deduplicated meeting memory.
        let title = db.unique_meeting_title(&base_title)?;
        ("meeting".to_string(), "Meeting".to_string(), title, true)
    } else {
        // extract-voice-note → lighter, not a meeting memory.
        let base = if base_title.is_empty() {
            "Voice note".to_string()
        } else {
            base_title
        };
        (
            "voice-note".to_string(),
            "Voice note".to_string(),
            base,
            false,
        )
    };

    let memory = db.insert_memory(
        &source,
        Some(app_name.as_str()),
        &title,
        trimmed,
        meeting,
        false,
        None,
    )?;
    // Voice notes also keep their transcript attached in conversations; a
    // meeting's inserted memory keeps the full text for the memory layer.
    let _ = app.emit("memory-new", &memory);
    // What the user (or others in the meeting) promised becomes follow-ups now,
    // not at the next hourly pass.
    if agent::extract_for_memory(db, &memory).unwrap_or(0) > 0 {
        let _ = app.emit("agent-updated", ());
    }
    Ok(memory)
}

/// (Re)transcribe a stored recording on demand — the retry path for failed or
/// previously untranscribed recordings. Runs in the background; the UI follows
/// the `transcription-*` events.
#[tauri::command]
async fn transcribe_conversation(
    state: State<'_, Arc<MonitorState>>,
    app: AppHandle,
    conversation_id: i64,
) -> Result<(), String> {
    // Validate up front so obvious errors (missing conversation) surface
    // synchronously instead of only via the failure event.
    let exists = state
        .db()
        .get_conversation(conversation_id)?
        .ok_or_else(|| format!("conversation {conversation_id} was not found"))?
        .transcript
        .map(|t| t.trim().is_empty())
        .unwrap_or(true);
    if !exists {
        return Err("this recording already has a transcript".into());
    }
    transcribe::spawn_for(app, state.inner().clone(), conversation_id);
    Ok(())
}

/// Explicit model download for the onboarding "Local AI models" step. Emits
/// `model-download-progress` events while downloading; returns immediately if
/// the model is already on disk.
#[tauri::command]
async fn prepare_transcription_model(
    app: AppHandle,
    model: Option<String>,
) -> Result<(), String> {
    let model_id = transcribe::canonical_model_id(&model.unwrap_or_default());
    tauri::async_runtime::spawn_blocking(move || {
        transcribe::prepare_model(&app, &model_id).map(|_| ())
    })
    .await
    .map_err(|e| format!("model download: {e}"))?
}

/// Status of a transcription model (downloaded? how big?).
#[tauri::command]
fn transcription_model_status(model: Option<String>) -> transcribe::ModelStatus {
    transcribe::model_status(&transcribe::canonical_model_id(&model.unwrap_or_default()))
}

/// Delete a transcription model from disk (Settings model management).
#[tauri::command]
fn delete_transcription_model(model: Option<String>) -> Result<(), String> {
    transcribe::delete_model(&transcribe::canonical_model_id(&model.unwrap_or_default()))
}

/// Payload for the `recording-auto-stopped` event (micwatch).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingAutoStopped {
    result: audio::RecordingResult,
    reason: String,
}

// ---- agent surface (pluggable; local heuristics for now) ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentResult {
    id: i64,
    result: String,
}

#[tauri::command]
async fn agent_run(
    state: State<'_, Arc<MonitorState>>,
    kind: String,
    prompt: String,
) -> Result<AgentResult, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || agent_run_blocking(&state, kind, prompt))
        .await
        .map_err(|e| format!("agent: {e}"))?
}

fn agent_run_blocking(
    state: &MonitorState,
    kind: String,
    prompt: String,
) -> Result<AgentResult, String> {
    let db = state.db();
    if kind == "reconcile" {
        let summary = agent::run_cycle(&db, "manual", 5_000)?;
        return Ok(AgentResult {
            id: db.conn.last_insert_rowid(),
            result: serde_json::to_string_pretty(&summary).map_err(|error| error.to_string())?,
        });
    }
    let result = match kind.as_str() {
        "open-loops" => {
            let items = db.list_action_items("open")?;
            if items.is_empty() {
                "No open loops right now — you're all caught up.".to_string()
            } else {
                let mut out = format!("You have {} open loop(s):\n", items.len());
                for (i, item) in items.iter().take(10).enumerate() {
                    out.push_str(&format!("{}. {}\n", i + 1, item.content));
                }
                out
            }
        }
        "recent-context" => {
            let memories = db.list_memories(5, 0)?;
            if memories.is_empty() {
                "No captured memories yet.".to_string()
            } else {
                let mut out = "Recent context:\n".to_string();
                for memory in memories {
                    out.push_str(&format!(
                        "- {}: {}\n",
                        memory.title,
                        memory.content.chars().take(120).collect::<String>()
                    ));
                }
                out
            }
        }
        _ => {
            return Err("unknown agent kind".into());
        }
    };
    let id = db.insert_agent_run(&kind, Some(&prompt), Some(&result))?;
    Ok(AgentResult { id, result })
}

#[tauri::command]
fn list_agent_runs(
    state: State<'_, Arc<MonitorState>>,
    limit: Option<i64>,
) -> Result<Vec<db::AgentRunRow>, String> {
    state.db().list_agent_runs(limit.unwrap_or(50))
}

// ---- memory versions (eras of state per thread) ----

#[tauri::command]
fn get_thread_versions(
    state: State<'_, Arc<MonitorState>>,
    thread_id: i64,
) -> Result<Vec<db::MemoryVersionRow>, String> {
    state.db().list_thread_versions(thread_id, 100)
}

// ---- action items (rich model) ----

#[tauri::command]
fn update_action_item(
    state: State<'_, Arc<MonitorState>>,
    id: i64,
    urgent: Option<bool>,
    remind_at: Option<Option<i64>>,
    unread: Option<bool>,
    sort_order: Option<i64>,
) -> Result<(), String> {
    state
        .db()
        .update_action_item(id, urgent, remind_at, unread, sort_order)
}

// ---- reminders / nudges ----

#[tauri::command]
fn get_due_reminders(
    state: State<'_, Arc<MonitorState>>,
) -> Result<reminders::DueReminders, String> {
    let db = state.db();
    reminders::due_reminders(&db)
}

#[tauri::command]
fn get_dnd_state() -> bool {
    reminders::dnd_state()
}

#[tauri::command]
fn mark_reminders_shown(state: State<'_, Arc<MonitorState>>, ids: Vec<i64>) -> Result<(), String> {
    state.db().mark_reminders_shown(&ids)
}

// ---- local MCP server ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct McpState {
    running: bool,
    port: u16,
    url: String,
    bridge_path: String,
}

#[tauri::command]
fn mcp_state(
    mcp: State<'_, Arc<mcp_http::McpServerState>>,
    monitor: State<'_, Arc<MonitorState>>,
) -> McpState {
    let port = mcp.port();
    let token = ensure_mcp_token(&monitor.db()).unwrap_or_default();
    McpState {
        running: mcp.is_running(),
        port,
        url: format!("http://127.0.0.1:{port}/sse?token={token}"),
        bridge_path: crate::mcp_bridge_path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    }
}

/// The stdio bridge script bundled into the binary and materialized next to
/// the local database.
fn mcp_bridge_path() -> Result<std::path::PathBuf, String> {
    let dir = db::app_data_dir()?;
    Ok(dir.join("mcpStdio.cjs"))
}

/// Write the bundled bridge script to disk (idempotent) so Claude Desktop can
/// be pointed at it without shipping source files.
fn ensure_mcp_bridge() -> Result<std::path::PathBuf, String> {
    let path = mcp_bridge_path()?;
    let bundled = include_str!("../../mcpStdio.cjs");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(bundled) {
        std::fs::write(&path, bundled).map_err(|e| format!("write mcp bridge: {e}"))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("secure mcp bridge: {e}"))?;
    }
    Ok(path)
}

fn ensure_mcp_token(db: &db::Db) -> Result<String, String> {
    if let Some(token) = db.get_setting("mcpToken")? {
        if token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(token);
        }
    }
    use std::io::Read;
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|e| format!("generate MCP access token: {e}"))?;
    let token = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    db.set_setting("mcpToken", &token)?;
    Ok(token)
}

/// A GUI-launched app gets a minimal PATH, so `node` from Homebrew/nvm/volta
/// is invisible to `Command::new("node")`. Look in the usual places.
fn find_node() -> Option<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    dirs.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"]
            .into_iter()
            .map(std::path::PathBuf::from),
    );
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".volta/bin"));
        dirs.push(home.join(".local/bin"));
        if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            let mut versions: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
            versions.sort();
            dirs.extend(versions.into_iter().rev().map(|version| version.join("bin")));
        }
    }
    dirs.into_iter()
        .map(|dir| dir.join("node"))
        .find(|candidate| candidate.is_file())
}

/// add-to-claude-desktop: write the poppy entry into Claude Desktop's config
/// and report the result. Claude Desktop must be restarted by the user.
#[tauri::command]
async fn mcp_install_claude_desktop(
    monitor: State<'_, Arc<MonitorState>>,
) -> Result<String, String> {
    let bridge = ensure_mcp_bridge()?;
    let token = ensure_mcp_token(&monitor.db())?;
    tauri::async_runtime::spawn_blocking(move || {
        let node = find_node().ok_or_else(|| {
            "Node.js was not found. Install it (https://nodejs.org) and try again.".to_string()
        })?;
        let output = std::process::Command::new(node)
            .arg(&bridge)
            .arg("--add-to-claude-desktop")
            .env("POPPY_MCP_TOKEN", token)
            .output()
            .map_err(|e| format!("could not run node: {e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    })
    .await
    .map_err(|e| format!("install: {e}"))?
}

#[tauri::command]
fn mcp_set_enabled(
    mcp: State<'_, Arc<mcp_http::McpServerState>>,
    monitor: State<'_, Arc<MonitorState>>,
    enabled: bool,
    port: Option<u16>,
) -> Result<McpState, String> {
    let port = port.unwrap_or_else(|| mcp.port());
    if enabled {
        let token = ensure_mcp_token(&monitor.db())?;
        // Binds synchronously (restarting on a changed port), so a port
        // conflict is returned to the UI instead of being silently saved.
        mcp.start(monitor.inner().clone(), port, token)?;
    } else {
        mcp.stop()?;
    }
    {
        let db = monitor.db();
        db.set_setting("mcpEnabled", &enabled.to_string())?;
        db.set_setting("mcpPort", &port.to_string())?;
    }
    Ok(mcp_state(mcp, monitor))
}

fn first_line(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim().to_string();
    if line.chars().count() > max {
        line.chars().take(max).collect::<String>() + "…"
    } else {
        line
    }
}

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// Bring the main window to the front and open the "New memory" dialog.
fn open_new_memory(app: &AppHandle) {
    show_main_window(app);
    let _ = app.emit("new-memory", ());
}

pub fn run() {
    let db = match db::open() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("could not open database: {e}");
            std::process::exit(1);
        }
    };

    // A recording that was mid-flight when the app last died is adopted (or
    // discarded) before anything can start a new one.
    match db::recordings_dir().and_then(|dir| db.recover_interrupted_recordings(&dir)) {
        Ok(0) => {}
        Ok(count) => log::info!("recovered {count} interrupted recording(s)"),
        Err(error) => eprintln!("could not recover interrupted recordings: {error}"),
    }

    let monitor_state = Arc::new(MonitorState::new(db));
    let recording_state = Arc::new(RecordingState::new());
    let mcp_server = Arc::new(mcp_http::McpServerState::new());

    tauri::Builder::default()
        // Must be registered first so a second launch exits before any other
        // plugin initializes.
        .plugin(
            tauri_plugin_single_instance::Builder::default()
                .callback(|app, _args, _cwd| show_main_window(app))
                .build(),
        )
        .plugin(
            tauri_plugin_log::Builder::new()
                // `target()` appends to the plugin's default Stdout + LogDir
                // targets; on a case-insensitive filesystem the default
                // "Memento.log" and ours are one file, so every line was
                // written twice.
                .clear_targets()
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("memento".into()),
                    },
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ))
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_macos_permissions::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        // Persist only the normal application window. Transient overlays must
        // always start from tauri.conf.json (hidden) and never be resurrected.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_filter(|label| label == "main")
                .skip_initial_state("quickcapture")
                .build(),
        )
        .plugin(tauri_plugin_deep_link::init())
        .manage(monitor_state.clone())
        .manage(recording_state.clone())
        .manage(mcp_server.clone())
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_settings,
            set_setting,
            get_excluded_apps,
            set_excluded_apps,
            get_blocked_domains,
            set_blocked_domains,
            list_installed_apps,
            get_app_icons,
            get_follow_up_sources,
            set_follow_up_source_enabled,
            set_paused,
            get_permissions,
            request_permission,
            get_frontmost,
            get_focused,
            desktop_activate,
            desktop_find,
            desktop_set_text,
            desktop_press,
            desktop_focus,
            desktop_get_bounds,
            desktop_move_cursor,
            list_memories,
            get_memory,
            list_memory_enrichments,
            enrich_memories_now,
            delete_memory,
            insert_note,
            search_memories,
            list_threads,
            get_thread,
            mcp_call,
            overlay_expand,
            overlay_collapse,
            overlay_hide,
            overlay_view,
            chat_ask,
            list_visits,
            list_action_items,
            set_action_item_status,
            list_conversations,
            get_conversation,
            list_activity_conversations,
            get_activity_conversation_events,
            delete_conversation,
            start_recording,
            stop_recording,
            recording_status,
            transcribe_conversation,
            prepare_transcription_model,
            transcription_model_status,
            delete_transcription_model,
            agent_run,
            list_agent_runs,
            get_thread_versions,
            update_action_item,
            get_due_reminders,
            get_dnd_state,
            mark_reminders_shown,
            mcp_state,
            mcp_set_enabled,
            mcp_install_claude_desktop,
            get_autostart,
            set_autostart,
            send_notification,
            vault_status,
            vault_sync_now,
            vault_open,
            untranscribed::list_untranscribed_recordings,
            health::get_capture_health,
            health::get_app_capture_intervals,
            health::set_app_capture_interval,
            lifecycle::get_capture_summary,
            lifecycle::preview_delete,
            lifecycle::delete_memories,
            lifecycle::list_memories_filtered,
            summary::get_day_summary,
            summary::get_week_summary,
            followups::edit_action_item_text,
            followups::snooze_action_item,
            followups::get_snooze_presets,
            tray::tray_panel_hide,
            tray::open_main_view,
            // --- HANDLERS: add your commands directly above this marker ---
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            let app_menu = SubmenuBuilder::new(&handle, "Memento")
                .about(None)
                .separator()
                .item(&MenuItem::with_id(
                    &handle,
                    "settings",
                    "Settings…",
                    true,
                    Some("Cmd+,"),
                )?)
                .separator()
                .services()
                .separator()
                .hide()
                .hide_others()
                .separator()
                .quit()
                .build()?;
            let file_menu = SubmenuBuilder::new(&handle, "File")
                .item(&MenuItem::with_id(
                    &handle,
                    "new-note",
                    "New Note",
                    true,
                    Some("Cmd+N"),
                )?)
                .separator()
                .close_window()
                .build()?;
            let edit_menu = SubmenuBuilder::new(&handle, "Edit")
                .undo()
                .redo()
                .separator()
                .cut()
                .copy()
                .paste()
                .select_all()
                .build()?;
            let view_menu = SubmenuBuilder::new(&handle, "View")
                .item(&MenuItem::with_id(
                    &handle,
                    "today",
                    "Today",
                    true,
                    Some("Cmd+1"),
                )?)
                .item(&MenuItem::with_id(
                    &handle,
                    "memory",
                    "Memory",
                    true,
                    Some("Cmd+2"),
                )?)
                .item(&MenuItem::with_id(
                    &handle,
                    "actions",
                    "Follow-ups",
                    true,
                    Some("Cmd+3"),
                )?)
                .item(&MenuItem::with_id(
                    &handle,
                    "meetings",
                    "Meetings",
                    true,
                    Some("Cmd+4"),
                )?)
                .separator()
                .item(&MenuItem::with_id(
                    &handle,
                    "find",
                    "Find in Memory",
                    true,
                    Some("Cmd+F"),
                )?)
                .separator()
                .fullscreen()
                .build()?;
            let window_menu = SubmenuBuilder::new(&handle, "Window")
                .minimize()
                .close_window()
                .build()?;
            let help_menu = SubmenuBuilder::new(&handle, "Help").build()?;
            let menu = MenuBuilder::new(&handle)
                .items(&[
                    &app_menu,
                    &file_menu,
                    &edit_menu,
                    &view_menu,
                    &window_menu,
                    &help_menu,
                ])
                .build()?;
            app.set_menu(menu)?;
            app.on_menu_event(|app, event| {
                let _ = app.emit("menu-command", event.id().as_ref());
            });

            // Hide instead of quitting when the window is closed; keep running in the tray.
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win.hide();
                    }
                });
            }

            tray::setup(&handle)?;
            overlay::setup(&handle)?;
            monitor::spawn(handle.clone(), monitor_state.clone());
            llm::spawn(handle.clone(), monitor_state.clone());
            agent::spawn(handle.clone(), monitor_state.clone());
            versions::spawn(handle.clone(), monitor_state.clone());
            vault::spawn(monitor_state.clone());
            reminders::spawn_nudge_worker(handle.clone(), monitor_state.clone());
            micwatch::spawn(
                handle.clone(),
                monitor_state.clone(),
                recording_state.clone(),
            );
            checkin::spawn(handle.clone(), monitor_state.clone());

            // Start the local MCP server if it was previously enabled.
            let settings = load_settings(&monitor_state);
            if settings.mcp_enabled {
                if let Ok(token) = ensure_mcp_token(&monitor_state.db()) {
                    let _ = mcp_server.start(monitor_state.clone(), settings.mcp_port, token);
                }
            }

            // Global shortcut: open the "New memory" dialog in the main window.
            use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
            let core = handle.global_shortcut();
            if let Err(error) = core.on_shortcut("Cmd+Shift+M", move |app, _sc, event| {
                if event.state == ShortcutState::Pressed {
                    open_new_memory(app);
                }
            }) {
                log::warn!("could not register Cmd+Shift+M: {error}");
            }

            // Global shortcut: toggle the "ask your day" overlay chat.
            if let Err(error) = core.on_shortcut("Cmd+Shift+L", move |app, _sc, event| {
                if event.state == ShortcutState::Pressed {
                    overlay::toggle(app);
                }
            }) {
                log::warn!("could not register Cmd+Shift+L: {error}");
            }

            // Deep link (memento://...) — e.g. an external LLM opens the app
            // with a query; show the main window and let the renderer handle it.
            use tauri_plugin_deep_link::DeepLinkExt;
            let link_handle = handle.clone();
            let _ = handle.deep_link().on_open_url(move |event| {
                let urls: Vec<String> = event.urls().iter().map(|url| url.to_string()).collect();
                log::info!("deep link opened: {urls:?}");
                show_main_window(&link_handle);
                let _ = link_handle.emit("deep-link", urls);
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // The window hides on close and the app lives in the menu bar, so
            // clicking the Dock icon must bring it back.
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                show_main_window(app);
            }
        });
}
