//! Event-assisted background watcher: observes AX focus/window changes, uses a
//! slow reconciliation interval, and captures visible text as memories — only
//! for apps/websites that have a dedicated adapter, never for sensitive or
//! system-UI surfaces. Every outcome is a `decision::Reason`, recorded in the
//! shared health state. The per-tick logic is `tick`, driven through the
//! `Source` trait so it can be tested without a real desktop.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter};

#[cfg(target_os = "macos")]
use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};

use crate::adapters::{self, AdapterKind, NormalizedCapture};
use crate::ax::{self, AppIdentity};
use crate::db::MemoryRow;
use crate::decision::{self, Reason};
use crate::health::{DecisionUpdate, FrontApp};
use crate::state::MonitorState;

const POLL_MS: u64 = 2000;
/// AX change notifications wake the run loop early, and chatty apps fire them
/// in bursts. Never tick more often than this (each tick does AX IPC).
const MIN_TICK_MS: u64 = 400;
/// 30s between visible-text captures by default: frequent enough to follow
/// real work, slow enough that window noise doesn't flood memory.
const DEFAULT_CAPTURE_INTERVAL_MS: i64 = 30_000;
const MIN_CAPTURE_INTERVAL_MS: i64 = 2_000;
const MAX_CAPTURE_INTERVAL_MS: i64 = 15 * 60_000;
/// Setting holding `{ "<bundleId>": ms }` overrides of the global interval.
pub const INTERVAL_BY_APP_KEY: &str = "captureIntervalByApp";
/// A capture needs at least this much text to be worth remembering.
const MIN_TEXT_CHARS: usize = 80;
/// Above this Jaccard similarity, a capture is treated as the same screen
/// with cosmetic changes (cursor, timestamps, scroll) and skipped.
const NEAR_DUPLICATE_THRESHOLD: f32 = 0.85;
const MEETING_APPS: &[&str] = &[
    "zoom",
    "google meet",
    "microsoft teams",
    "teams",
    "facetime",
    "slack huddle",
    "webex",
    "goto meeting",
    "discord",
];

pub fn clamp_interval(ms: i64) -> i64 {
    ms.clamp(MIN_CAPTURE_INTERVAL_MS, MAX_CAPTURE_INTERVAL_MS)
}

/// Parse the per-app override map, dropping malformed entries and clamping
/// values into the allowed range.
pub fn parse_interval_overrides(raw: Option<&str>) -> BTreeMap<String, i64> {
    raw.and_then(|value| serde_json::from_str::<BTreeMap<String, i64>>(value).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|(bundle, _)| !bundle.trim().is_empty())
        .map(|(bundle, ms)| (bundle, clamp_interval(ms)))
        .collect()
}

/// The override map after setting (or, with `None`, clearing) one app.
pub fn with_interval_override(
    raw: Option<&str>,
    bundle_id: &str,
    ms: Option<i64>,
) -> Result<BTreeMap<String, i64>, String> {
    let bundle = bundle_id.trim();
    if bundle.is_empty() {
        return Err("Missing app identifier".into());
    }
    let mut map = parse_interval_overrides(raw);
    map.retain(|key, _| !key.eq_ignore_ascii_case(bundle));
    if let Some(ms) = ms {
        map.insert(bundle.to_string(), clamp_interval(ms));
    }
    Ok(map)
}

/// Capture interval for the frontmost app: its override, else the global
/// setting, else the default.
pub fn effective_interval_ms(
    global_raw: Option<&str>,
    overrides: &BTreeMap<String, i64>,
    bundle_id: Option<&str>,
) -> i64 {
    let by_app = bundle_id.and_then(|bundle| {
        overrides
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(bundle))
            .map(|(_, ms)| *ms)
    });
    clamp_interval(by_app.unwrap_or_else(|| {
        global_raw
            .and_then(|raw| raw.parse::<i64>().ok())
            .unwrap_or(DEFAULT_CAPTURE_INTERVAL_MS)
    }))
}

/// Word-set Jaccard similarity between the previous capture and the candidate.
/// Screens that only ticked a clock or scrolled a line are near-duplicates.
fn is_near_duplicate(previous: Option<&str>, candidate: &str) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    let previous_tokens = meaningful_tokens(previous);
    let candidate_tokens = meaningful_tokens(candidate);
    if previous_tokens.is_empty() || candidate_tokens.is_empty() {
        return false;
    }
    let shared = previous_tokens.intersection(&candidate_tokens).count();
    let union = previous_tokens.len() + candidate_tokens.len() - shared;
    union > 0 && (shared as f32 / union as f32) >= NEAR_DUPLICATE_THRESHOLD
}

fn meaningful_tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(str::to_string)
        .collect()
}

/// Stable 64-bit FNV-1a hash of content, used to avoid storing duplicate
/// versions of the same screen/text.
fn content_hash(content: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in content.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// How long to hold back so ticks stay at least MIN_TICK_MS apart.
fn debounce_wait(since_last_tick: Duration) -> Option<Duration> {
    let min = Duration::from_millis(MIN_TICK_MS);
    (since_last_tick < min).then(|| min - since_last_tick)
}

#[derive(Default)]
pub struct Watch {
    current_pid: Option<i32>,
    current_window: Option<String>,
    visit_id: Option<i64>,
    last_text: Option<String>,
    last_capture: i64,
}

/// Everything a tick reads from the desktop, so tests can fake it.
pub trait Source {
    fn trusted(&self) -> bool;
    fn own_pid(&self) -> i32;
    fn frontmost(&self) -> Option<AppIdentity>;
    fn window_title(&self, pid: i32) -> Option<String>;
    fn document_url(&self, pid: i32, app: &str, bundle_id: Option<&str>) -> Option<String>;
    fn extract(
        &self,
        kind: AdapterKind,
        pid: i32,
        app: &str,
        bundle_id: Option<&str>,
        window_title: Option<&str>,
    ) -> Option<NormalizedCapture>;
}

struct AxSource;

impl Source for AxSource {
    fn trusted(&self) -> bool {
        ax::is_trusted()
    }
    fn own_pid(&self) -> i32 {
        std::process::id() as i32
    }
    fn frontmost(&self) -> Option<AppIdentity> {
        ax::frontmost_app()
    }
    fn window_title(&self, pid: i32) -> Option<String> {
        ax::window_title(pid)
    }
    fn document_url(&self, pid: i32, app: &str, bundle_id: Option<&str>) -> Option<String> {
        adapters::read_document_url(pid, app, bundle_id)
    }
    fn extract(
        &self,
        kind: AdapterKind,
        pid: i32,
        app: &str,
        bundle_id: Option<&str>,
        window_title: Option<&str>,
    ) -> Option<NormalizedCapture> {
        adapters::extract(kind, pid, app, bundle_id, window_title)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PauseStep {
    Running,
    Paused,
    /// A timed pause just ended; capture is running again.
    Resumed,
}

/// Apply pause / timed-pause state. While paused any open visit is closed.
fn pause_step(state: &MonitorState, watch: &mut Watch, now: i64) -> PauseStep {
    if !state.is_paused() {
        return PauseStep::Running;
    }
    // A timed pause ("pause for 10 minutes") auto-resumes.
    if let Some(until) = state.pause_until() {
        if now >= until {
            state.set_pause_until(None);
            state.set_paused(false);
            return PauseStep::Resumed;
        }
    }
    close_watch_visit(state, watch);
    PauseStep::Paused
}

#[derive(Default)]
pub struct TickResult {
    /// A visit opened or closed.
    pub visit_changed: bool,
    pub memory: Option<MemoryRow>,
    /// The summarized health state or decision changed.
    pub health_changed: bool,
}

pub fn spawn(app: AppHandle, state: Arc<MonitorState>) {
    std::thread::spawn(move || {
        let mut watch = Watch::default();
        let source = AxSource;
        #[cfg(target_os = "macos")]
        let mut observer: Option<(i32, ax::AxChangeObserver)> = None;
        let mut last_tick = std::time::Instant::now() - Duration::from_secs(60);
        while state.running.load(Ordering::Relaxed) {
            #[cfg(target_os = "macos")]
            {
                let mode = unsafe { kCFRunLoopDefaultMode };
                CFRunLoop::run_in_mode(mode, POLL_MS as f64 / 1000.0, true);
            }
            #[cfg(not(target_os = "macos"))]
            std::thread::sleep(Duration::from_millis(POLL_MS));
            if !state.running.load(Ordering::Relaxed) {
                break;
            }
            // Debounce notification bursts: the run loop returns on every AX
            // event, so without this the loop spins at event rate.
            if let Some(wait) = debounce_wait(last_tick.elapsed()) {
                std::thread::sleep(wait);
            }
            last_tick = std::time::Instant::now();
            let now = now_ms();
            match pause_step(&state, &mut watch, now) {
                PauseStep::Resumed => {
                    let _ = app.emit("pause-changed", false);
                    crate::tray::update_pause_label(&app, false);
                    continue;
                }
                PauseStep::Paused => {
                    if state
                        .health
                        .observe(source.trusted(), true, None, DecisionUpdate::Set(Reason::Paused), now)
                    {
                        let _ = app.emit("capture-health", state.health.snapshot());
                    }
                    continue;
                }
                PauseStep::Running => {}
            }
            let result = tick(&source, &state, &mut watch, now);
            if result.visit_changed {
                let _ = app.emit("visit", ());
            }
            if let Some(memory) = &result.memory {
                let _ = app.emit("memory-new", memory);
                // Requests found on the captured screen become follow-ups right
                // away (the source tag was set during the tick).
                let created = crate::agent::extract_for_memory(&state.db(), memory).unwrap_or(0);
                if created > 0 {
                    let _ = app.emit("agent-updated", ());
                }
            }
            if result.health_changed {
                let _ = app.emit("capture-health", state.health.snapshot());
            }
            #[cfg(target_os = "macos")]
            {
                let pid = watch.current_pid;
                if observer.as_ref().map(|(observed, _)| *observed) != pid {
                    observer =
                        pid.and_then(|pid| ax::observe_changes(pid).map(|watcher| (pid, watcher)));
                }
            }
        }
    });
}

/// One pass of the capture loop for the frontmost app. Never reads content
/// for an app that fails an app-level gate, and never holds the DB lock
/// across a `Source` call.
pub fn tick(
    src: &dyn Source,
    state: &Arc<MonitorState>,
    watch: &mut Watch,
    now: i64,
) -> TickResult {
    let mut result = TickResult::default();
    let trusted = src.trusted();
    let Some(identity) = src.frontmost() else {
        result.visit_changed = close_watch_visit(state, watch);
        result.health_changed = state
            .health
            .observe(trusted, false, None, DecisionUpdate::Clear, now);
        return result;
    };
    let pid = identity.pid;
    let name = identity.name.clone();
    let bundle = identity.bundle_id.clone();
    let front = Some(FrontApp {
        name: name.clone(),
        bundle_id: bundle.clone(),
    });

    let excluded = state.excluded_apps();
    if let Some(reason) = decision::app_gate(pid, src.own_pid(), &name, bundle.as_deref(), &excluded)
    {
        result.visit_changed = close_watch_visit(state, watch);
        result.health_changed =
            state
                .health
                .observe(trusted, false, front, DecisionUpdate::Set(reason), now);
        return result;
    }

    // Accessibility reads can take hundreds of ms; never do them while holding
    // the shared database lock or every UI command stalls behind capture.
    let window = src.window_title(pid);
    let mut changed = false;

    // App switch.
    if watch.current_pid != Some(pid) {
        let previous = watch.visit_id.take();
        let opened = {
            let db = state.db();
            if let Some(visit_id) = previous {
                let _ = db.close_visit(visit_id);
            }
            db.open_visit(&name, window.as_deref())
        };
        let visit_id = match opened {
            Ok(id) => id,
            Err(e) => {
                log::warn!("visit: {e}");
                return result;
            }
        };
        watch.visit_id = Some(visit_id);
        watch.current_pid = Some(pid);
        watch.current_window = window.clone();
        watch.last_text = None;
        changed = true;
        result.visit_changed = true;
    }

    // Window title change.
    if window != watch.current_window {
        watch.current_window = window.clone();
        if let (Some(visit_id), Some(title)) = (watch.visit_id, &window) {
            let _ = state.db().update_visit_title(visit_id, title);
        }
        watch.last_text = None;
        changed = true;
    }

    let capture_interval = {
        let db = state.db();
        let global = db.get_setting("captureIntervalMs").ok().flatten();
        let overrides = parse_interval_overrides(
            db.get_setting(INTERVAL_BY_APP_KEY).ok().flatten().as_deref(),
        );
        effective_interval_ms(global.as_deref(), &overrides, bundle.as_deref())
    };
    let periodic = now - watch.last_capture >= capture_interval;
    if !changed && !periodic {
        result.health_changed = state
            .health
            .observe(trusted, false, front, DecisionUpdate::Keep, now);
        return result;
    }

    let outcome = capture_step(src, state, watch, &identity, trusted, now);
    let (reason, memory) = outcome;
    result.memory = memory;
    result.health_changed =
        state
            .health
            .observe(trusted, false, front, DecisionUpdate::Set(reason), now);
    result
}

/// Decide and (when allowed) read, dedupe and store one capture.
fn capture_step(
    src: &dyn Source,
    state: &Arc<MonitorState>,
    watch: &mut Watch,
    identity: &AppIdentity,
    trusted: bool,
    now: i64,
) -> (Reason, Option<MemoryRow>) {
    let pid = identity.pid;
    let name = identity.name.as_str();
    let bundle = identity.bundle_id.as_deref();
    if !trusted {
        watch.last_capture = now;
        return (Reason::NeedsPermission, None);
    }

    let document_url = src.document_url(pid, name, bundle);
    let kind = match adapters::plan(
        name,
        bundle,
        document_url.as_deref(),
        watch.current_window.as_deref(),
        &state.blocked_domains(),
    ) {
        Ok(kind) => kind,
        Err(reason) => {
            watch.last_capture = now;
            return (reason, None);
        }
    };
    // Each adapter produces normalized { title, content }; None means nothing
    // readable.
    let Some(capture) = src.extract(kind, pid, name, bundle, watch.current_window.as_deref())
    else {
        watch.last_capture = now;
        return (Reason::NothingToRead, None);
    };
    let trimmed = capture.content;
    let is_conversation = capture.is_conversation;
    let service = capture.service;
    if trimmed.chars().count() < MIN_TEXT_CHARS {
        watch.last_capture = now;
        return (Reason::TooLittleText, None);
    }

    // Same screen with cosmetic differences (clock, cursor, scroll)?
    if is_near_duplicate(watch.last_text.as_deref(), &trimmed)
        || watch.last_text.as_deref() == Some(trimmed.as_str())
    {
        watch.last_capture = now;
        return (Reason::NearDuplicate, None);
    }

    watch.last_text = Some(trimmed.clone());
    watch.last_capture = now;

    // Content-hash dedup: skip if this exact screen was already indexed for
    // this app, so we don't pile up duplicate versions of unchanged content.
    let hash = content_hash(&trimmed);
    let meeting = looks_like_meeting(name, watch.current_window.as_deref());
    let title = capture.title;
    let memory = {
        let db = state.db();
        if db.capture_hash_exists(name, hash).unwrap_or(false) {
            return (Reason::AlreadySaved, None);
        }
        let memory = match db.insert_memory(
            "capture",
            Some(name),
            &title,
            &trimmed,
            meeting,
            false,
            Some(hash),
        ) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("capture: {e}");
                return (Reason::NothingToRead, None);
            }
        };
        if let Some(service) = service {
            let _ = db.set_memory_service(memory.id, service);
        }
        if is_conversation {
            let _ = db.record_conversation_snapshot(name, &title, &trimmed, memory.id);
        }
        memory
    };
    (Reason::Captured, Some(memory))
}

/// Close the open visit and forget the current app. True if a visit closed.
fn close_watch_visit(state: &MonitorState, watch: &mut Watch) -> bool {
    let closed = if let Some(visit_id) = watch.visit_id.take() {
        let _ = state.db().close_visit(visit_id);
        true
    } else {
        false
    };
    watch.current_pid = None;
    watch.current_window = None;
    watch.last_text = None;
    closed
}

fn looks_like_meeting(app_name: &str, window_title: Option<&str>) -> bool {
    let app_lower = app_name.to_lowercase();
    if MEETING_APPS.iter().any(|m| app_lower.contains(m)) {
        return true;
    }
    // Match whole words so "Syncing…" or "Recall" are not meetings.
    window_title.is_some_and(|title| {
        title
            .to_lowercase()
            .split(|ch: char| !ch.is_alphanumeric())
            .any(|word| {
                matches!(
                    word,
                    "meeting" | "call" | "huddle" | "standup" | "sync" | "retro" | "interview"
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::CaptureAppSelection;
    use std::cell::{Cell, RefCell};

    const LONG_A: &str = "Quarterly planning notes: ship the editor refresh by Friday, owners are Priya and Sam, review the rollout checklist with the whole team on Monday.";
    const LONG_B: &str = "Sprint retro: what went well was the launch, follow ups include sharing the metrics dashboard and scheduling the customer interviews next week.";

    struct Fake {
        trusted: Cell<bool>,
        front: RefCell<Option<AppIdentity>>,
        title: RefCell<Option<String>>,
        url: RefCell<Option<String>>,
        content: RefCell<Option<String>>,
        title_calls: Cell<usize>,
        url_calls: Cell<usize>,
        extract_calls: Cell<usize>,
    }

    impl Fake {
        fn new() -> Self {
            Fake {
                trusted: Cell::new(true),
                front: RefCell::new(None),
                title: RefCell::new(None),
                url: RefCell::new(None),
                content: RefCell::new(None),
                title_calls: Cell::new(0),
                url_calls: Cell::new(0),
                extract_calls: Cell::new(0),
            }
        }
        fn open(&self, pid: i32, name: &str, bundle: &str, title: &str, content: &str) {
            *self.front.borrow_mut() = Some(AppIdentity {
                pid,
                name: name.into(),
                bundle_id: Some(bundle.into()),
            });
            *self.title.borrow_mut() = Some(title.into());
            *self.content.borrow_mut() = Some(content.into());
        }
        fn reads(&self) -> usize {
            self.title_calls.get() + self.url_calls.get() + self.extract_calls.get()
        }
    }

    impl Source for Fake {
        fn trusted(&self) -> bool {
            self.trusted.get()
        }
        fn own_pid(&self) -> i32 {
            999
        }
        fn frontmost(&self) -> Option<AppIdentity> {
            self.front.borrow().clone()
        }
        fn window_title(&self, _pid: i32) -> Option<String> {
            self.title_calls.set(self.title_calls.get() + 1);
            self.title.borrow().clone()
        }
        fn document_url(&self, _pid: i32, _app: &str, _bundle: Option<&str>) -> Option<String> {
            self.url_calls.set(self.url_calls.get() + 1);
            self.url.borrow().clone()
        }
        fn extract(
            &self,
            _kind: AdapterKind,
            _pid: i32,
            _app: &str,
            _bundle: Option<&str>,
            window_title: Option<&str>,
        ) -> Option<NormalizedCapture> {
            self.extract_calls.set(self.extract_calls.get() + 1);
            self.content.borrow().clone().map(|content| NormalizedCapture {
                title: window_title.unwrap_or("untitled").to_string(),
                content,
                is_conversation: false,
                service: Some("slack"),
            })
        }
    }

    fn state() -> Arc<MonitorState> {
        Arc::new(MonitorState::new(crate::db::open_in_memory_for_test().unwrap()))
    }

    const SLACK: &str = "com.tinyspeck.slackmacgap";

    fn last_reason(state: &MonitorState) -> Reason {
        state.health.snapshot().last_decision.unwrap().reason
    }

    #[test]
    fn captures_then_skips_until_something_changes() {
        let (fake, state, mut watch) = (Fake::new(), state(), Watch::default());
        fake.open(1, "Slack", SLACK, "#general", LONG_A);
        let first = tick(&fake, &state, &mut watch, 1_000_000);
        assert!(first.memory.is_some() && first.visit_changed && first.health_changed);
        assert_eq!(last_reason(&state), Reason::Captured);

        // Nothing changed and the interval hasn't elapsed: no AX content reads.
        let extracts = fake.extract_calls.get();
        let again = tick(&fake, &state, &mut watch, 1_005_000);
        assert!(again.memory.is_none() && !again.health_changed);
        assert_eq!(fake.extract_calls.get(), extracts);

        // Interval elapsed with identical text: near-duplicate, not saved twice.
        let later = tick(&fake, &state, &mut watch, 1_040_000);
        assert!(later.memory.is_none());
        assert_eq!(last_reason(&state), Reason::NearDuplicate);
        assert_eq!(state.db().list_memories(10, 0).unwrap().len(), 1);

        // A window title change triggers an immediate read of new content.
        *fake.title.borrow_mut() = Some("#random".into());
        *fake.content.borrow_mut() = Some(LONG_B.into());
        let changed = tick(&fake, &state, &mut watch, 1_041_000);
        assert!(changed.memory.is_some());
        assert_eq!(state.db().list_memories(10, 0).unwrap().len(), 2);
    }

    #[test]
    fn already_saved_screens_are_not_stored_again() {
        let (fake, state, mut watch) = (Fake::new(), state(), Watch::default());
        fake.open(1, "Slack", SLACK, "#general", LONG_A);
        assert!(tick(&fake, &state, &mut watch, 1_000_000).memory.is_some());
        // Leave and come back: last_text is forgotten, but the hash is known.
        fake.open(2, "Mail", "com.apple.mail", "Inbox", LONG_B);
        assert!(tick(&fake, &state, &mut watch, 1_001_000).memory.is_some());
        fake.open(1, "Slack", SLACK, "#general", LONG_A);
        let back = tick(&fake, &state, &mut watch, 1_002_000);
        assert!(back.memory.is_none());
        assert_eq!(last_reason(&state), Reason::AlreadySaved);
    }

    #[test]
    fn app_switch_closes_and_opens_visits() {
        let (fake, state, mut watch) = (Fake::new(), state(), Watch::default());
        fake.open(1, "Slack", SLACK, "#general", LONG_A);
        tick(&fake, &state, &mut watch, 1_000_000);
        fake.open(2, "Mail", "com.apple.mail", "Inbox", LONG_B);
        let switched = tick(&fake, &state, &mut watch, 1_001_000);
        assert!(switched.visit_changed);
        let visits = state.db().list_visits(10).unwrap();
        assert_eq!(visits.len(), 2);
        let slack = visits.iter().find(|v| v.app == "Slack").unwrap();
        let mail = visits.iter().find(|v| v.app == "Mail").unwrap();
        assert!(slack.ended_at.is_some() && mail.ended_at.is_none());
        // Going to a skipped app closes the open visit and opens none.
        fake.open(3, "Finder", "com.apple.finder", "Desktop", "");
        let finder = tick(&fake, &state, &mut watch, 1_002_000);
        assert!(finder.visit_changed);
        assert!(state.db().list_visits(10).unwrap().iter().all(|v| v.ended_at.is_some()));
        assert_eq!(state.db().list_visits(10).unwrap().len(), 2);
    }

    #[test]
    fn excluded_and_denied_apps_never_read_content() {
        let (fake, state, mut watch) = (Fake::new(), state(), Watch::default());
        state.set_excluded_apps(vec![CaptureAppSelection {
            name: "Slack".into(),
            bundle_id: SLACK.into(),
        }]);
        fake.open(1, "Slack", SLACK, "#general", LONG_A);
        tick(&fake, &state, &mut watch, 1_000_000);
        assert_eq!(last_reason(&state), Reason::ExcludedByYou);
        for (name, bundle, reason) in [
            ("Terminal", "com.apple.Terminal", Reason::DeniedByDefault),
            ("Finder", "com.apple.finder", Reason::SystemApp),
            ("1Password", "com.1password", Reason::SensitiveApp),
            ("Memento", "com.memento.app", Reason::OwnApp),
        ] {
            fake.open(if reason == Reason::OwnApp { 999 } else { 4 }, name, bundle, "w", LONG_A);
            tick(&fake, &state, &mut watch, 2_000_000);
            assert_eq!(last_reason(&state), reason);
        }
        assert_eq!(fake.reads(), 0);
        assert!(state.db().list_memories(10, 0).unwrap().is_empty());
        assert!(state.db().list_visits(10).unwrap().is_empty());
    }

    #[test]
    fn unsupported_pages_and_missing_urls_are_explained_without_extracting() {
        let (fake, state, mut watch) = (Fake::new(), state(), Watch::default());
        fake.open(1, "Google Chrome", "com.google.Chrome", "News", LONG_A);
        *fake.url.borrow_mut() = Some("https://example.com/".into());
        tick(&fake, &state, &mut watch, 1_000_000);
        assert_eq!(last_reason(&state), Reason::NotSupported);
        *fake.url.borrow_mut() = None;
        *fake.title.borrow_mut() = Some("News 2".into());
        tick(&fake, &state, &mut watch, 1_001_000);
        assert_eq!(last_reason(&state), Reason::PageUnreadable);
        assert_eq!(state.health.snapshot().state, "unreadable");
        assert_eq!(fake.extract_calls.get(), 0);
    }

    #[test]
    fn missing_permission_reads_no_content_and_nothing_readable_is_reported() {
        let (fake, state, mut watch) = (Fake::new(), state(), Watch::default());
        fake.open(1, "Slack", SLACK, "#general", LONG_A);
        fake.trusted.set(false);
        tick(&fake, &state, &mut watch, 1_000_000);
        assert_eq!(state.health.snapshot().state, "needs-permission");
        assert_eq!(fake.extract_calls.get(), 0);

        fake.trusted.set(true);
        *fake.content.borrow_mut() = None;
        *fake.title.borrow_mut() = Some("#new".into());
        tick(&fake, &state, &mut watch, 1_001_000);
        assert_eq!(last_reason(&state), Reason::NothingToRead);
        *fake.content.borrow_mut() = Some("short".into());
        *fake.title.borrow_mut() = Some("#newer".into());
        tick(&fake, &state, &mut watch, 1_002_000);
        assert_eq!(last_reason(&state), Reason::TooLittleText);
    }

    #[test]
    fn pause_and_timed_pause_auto_resume() {
        let (fake, state, mut watch) = (Fake::new(), state(), Watch::default());
        fake.open(1, "Slack", SLACK, "#general", LONG_A);
        tick(&fake, &state, &mut watch, 1_000_000);
        assert_eq!(pause_step(&state, &mut watch, 1_000_100), PauseStep::Running);

        state.set_paused(true);
        state.set_pause_until(Some(1_600_000));
        assert_eq!(pause_step(&state, &mut watch, 1_100_000), PauseStep::Paused);
        // Pausing closes the open visit.
        assert!(state.db().list_visits(10).unwrap().iter().all(|v| v.ended_at.is_some()));
        assert_eq!(pause_step(&state, &mut watch, 1_599_999), PauseStep::Paused);
        assert_eq!(pause_step(&state, &mut watch, 1_600_000), PauseStep::Resumed);
        assert!(!state.is_paused() && state.pause_until().is_none());

        // An indefinite pause never auto-resumes.
        state.set_paused(true);
        assert_eq!(pause_step(&state, &mut watch, i64::MAX), PauseStep::Paused);
    }

    #[test]
    fn debounce_holds_back_bursts_only() {
        assert_eq!(debounce_wait(Duration::from_millis(100)), Some(Duration::from_millis(300)));
        assert_eq!(debounce_wait(Duration::from_millis(400)), None);
        assert_eq!(debounce_wait(Duration::from_secs(5)), None);
    }

    #[test]
    fn per_app_interval_overrides_the_global_one() {
        let overrides = parse_interval_overrides(Some(
            r#"{"com.tinyspeck.slackmacgap": 5000, "bad": 1, "": 9000}"#,
        ));
        assert_eq!(overrides.len(), 2);
        assert_eq!(effective_interval_ms(Some("60000"), &overrides, Some("COM.tinyspeck.slackmacgap")), 5000);
        // Out-of-range stored values are clamped to the 2s floor.
        assert_eq!(effective_interval_ms(None, &overrides, Some("bad")), 2000);
        assert_eq!(effective_interval_ms(Some("60000"), &overrides, Some("com.apple.mail")), 60_000);
        assert_eq!(effective_interval_ms(None, &overrides, None), 30_000);
        assert_eq!(effective_interval_ms(Some("1"), &overrides, None), 2000);
        assert_eq!(effective_interval_ms(Some("99999999"), &overrides, None), 900_000);

        let set = with_interval_override(Some(r#"{"A.b": 5000}"#), "a.B", Some(120_000)).unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(set["a.B"], 120_000);
        assert!(with_interval_override(Some(r#"{"a.b": 5000}"#), "A.B", None).unwrap().is_empty());
        assert!(with_interval_override(None, "  ", Some(5000)).is_err());
    }

    #[test]
    fn interval_override_changes_when_a_periodic_capture_is_due() {
        let (fake, state, mut watch) = (Fake::new(), state(), Watch::default());
        fake.open(1, "Slack", SLACK, "#general", LONG_A);
        tick(&fake, &state, &mut watch, 1_000_000);
        let extracts = fake.extract_calls.get();
        // 10s later: not due at the 30s default...
        tick(&fake, &state, &mut watch, 1_010_000);
        assert_eq!(fake.extract_calls.get(), extracts);
        // ...but due once Slack is set to 5s.
        state
            .db()
            .set_setting(INTERVAL_BY_APP_KEY, &format!(r#"{{"{SLACK}": 5000}}"#))
            .unwrap();
        tick(&fake, &state, &mut watch, 1_020_000);
        assert_eq!(fake.extract_calls.get(), extracts + 1);
    }

    #[test]
    fn cosmetic_screen_changes_are_near_duplicates() {
        let base = "Quarterly planning\nShip the editor refresh by Friday\nOwners: Priya and Sam";
        // Identical except a ticking clock line and punctuation noise.
        let ticked = "Quarterly planning\nShip the editor refresh by Friday\nOwners: Priya and Sam\n12:04";
        assert!(is_near_duplicate(Some(base), ticked));
        // A genuinely new screen is not.
        assert!(!is_near_duplicate(
            Some(base),
            "Sprint retro notes\nWhat went well: launch went smooth\nFollow-ups: share metrics"
        ));
        assert!(!is_near_duplicate(None, base));
    }
}
