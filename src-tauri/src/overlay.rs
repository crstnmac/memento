//! Floating "ask your day" overlay — the Tauri take on Minimi's screen chat.
//!
//! A small always-on-top pill floats on the primary display; clicking it (or
//! ⌘⇧L) expands it into a chat panel grounded in the current screen and local
//! memory. The window joins all Spaces so it never vanishes when switching to
//! a fullscreen app, and it can be dragged anywhere — the dragged position is
//! remembered across expand/collapse.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow};

// Room for the grip, label, expand, and dismiss controls on one line.
const PILL_WIDTH: f64 = 320.0;
const PILL_HEIGHT: f64 = 44.0;
const PANEL_WIDTH: f64 = 380.0;
const PANEL_HEIGHT: f64 = 520.0;
const RIGHT_MARGIN: f64 = 20.0;
const BOTTOM_INSET: f64 = 88.0; // keep clear of the Dock
const EDGE_MARGIN: f64 = 8.0; // never let the window leave the screen

const VIEW_HIDDEN: u8 = 0;
const VIEW_PILL: u8 = 1;
const VIEW_CHAT: u8 = 2;
static VIEW: AtomicU8 = AtomicU8::new(VIEW_HIDDEN);

/// Bottom-right corner (logical px) the window should occupy. Updated whenever
/// the user drags the window or it is placed programmatically.
static ANCHOR: Mutex<Option<(f64, f64)>> = Mutex::new(None);

fn anchor() -> Option<(f64, f64)> {
    *ANCHOR.lock().unwrap_or_else(|p| p.into_inner())
}

fn set_anchor(x: f64, y: f64) {
    if let Ok(mut guard) = ANCHOR.lock() {
        *guard = Some((x, y));
    }
}

/// Build the hidden overlay window. Called once from setup.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        "overlay",
        WebviewUrl::App("index.html".into()),
    )
    .title("Memento")
    .inner_size(PILL_WIDTH, PILL_HEIGHT)
    .resizable(false)
    .always_on_top(true)
    .decorations(false)
    .skip_taskbar(true)
    .shadow(false)
    .visible(false)
    .focused(false)
    .transparent(true)
    // Deliver the very first click to the webview instead of only using it
    // to activate the (unfocused) window — otherwise the first click on the
    // pill appears to do nothing.
    .accept_first_mouse(true)
    .build()?;
    let _ = window.set_background_color(Some(tauri::utils::config::Color(
        0, 0, 0, 0,
    )));
    // Join every Space/fullscreen desktop — without this the overlay belongs
    // to one Space and "vanishes" the moment a fullscreen app is focused.
    let _ = window.set_visible_on_all_workspaces(true);
    // Remember wherever the user drags the window so expand/collapse keep
    // their placement instead of snapping back to the screen edge.
    let moved_window = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Moved(position) = event {
            if let (Ok(size), Ok(Some(monitor))) = (
                moved_window.outer_size(),
                moved_window.current_monitor(),
            ) {
                let scale = monitor.scale_factor();
                set_anchor(
                    (f64::from(position.x) + f64::from(size.width)) / scale,
                    (f64::from(position.y) + f64::from(size.height)) / scale,
                );
            }
        }
    });
    Ok(())
}

/// Current view for the frontend: "pill" | "chat" | "hidden".
pub fn view() -> &'static str {
    match VIEW.load(Ordering::Relaxed) {
        VIEW_CHAT => "chat",
        VIEW_PILL => "pill",
        _ => "hidden",
    }
}

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("overlay")
}

/// Resize to `width`×`height` (logical) keeping the bottom-right corner (the
/// dragged anchor, or the default screen-edge spot) fixed, clamped so the
/// window always stays fully visible.
fn place(window: &WebviewWindow, width: f64, height: f64) -> Result<(), String> {
    window
        .set_size(tauri::Size::Logical(tauri::LogicalSize::new(
            width, height,
        )))
        .map_err(|e| e.to_string())?;
    if let Some(monitor) = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or_else(|| window.primary_monitor().ok().flatten())
    {
        let scale = monitor.scale_factor();
        let position = monitor.position();
        let size = monitor.size();
        let origin_x = position.x as f64 / scale;
        let origin_y = position.y as f64 / scale;
        let screen_w = size.width as f64 / scale;
        let screen_h = size.height as f64 / scale;

        let default_x = origin_x + screen_w - RIGHT_MARGIN;
        let default_y = origin_y + screen_h - BOTTOM_INSET;
        let (anchor_x, anchor_y) = anchor().unwrap_or((default_x, default_y));

        // Clamp: the window's top-left must stay on screen and its
        // bottom-right may not cross the edges.
        let br_x = anchor_x.clamp(origin_x + width + EDGE_MARGIN, origin_x + screen_w - EDGE_MARGIN);
        let br_y = anchor_y.clamp(origin_y + height + EDGE_MARGIN, origin_y + screen_h - EDGE_MARGIN);
        set_anchor(br_x, br_y);

        window
            .set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                br_x - width,
                br_y - height,
            )))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn expand(app: &AppHandle) {
    let Some(window) = window(app) else {
        return;
    };
    let _ = window.set_visible_on_all_workspaces(true);
    let _ = place(&window, PANEL_WIDTH, PANEL_HEIGHT);
    let _ = window.show();
    let _ = window.set_focus();
    VIEW.store(VIEW_CHAT, Ordering::Relaxed);
    let _ = app.emit("overlay-view", "chat");
}

pub fn collapse(app: &AppHandle) {
    let Some(window) = window(app) else {
        return;
    };
    VIEW.store(VIEW_PILL, Ordering::Relaxed);
    let _ = app.emit("overlay-view", "pill");
    let _ = place(&window, PILL_WIDTH, PILL_HEIGHT);
}

pub fn hide(app: &AppHandle) {
    VIEW.store(VIEW_HIDDEN, Ordering::Relaxed);
    if let Some(window) = window(app) {
        let _ = window.hide();
    }
}

pub fn toggle(app: &AppHandle) {
    match window(app) {
        Some(window) if window.is_visible().unwrap_or(false) => hide(app),
        _ => expand(app),
    }
}
