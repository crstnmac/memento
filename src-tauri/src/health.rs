//! Live capture health: a cheap in-memory picture of what the monitor is
//! doing, updated on every tick (no DB, no AX) and read by Settings.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::State;

use crate::decision::{Decision, Reason};
use crate::monitor;
use crate::state::MonitorState;

const RECENT_CAP: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontApp {
    pub name: String,
    pub bundle_id: Option<String>,
}

/// What a tick learned about the decision for the frontmost app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionUpdate {
    /// Nothing new this tick (e.g. the screen hadn't changed).
    Keep,
    /// No app is in front.
    Clear,
    Set(Reason),
}

#[derive(Clone, Debug)]
struct Recent {
    app_name: String,
    bundle_id: Option<String>,
    reason: Reason,
    at: i64,
    count: u32,
}

struct Inner {
    trusted: bool,
    paused: bool,
    frontmost: Option<FrontApp>,
    last: Option<(Reason, i64)>,
    last_capture_at: Option<i64>,
    consecutive_unreadable: u32,
    recent: VecDeque<Recent>,
    /// Summary last announced, so events fire only on a real change.
    announced: Option<(&'static str, Option<String>, Option<Reason>)>,
}

pub struct HealthState {
    inner: Mutex<Inner>,
}

impl Default for HealthState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentDecision {
    pub app_name: String,
    pub bundle_id: Option<String>,
    #[serde(flatten)]
    pub decision: Decision,
    pub at: i64,
    pub count: u32,
    /// The user could change how often this app is read.
    pub tunable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureHealth {
    /// "capturing" | "paused" | "needs-permission" | "idle" | "not-supported"
    /// | "unreadable" | "excluded"
    pub state: &'static str,
    pub headline: String,
    pub trusted: bool,
    pub paused: bool,
    pub frontmost: Option<FrontApp>,
    pub last_decision: Option<Decision>,
    pub last_decision_at: Option<i64>,
    pub last_capture_at: Option<i64>,
    pub consecutive_unreadable: u32,
    pub recent: Vec<RecentDecision>,
}

fn app_key(app: &FrontApp) -> String {
    app.bundle_id
        .as_deref()
        .map(str::to_lowercase)
        .unwrap_or_else(|| app.name.to_lowercase())
}

pub fn coarse_state(
    trusted: bool,
    paused: bool,
    has_app: bool,
    reason: Option<Reason>,
) -> &'static str {
    if !trusted {
        return "needs-permission";
    }
    if paused {
        return "paused";
    }
    let (true, Some(reason)) = (has_app, reason) else {
        return "idle";
    };
    match reason {
        Reason::Paused => "paused",
        Reason::NeedsPermission => "needs-permission",
        Reason::OwnApp => "idle",
        Reason::ExcludedByYou | Reason::BlockedDomain => "excluded",
        Reason::PageUnreadable | Reason::NothingToRead => "unreadable",
        Reason::SensitiveApp
        | Reason::SystemApp
        | Reason::DeniedByDefault
        | Reason::SensitivePage
        | Reason::PrivateWindow
        | Reason::NotSupported => "not-supported",
        Reason::TooLittleText | Reason::NearDuplicate | Reason::AlreadySaved | Reason::Captured => {
            "capturing"
        }
    }
}

pub fn headline(state: &str, app: Option<&str>, reason: Option<Reason>) -> String {
    let app = app.unwrap_or("this app");
    match state {
        "needs-permission" => "Needs Accessibility permission".into(),
        "paused" => "Paused".into(),
        "idle" => "Waiting for you to open a supported app".into(),
        "capturing" => format!("Capturing {app}"),
        "unreadable" => "Couldn't read this page right now".into(),
        "excluded" => match reason {
            Some(Reason::BlockedDomain) => "Not captured: you blocked this website".into(),
            _ => format!("Not captured: you excluded {app}"),
        },
        _ => match reason {
            Some(Reason::SystemApp) => format!("Not captured: {app} is a system app"),
            Some(Reason::SensitiveApp) => format!("Not captured: {app} is a private app"),
            Some(Reason::DeniedByDefault) => format!("Not captured: {app} is left out on purpose"),
            Some(Reason::SensitivePage) => "Not captured: this page is private".into(),
            Some(Reason::PrivateWindow) => "Not captured: this is a private window".into(),
            _ => format!("Not captured: {app} isn't on the list of supported apps"),
        },
    }
}

impl HealthState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                trusted: true,
                paused: false,
                frontmost: None,
                last: None,
                last_capture_at: None,
                consecutive_unreadable: 0,
                recent: VecDeque::new(),
                announced: None,
            }),
        }
    }

    /// Fold one tick's observations in. Returns true when the summarized
    /// state or decision changed (the caller emits `capture-health`).
    pub fn observe(
        &self,
        trusted: bool,
        paused: bool,
        frontmost: Option<FrontApp>,
        update: DecisionUpdate,
        now: i64,
    ) -> bool {
        let mut s = self.inner.lock();
        s.trusted = trusted;
        s.paused = paused;
        if s.frontmost != frontmost {
            s.frontmost = frontmost.clone();
        }
        match update {
            DecisionUpdate::Keep => {}
            DecisionUpdate::Clear => s.last = None,
            DecisionUpdate::Set(reason) => {
                s.last = Some((reason, now));
                match reason {
                    Reason::Captured => {
                        s.last_capture_at = Some(now);
                        s.consecutive_unreadable = 0;
                    }
                    Reason::NearDuplicate | Reason::AlreadySaved | Reason::TooLittleText => {
                        s.consecutive_unreadable = 0;
                    }
                    Reason::PageUnreadable | Reason::NothingToRead => {
                        s.consecutive_unreadable = s.consecutive_unreadable.saturating_add(1);
                    }
                    _ => {}
                }
                if let (Some(app), true) = (frontmost.as_ref(), !matches!(reason, Reason::OwnApp | Reason::Paused)) {
                    let key = app_key(app);
                    let existing = s.recent.iter().position(|r| {
                        r.reason == reason
                            && r.bundle_id
                                .as_deref()
                                .map(str::to_lowercase)
                                .unwrap_or_else(|| r.app_name.to_lowercase())
                                == key
                    });
                    let mut entry = match existing.and_then(|i| s.recent.remove(i)) {
                        Some(mut entry) => {
                            entry.count = entry.count.saturating_add(1);
                            entry
                        }
                        None => Recent {
                            app_name: app.name.clone(),
                            bundle_id: app.bundle_id.clone(),
                            reason,
                            at: now,
                            count: 1,
                        },
                    };
                    entry.at = now;
                    s.recent.push_front(entry);
                    s.recent.truncate(RECENT_CAP);
                }
            }
        }
        let reason = s.last.map(|(r, _)| r);
        let state = coarse_state(s.trusted, s.paused, s.frontmost.is_some(), reason);
        let summary = (state, s.frontmost.as_ref().map(app_key), reason);
        if s.announced.as_ref() == Some(&summary) {
            return false;
        }
        s.announced = Some(summary);
        true
    }

    pub fn snapshot(&self) -> CaptureHealth {
        let s = self.inner.lock();
        let reason = s.last.map(|(r, _)| r);
        let state = coarse_state(s.trusted, s.paused, s.frontmost.is_some(), reason);
        CaptureHealth {
            state,
            headline: headline(state, s.frontmost.as_ref().map(|a| a.name.as_str()), reason),
            trusted: s.trusted,
            paused: s.paused,
            frontmost: s.frontmost.clone(),
            last_decision: reason.map(Reason::decision),
            last_decision_at: s.last.map(|(_, at)| at),
            last_capture_at: s.last_capture_at,
            consecutive_unreadable: s.consecutive_unreadable,
            recent: s
                .recent
                .iter()
                .map(|r| RecentDecision {
                    app_name: r.app_name.clone(),
                    bundle_id: r.bundle_id.clone(),
                    decision: r.reason.decision(),
                    at: r.at,
                    count: r.count,
                    tunable: r.reason.is_tunable(),
                })
                .collect(),
        }
    }
}

#[tauri::command]
pub fn get_capture_health(state: State<'_, Arc<MonitorState>>) -> CaptureHealth {
    state.health.snapshot()
}

#[tauri::command]
pub fn get_app_capture_intervals(
    state: State<'_, Arc<MonitorState>>,
) -> Result<BTreeMap<String, i64>, String> {
    let raw = state.db().get_setting(monitor::INTERVAL_BY_APP_KEY)?;
    Ok(monitor::parse_interval_overrides(raw.as_deref()))
}

/// `ms = None` returns the app to the global speed.
#[tauri::command]
pub fn set_app_capture_interval(
    state: State<'_, Arc<MonitorState>>,
    bundle_id: String,
    ms: Option<i64>,
) -> Result<BTreeMap<String, i64>, String> {
    let db = state.db();
    let raw = db.get_setting(monitor::INTERVAL_BY_APP_KEY)?;
    let next = monitor::with_interval_override(raw.as_deref(), &bundle_id, ms)?;
    let json = serde_json::to_string(&next).map_err(|e| e.to_string())?;
    db.set_setting(monitor::INTERVAL_BY_APP_KEY, &json)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slack() -> Option<FrontApp> {
        Some(FrontApp { name: "Slack".into(), bundle_id: Some("com.tinyspeck.slackmacgap".into()) })
    }
    fn finder() -> Option<FrontApp> {
        Some(FrontApp { name: "Finder".into(), bundle_id: Some("com.apple.finder".into()) })
    }

    #[test]
    fn states_cover_permission_pause_and_each_skip_kind() {
        assert_eq!(coarse_state(false, true, true, Some(Reason::Captured)), "needs-permission");
        assert_eq!(coarse_state(true, true, true, Some(Reason::Captured)), "paused");
        assert_eq!(coarse_state(true, false, false, None), "idle");
        assert_eq!(coarse_state(true, false, true, Some(Reason::Captured)), "capturing");
        assert_eq!(coarse_state(true, false, true, Some(Reason::NearDuplicate)), "capturing");
        assert_eq!(coarse_state(true, false, true, Some(Reason::NotSupported)), "not-supported");
        assert_eq!(coarse_state(true, false, true, Some(Reason::PageUnreadable)), "unreadable");
        assert_eq!(coarse_state(true, false, true, Some(Reason::ExcludedByYou)), "excluded");
        assert_eq!(coarse_state(true, false, true, Some(Reason::OwnApp)), "idle");
    }

    #[test]
    fn headlines_name_the_app_in_plain_words() {
        assert_eq!(headline("capturing", Some("Slack"), None), "Capturing Slack");
        assert_eq!(
            headline("not-supported", Some("Finder"), Some(Reason::NotSupported)),
            "Not captured: Finder isn't on the list of supported apps"
        );
        assert_eq!(headline("unreadable", None, None), "Couldn't read this page right now");
    }

    #[test]
    fn events_fire_only_when_the_summary_changes() {
        let h = HealthState::new();
        assert!(h.observe(true, false, slack(), DecisionUpdate::Set(Reason::Captured), 1));
        // Same app, same reason, later capture: no event.
        assert!(!h.observe(true, false, slack(), DecisionUpdate::Set(Reason::Captured), 2));
        assert!(!h.observe(true, false, slack(), DecisionUpdate::Keep, 3));
        // Different reason for the same app is a change.
        assert!(h.observe(true, false, slack(), DecisionUpdate::Set(Reason::NearDuplicate), 4));
        assert!(h.observe(true, true, slack(), DecisionUpdate::Keep, 5));
        assert!(h.observe(false, true, slack(), DecisionUpdate::Keep, 6));
        assert_eq!(h.snapshot().state, "needs-permission");
    }

    #[test]
    fn recent_decisions_are_distinct_bounded_and_newest_first() {
        let h = HealthState::new();
        h.observe(true, false, finder(), DecisionUpdate::Set(Reason::SystemApp), 1);
        h.observe(true, false, slack(), DecisionUpdate::Set(Reason::Captured), 2);
        h.observe(true, false, slack(), DecisionUpdate::Set(Reason::Captured), 3);
        h.observe(true, false, finder(), DecisionUpdate::Set(Reason::SystemApp), 4);
        let snap = h.snapshot();
        assert_eq!(snap.recent.len(), 2);
        assert_eq!(snap.recent[0].app_name, "Finder");
        assert_eq!(snap.recent[0].count, 2);
        assert_eq!(snap.recent[1].count, 2);
        assert_eq!(snap.last_capture_at, Some(3));
        assert!(snap.recent[1].tunable && !snap.recent[0].tunable);

        for i in 0..100 {
            let app = Some(FrontApp { name: format!("App{i}"), bundle_id: Some(format!("b.{i}")) });
            h.observe(true, false, app, DecisionUpdate::Set(Reason::NotSupported), 10 + i);
        }
        assert_eq!(h.snapshot().recent.len(), RECENT_CAP);
        assert_eq!(h.snapshot().recent[0].app_name, "App99");
    }

    #[test]
    fn unreadable_streak_resets_on_success_and_own_app_is_not_listed() {
        let h = HealthState::new();
        h.observe(true, false, slack(), DecisionUpdate::Set(Reason::NothingToRead), 1);
        h.observe(true, false, slack(), DecisionUpdate::Set(Reason::NothingToRead), 2);
        assert_eq!(h.snapshot().consecutive_unreadable, 2);
        h.observe(true, false, slack(), DecisionUpdate::Set(Reason::Captured), 3);
        assert_eq!(h.snapshot().consecutive_unreadable, 0);
        let own = Some(FrontApp { name: "Memento".into(), bundle_id: None });
        h.observe(true, false, own, DecisionUpdate::Set(Reason::OwnApp), 4);
        assert!(h.snapshot().recent.iter().all(|r| r.app_name != "Memento"));
    }

    #[test]
    fn snapshot_serializes_camel_case() {
        let h = HealthState::new();
        h.observe(true, false, slack(), DecisionUpdate::Set(Reason::Captured), 1);
        let json = serde_json::to_value(h.snapshot()).unwrap();
        assert_eq!(json["state"], "capturing");
        assert_eq!(json["lastDecision"]["reason"], "captured");
        assert_eq!(json["recent"][0]["bundleId"], "com.tinyspeck.slackmacgap");
        assert_eq!(json["recent"][0]["reason"], "captured");
    }
}
