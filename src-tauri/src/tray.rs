//! Menu bar status item.
//!
//! - The icon shows the app's state (running, paused, recording, needs
//!   attention); a live title shows the recording timer or how many follow-ups
//!   are due; the tooltip spells the state out in words.
//! - Right-click opens the menu: a status header, quick actions and pause.
//! - Left-click opens a popover panel (the `tray-panel` window) anchored under
//!   the icon: status, pause, recording, a quick note and follow-ups.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Rect, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;

use crate::db;
use crate::localtime::{self as lt, Offset};
use crate::state::MonitorState;
use crate::RecordingState;

pub const TRAY_ID: &str = "poppy-tray";
pub const PANEL_LABEL: &str = "tray-panel";

// Template images (black + alpha) are tinted by macOS for light/dark menu
// bars; the recording icon is a deliberate un-tinted red dot.
const ICON_IDLE: &[u8] = include_bytes!("../icons/tray/tray-idle.png");
const ICON_PAUSED: &[u8] = include_bytes!("../icons/tray/tray-paused.png");
const ICON_ATTENTION: &[u8] = include_bytes!("../icons/tray/tray-attention.png");
const ICON_RECORDING: &[u8] = include_bytes!("../icons/tray/tray-recording.png");

const PANEL_WIDTH: f64 = 340.0;
const PANEL_HEIGHT: f64 = 480.0;
const SCREEN_MARGIN: f64 = 8.0;
const ICON_GAP: f64 = 6.0;
/// Clicking the icon while the panel is open first blurs (hides) the panel and
/// then delivers the click; without this guard that click would reopen it.
const REOPEN_GUARD_MS: i64 = 350;
static PANEL_HIDDEN_AT: AtomicI64 = AtomicI64::new(0);

const IDLE_TICK: Duration = Duration::from_secs(2);
const RECORDING_TICK: Duration = Duration::from_secs(1);
const DUE_REFRESH: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------- status

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visual {
    Idle,
    Paused,
    Recording,
    Attention,
}

impl Visual {
    fn image(self) -> &'static [u8] {
        match self {
            Visual::Idle => ICON_IDLE,
            Visual::Paused => ICON_PAUSED,
            Visual::Recording => ICON_RECORDING,
            Visual::Attention => ICON_ATTENTION,
        }
    }

    /// Only the recording dot keeps its own colour.
    fn is_template(self) -> bool {
        self != Visual::Recording
    }
}

/// Everything the menu bar shows, derived from app state.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TrayStatus {
    pub visual: Visual,
    /// Text next to the icon (None clears it).
    pub title: Option<String>,
    pub tooltip: String,
    /// First (disabled) line of the menu.
    pub header: String,
    pub record_label: &'static str,
}

/// The slice of capture health the tray needs.
pub struct HealthInput<'a> {
    pub state: &'a str,
    pub headline: &'a str,
    pub last_capture_at: Option<i64>,
}

fn format_elapsed(ms: i64) -> String {
    let secs = (ms / 1000).max(0);
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn ago(now: i64, then: i64) -> String {
    let minutes = ((now - then) / 60_000).max(0);
    match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes} min ago"),
        _ => format!("{} hr ago", minutes / 60),
    }
}

/// Pure so it can be tested: no app handle, clock, or timezone of its own.
pub fn compute_status(
    paused: bool,
    pause_until: Option<i64>,
    recording_since: Option<i64>,
    now: i64,
    off: Offset,
    health: &HealthInput,
    due_follow_ups: usize,
) -> TrayStatus {
    let paused_note = if paused {
        match pause_until.filter(|until| *until > now) {
            Some(until) => {
                let local = until + off(until);
                let minutes = local.rem_euclid(lt::DAY_MS) / 60_000;
                format!("paused until {:02}:{:02}", minutes / 60, minutes % 60)
            }
            None => "paused".to_string(),
        }
    } else {
        String::new()
    };

    // Recording wins (it is the most time-sensitive); then the user's own
    // pause; then a problem that needs them (e.g. a missing permission).
    let visual = if recording_since.is_some() {
        Visual::Recording
    } else if paused {
        Visual::Paused
    } else if health.state == "needs-permission" {
        Visual::Attention
    } else {
        Visual::Idle
    };

    let elapsed = recording_since.map(|since| format_elapsed(now - since));
    let header = match (&elapsed, paused) {
        (Some(elapsed), _) => format!("Recording · {elapsed}"),
        (None, true) => {
            let mut text = paused_note.clone();
            text[..1].make_ascii_uppercase();
            text
        }
        (None, false) => match health.last_capture_at {
            Some(at) if health.state == "capturing" => {
                format!("{} · saved {}", health.headline, ago(now, at))
            }
            _ => health.headline.to_string(),
        },
    };

    let title = match &elapsed {
        Some(elapsed) => Some(format!("REC {elapsed}")),
        None if due_follow_ups > 0 => Some(due_follow_ups.to_string()),
        None => None,
    };

    let mut tooltip = format!("Memento — {}", header.to_lowercase());
    if elapsed.is_some() && paused {
        tooltip.push_str("; capture paused");
    }
    if due_follow_ups > 0 {
        tooltip.push_str(&format!(
            "; {due_follow_ups} follow-up{} due",
            if due_follow_ups == 1 { "" } else { "s" }
        ));
    }

    TrayStatus {
        visual,
        title,
        tooltip,
        header,
        record_label: if recording_since.is_some() {
            "Stop recording"
        } else {
            "Start recording"
        },
    }
}

// ------------------------------------------------------------------ setup

pub struct TrayItems {
    pub pause: std::sync::Mutex<Option<MenuItem<tauri::Wry>>>,
    header: MenuItem<tauri::Wry>,
    record: MenuItem<tauri::Wry>,
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let state = app.state::<Arc<MonitorState>>();

    let header = MenuItem::with_id(app, "status", "Memento", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open Memento", true, None::<&str>)?;
    let new_memory = MenuItem::with_id(
        app,
        "new-memory",
        "New memory",
        true,
        Some("CmdOrCtrl+Shift+M"),
    )?;
    let overlay = MenuItem::with_id(
        app,
        "show-overlay",
        "Ask your day",
        true,
        Some("CmdOrCtrl+Shift+L"),
    )?;
    let record = MenuItem::with_id(app, "record", "Start recording", true, None::<&str>)?;
    let pause = MenuItem::with_id(
        app,
        "toggle-pause",
        if state.is_paused() {
            "Resume capture"
        } else {
            "Pause capture"
        },
        true,
        None::<&str>,
    )?;
    // Timed pause: capture auto-resumes after the chosen window.
    let mut pause_for = SubmenuBuilder::new(app, "Pause for");
    for (id, label) in [
        ("pause-5", "5 minutes"),
        ("pause-10", "10 minutes"),
        ("pause-30", "30 minutes"),
        ("pause-60", "1 hour"),
    ] {
        pause_for = pause_for.item(&MenuItem::with_id(app, id, label, true, None::<&str>)?);
    }
    let pause_for = pause_for.build()?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let quit = MenuItem::with_id(app, "quit", "Quit Memento", true, Some("CmdOrCtrl+Q"))?;

    let menu = Menu::with_items(
        app,
        &[
            &header,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &new_memory,
            &overlay,
            &PredefinedMenuItem::separator(app)?,
            &record,
            &pause,
            &pause_for,
            &PredefinedMenuItem::separator(app)?,
            &settings,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        // Left-click is the popover panel; the menu is on right-click.
        .show_menu_on_left_click(false)
        .icon(Image::from_bytes(Visual::Idle.image())?)
        .icon_as_template(true)
        .tooltip("Memento — private memory layer")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => crate::show_main_window(app),
            "new-memory" => {
                crate::show_main_window(app);
                let _ = app.emit("new-memory", ());
            }
            "show-overlay" => crate::overlay::expand(app),
            "settings" => show_view(app, "settings"),
            "record" => {
                let app = app.clone();
                tauri::async_runtime::spawn_blocking(move || toggle_recording(&app));
            }
            "toggle-pause" => {
                let state = app.state::<Arc<MonitorState>>();
                let next = !state.is_paused();
                // Manual pauses are indefinite; timed pauses own the deadline.
                state.set_pause_until(None);
                state.set_paused(next);
                let _ = app.emit("pause-changed", next);
                update_pause_label(app, next);
            }
            "pause-5" | "pause-10" | "pause-30" | "pause-60" => {
                let minutes: i64 = match event.id().as_ref() {
                    "pause-5" => 5,
                    "pause-10" => 10,
                    "pause-30" => 30,
                    _ => 60,
                };
                let state = app.state::<Arc<MonitorState>>();
                state.set_pause_until(Some(db::now_ms() + minutes * 60_000));
                state.set_paused(true);
                let _ = app.emit("pause-changed", true);
                update_pause_label(app, true);
            }
            "quit" => {
                if let Some(state) = app.try_state::<Arc<MonitorState>>() {
                    state.stop();
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                toggle_panel(tray.app_handle(), &rect);
            }
        })
        .build(app)?;

    app.manage(TrayItems {
        pause: std::sync::Mutex::new(Some(pause)),
        header,
        record,
    });

    if let Err(error) = setup_panel(app) {
        log::warn!("tray panel unavailable: {error}");
    }
    spawn_status_updater(app.clone());
    Ok(())
}

// ------------------------------------------------------------ popover panel

fn setup_panel(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, PANEL_LABEL, WebviewUrl::App("index.html".into()))
        .title("Memento")
        .inner_size(PANEL_WIDTH, PANEL_HEIGHT)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .visible(false)
        .focused(false)
        .transparent(true)
        // Deliver the first click instead of only activating the window.
        .accept_first_mouse(true)
        .build()?;
    let _ = window.set_background_color(Some(tauri::utils::config::Color(0, 0, 0, 0)));
    // Open over full-screen apps too.
    let _ = window.set_visible_on_all_workspaces(true);
    let blurred = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            hide_panel_window(&blurred);
        }
    });
    Ok(())
}

fn hide_panel_window(window: &WebviewWindow) {
    PANEL_HIDDEN_AT.store(db::now_ms(), Ordering::Relaxed);
    let _ = window.hide();
}

pub fn hide_panel(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(PANEL_LABEL) {
        hide_panel_window(&window);
    }
}

fn toggle_panel(app: &AppHandle, icon: &Rect) {
    let Some(window) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        hide_panel_window(&window);
        return;
    }
    if db::now_ms() - PANEL_HIDDEN_AT.load(Ordering::Relaxed) < REOPEN_GUARD_MS {
        return;
    }
    // The icon rect arrives in physical pixels on macOS; use it only to pick
    // the display, then convert exactly with that display's scale factor.
    let approx = icon.position.to_physical::<f64>(1.0);
    let monitor = app
        .monitor_from_point(approx.x, approx.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let scale = monitor.scale_factor();
    let position = icon.position.to_physical::<f64>(scale);
    let size = icon.size.to_physical::<f64>(scale);
    let screen = monitor.position();
    let screen_size = monitor.size();
    let (x, y) = panel_origin(
        position.x + size.width / 2.0,
        position.y + size.height,
        PANEL_WIDTH * scale,
        f64::from(screen.x),
        f64::from(screen_size.width),
        SCREEN_MARGIN * scale,
        ICON_GAP * scale,
    );
    let _ = window.set_size(LogicalSize::new(PANEL_WIDTH, PANEL_HEIGHT));
    let _ = window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    let _ = window.show();
    let _ = window.set_focus();
    let _ = app.emit("tray-panel-shown", ());
}

/// Top-left of the panel (physical px): centred under the icon, kept fully on
/// the display with a margin, a small gap below the menu bar.
pub fn panel_origin(
    icon_center_x: f64,
    icon_bottom: f64,
    panel_width: f64,
    screen_x: f64,
    screen_width: f64,
    margin: f64,
    gap: f64,
) -> (f64, f64) {
    let min_x = screen_x + margin;
    let max_x = (screen_x + screen_width - panel_width - margin).max(min_x);
    ((icon_center_x - panel_width / 2.0).clamp(min_x, max_x), icon_bottom + gap)
}

#[tauri::command]
pub fn tray_panel_hide(app: AppHandle) {
    hide_panel(&app);
}

/// Show the main window on `view` ("today" | "memory" | "actions" |
/// "meetings" | "settings") and close the panel.
#[tauri::command]
pub fn open_main_view(app: AppHandle, view: String) {
    show_view(&app, &view);
}

fn show_view(app: &AppHandle, view: &str) {
    if !matches!(view, "today" | "memory" | "actions" | "meetings" | "settings") {
        return;
    }
    hide_panel(app);
    crate::show_main_window(app);
    let _ = app.emit("menu-command", view);
}

// -------------------------------------------------------- recording from tray

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Start or stop a recording from the menu. Starting needs the microphone to
/// be allowed already (the permission prompt belongs in the main window);
/// without Screen Recording it records the microphone only, like auto-start.
fn toggle_recording(app: &AppHandle) {
    let (Some(state), Some(recordings)) = (
        app.try_state::<Arc<MonitorState>>(),
        app.try_state::<Arc<RecordingState>>(),
    ) else {
        return;
    };
    let (state, recordings) = (state.inner().clone(), recordings.inner().clone());

    if recordings.active.lock().is_some() {
        if let Err(error) = crate::stop_recording_blocking(app, &state, &recordings) {
            notify(app, "Couldn't stop the recording", &error);
        }
        return;
    }

    let (record_mic, record_system) = {
        let db = state.db();
        let flag = |key: &str| db.get_setting(key).ok().flatten().is_none_or(|v| v == "true");
        (flag("recordMic"), flag("recordSystem"))
    };
    let permissions = tauri::async_runtime::block_on(crate::permissions::get());
    let mic_ok = permissions.microphone == crate::permissions::PermissionState::Granted;
    let system_ok = permissions.screen_recording == crate::permissions::PermissionState::Granted;
    if (record_mic || !record_system) && !mic_ok {
        show_view(app, "meetings");
        notify(
            app,
            "Microphone access needed",
            "Allow the microphone in Memento to start recording.",
        );
        return;
    }
    let mode = match (record_mic, record_system && system_ok) {
        (true, true) => "both",
        (false, true) => "system",
        _ => "mic",
    };
    match crate::start_recording_blocking(app, &state, &recordings, mode, "meeting") {
        Ok(start) => {
            let _ = app.emit("recording-started", &start);
            if record_system && !system_ok {
                notify(
                    app,
                    "Recording your microphone only",
                    "Allow Screen & System Audio Recording to capture the other people on a call.",
                );
            }
        }
        Err(error) => notify(app, "Couldn't start recording", &error),
    }
}

// ------------------------------------------------------------ status updater

fn due_follow_ups(state: &MonitorState, now: i64) -> usize {
    state
        .db()
        .conn
        .query_row(
            "SELECT COUNT(*) FROM action_items
             WHERE status = 'open' AND remind_at IS NOT NULL AND remind_at <= ?1",
            [now],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as usize
}

/// Keeps the icon, title, tooltip and menu in step with app state. Wakes every
/// couple of seconds when idle and every second only while a recording runs
/// (to move the timer); it only touches the tray when something changed, and
/// exits when the app is shutting down.
fn spawn_status_updater(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("memento-tray-status".into())
        .spawn(move || {
            let mut last: Option<TrayStatus> = None;
            let mut due = 0usize;
            let mut due_checked: Option<Instant> = None;
            loop {
                let Some(monitor) = app.try_state::<Arc<MonitorState>>() else {
                    return;
                };
                if !monitor.running.load(Ordering::Relaxed) {
                    return;
                }
                let recording_since = app
                    .try_state::<Arc<RecordingState>>()
                    .and_then(|r| r.active.lock().as_ref().map(|a| a.started_ms));
                let now = db::now_ms();
                if due_checked.is_none_or(|at| at.elapsed() >= DUE_REFRESH) {
                    due = due_follow_ups(&monitor, now);
                    due_checked = Some(Instant::now());
                }
                let health = monitor.health.snapshot();
                let status = compute_status(
                    monitor.is_paused(),
                    monitor.pause_until(),
                    recording_since,
                    now,
                    &lt::system_offset,
                    &HealthInput {
                        state: health.state,
                        headline: &health.headline,
                        last_capture_at: health.last_capture_at,
                    },
                    due,
                );
                if last.as_ref() != Some(&status) {
                    apply_status(&app, &status, last.as_ref());
                    last = Some(status);
                }
                std::thread::sleep(if recording_since.is_some() {
                    RECORDING_TICK
                } else {
                    IDLE_TICK
                });
            }
        });
    if let Err(error) = spawned {
        log::warn!("tray status updater did not start: {error}");
    }
}

fn apply_status(app: &AppHandle, status: &TrayStatus, previous: Option<&TrayStatus>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    if previous.is_none_or(|p| p.visual != status.visual) {
        if let Ok(image) = Image::from_bytes(status.visual.image()) {
            let _ = tray.set_icon_with_as_template(Some(image), status.visual.is_template());
        }
    }
    if previous.is_none_or(|p| p.title != status.title) {
        let _ = tray.set_title(status.title.as_deref());
    }
    if previous.is_none_or(|p| p.tooltip != status.tooltip) {
        let _ = tray.set_tooltip(Some(status.tooltip.as_str()));
    }
    if let Some(items) = app.try_state::<TrayItems>() {
        if previous.is_none_or(|p| p.header != status.header) {
            let _ = items.header.set_text(&status.header);
        }
        if previous.is_none_or(|p| p.record_label != status.record_label) {
            let _ = items.record.set_text(status.record_label);
        }
    }
}

pub fn update_pause_label(app: &AppHandle, paused: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        if let Some(menu_item) = items.pause.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            let _ = menu_item.set_text(if paused {
                "Resume capture"
            } else {
                "Pause capture"
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(_: i64) -> i64 {
        0
    }

    const NOW: i64 = 1_700_000_000_000;

    fn healthy<'a>() -> HealthInput<'a> {
        HealthInput {
            state: "capturing",
            headline: "Capturing Slack",
            last_capture_at: Some(NOW - 2 * 60_000),
        }
    }

    fn status(paused: bool, until: Option<i64>, rec: Option<i64>, h: &HealthInput, due: usize) -> TrayStatus {
        compute_status(paused, until, rec, NOW, &utc, h, due)
    }

    #[test]
    fn idle_shows_the_plain_glyph_and_no_title() {
        let s = status(false, None, None, &healthy(), 0);
        assert_eq!(s.visual, Visual::Idle);
        assert_eq!(s.title, None);
        assert_eq!(s.header, "Capturing Slack · saved 2 min ago");
        assert_eq!(s.record_label, "Start recording");
        assert!(s.tooltip.contains("capturing slack"));
    }

    #[test]
    fn paused_uses_the_paused_icon_and_names_the_resume_time() {
        let s = status(true, None, None, &healthy(), 0);
        assert_eq!(s.visual, Visual::Paused);
        assert_eq!(s.header, "Paused");
        let until = status(true, Some(NOW + 20 * 60_000), None, &healthy(), 0);
        assert!(until.header.starts_with("Paused until "), "{}", until.header);
        // A stale deadline in the past is just "paused".
        assert_eq!(status(true, Some(NOW - 1), None, &healthy(), 0).header, "Paused");
    }

    #[test]
    fn recording_wins_and_shows_the_timer() {
        let s = status(true, None, Some(NOW - 75_000), &healthy(), 3);
        assert_eq!(s.visual, Visual::Recording);
        assert_eq!(s.title.as_deref(), Some("REC 01:15"));
        assert_eq!(s.header, "Recording · 01:15");
        assert_eq!(s.record_label, "Stop recording");
        assert!(s.tooltip.contains("capture paused"));
        assert!(s.tooltip.contains("3 follow-ups due"));
    }

    #[test]
    fn a_missing_permission_asks_for_attention() {
        let health = HealthInput { state: "needs-permission", headline: "Needs Accessibility permission", last_capture_at: None };
        let s = status(false, None, None, &health, 0);
        assert_eq!(s.visual, Visual::Attention);
        assert_eq!(s.header, "Needs Accessibility permission");
        // The user's own pause still takes precedence over a problem.
        assert_eq!(status(true, None, None, &health, 0).visual, Visual::Paused);
    }

    #[test]
    fn due_follow_ups_show_as_a_count() {
        let s = status(false, None, None, &healthy(), 1);
        assert_eq!(s.title.as_deref(), Some("1"));
        assert!(s.tooltip.ends_with("1 follow-up due"));
        assert_eq!(status(false, None, None, &healthy(), 0).title, None);
    }

    #[test]
    fn only_the_recording_dot_is_untinted() {
        for v in [Visual::Idle, Visual::Paused, Visual::Attention] {
            assert!(v.is_template());
        }
        assert!(!Visual::Recording.is_template());
    }

    #[test]
    fn the_icons_are_valid_pngs() {
        for v in [Visual::Idle, Visual::Paused, Visual::Recording, Visual::Attention] {
            assert!(Image::from_bytes(v.image()).is_ok());
        }
    }

    #[test]
    fn panel_is_centred_under_the_icon_and_kept_on_screen() {
        // Plenty of room: centred, 6px below the icon.
        assert_eq!(panel_origin(1000.0, 24.0, 340.0, 0.0, 2000.0, 8.0, 6.0), (830.0, 30.0));
        // Icon near the right edge: pushed left to stay within the margin.
        assert_eq!(panel_origin(1990.0, 24.0, 340.0, 0.0, 2000.0, 8.0, 6.0).0, 1652.0);
        // Icon near the left edge of a display that starts at x = 2000.
        assert_eq!(panel_origin(2010.0, 24.0, 340.0, 2000.0, 1500.0, 8.0, 6.0).0, 2008.0);
        // A display narrower than the panel never yields a negative range.
        assert_eq!(panel_origin(50.0, 24.0, 340.0, 0.0, 300.0, 8.0, 6.0).0, 8.0);
    }

    #[test]
    fn elapsed_formatting() {
        assert_eq!(format_elapsed(0), "00:00");
        assert_eq!(format_elapsed(61_000), "01:01");
        assert_eq!(format_elapsed(3_725_000), "1:02:05");
        assert_eq!(format_elapsed(-5), "00:00");
    }

    #[test]
    fn ago_is_plain_language() {
        assert_eq!(ago(NOW, NOW - 10_000), "just now");
        assert_eq!(ago(NOW, NOW - 90_000), "1 min ago");
        assert_eq!(ago(NOW, NOW - 3 * 3_600_000), "3 hr ago");
    }
}
