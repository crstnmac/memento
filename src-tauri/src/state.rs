//! Shared application state.

use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;

use crate::adapters::{self, CaptureAppSelection};
use crate::db::Db;
use crate::health::HealthState;

pub struct MonitorState {
    pub running: AtomicBool,
    pub paused: AtomicBool,
    pub health: HealthState,
    /// When capture was paused for a fixed duration ("pause for 10 minutes"),
    /// the epoch-ms moment capture should resume. None = indefinite pause.
    pause_until: Mutex<Option<i64>>,
    db: Mutex<Db>,
    excluded_apps: Mutex<Vec<CaptureAppSelection>>,
    blocked_domains: Mutex<Vec<String>>,
}

impl MonitorState {
    pub fn new(db: Db) -> Self {
        let excluded_apps = adapters::effective_excluded_apps(
            db.get_setting("excludedApps").ok().flatten().as_deref(),
        );
        let blocked_domains = adapters::effective_blocked_domains(
            db.get_setting("blockedDomains").ok().flatten().as_deref(),
        );
        Self {
            running: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            health: HealthState::new(),
            pause_until: Mutex::new(None),
            db: Mutex::new(db),
            excluded_apps: Mutex::new(excluded_apps),
            blocked_domains: Mutex::new(blocked_domains),
        }
    }

    pub fn pause_until(&self) -> Option<i64> {
        *self.pause_until.lock()
    }

    pub fn set_pause_until(&self, until: Option<i64>) {
        *self.pause_until.lock() = until;
    }

    pub fn excluded_apps(&self) -> Vec<CaptureAppSelection> {
        self.excluded_apps.lock().clone()
    }

    pub fn set_excluded_apps(&self, apps: Vec<CaptureAppSelection>) {
        *self.excluded_apps.lock() = apps;
    }

    pub fn blocked_domains(&self) -> Vec<String> {
        self.blocked_domains.lock().clone()
    }

    pub fn set_blocked_domains(&self, domains: Vec<String>) {
        *self.blocked_domains.lock() = domains;
    }

    pub fn db(&self) -> parking_lot::MutexGuard<'_, Db> {
        self.db.lock()
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
