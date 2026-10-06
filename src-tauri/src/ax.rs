//! macOS Accessibility (AX) helpers: frontmost application, window titles,
//! and visible-text extraction from the accessibility tree.

use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_application_services::{
    AXError, AXIsProcessTrusted, AXObserver, AXUIElement, AXValue, AXValueType,
};
use objc2_core_foundation::{
    kCFRunLoopDefaultMode, CFArray, CFRetained, CFRunLoop, CFString, CFType, CGPoint,
};
use objc2_core_graphics::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton};

// The kAX* attribute/role constants are C preprocessor macros
// (#define kAXChildrenAttribute CFSTR("AXChildren")), so they are not
// exported symbols. We reproduce them with CFString constants.

const KCHILDREN: &str = "AXChildren";
const KVISIBLE_CHILDREN: &str = "AXVisibleChildren";
const KROWS: &str = "AXRows";
const KCONTENTS: &str = "AXContents";
const KROLE: &str = "AXRole";
const KSUBROLE: &str = "AXSubrole";
const KTITLE: &str = "AXTitle";
const KVALUE: &str = "AXValue";
const KMAIN_WINDOW: &str = "AXMainWindow";
const KFOCUSED_WINDOW: &str = "AXFocusedWindow";
const KWINDOWS: &str = "AXWindows";
const KFOCUSED_UI: &str = "AXFocusedUIElement";
const KSELECTED_TEXT: &str = "AXSelectedText";
const KDOCUMENT: &str = "AXDocument";
const KURL: &str = "AXURL";
const KPOSITION: &str = "AXPosition";
const KSIZE: &str = "AXSize";
// Secure / password fields (and their subtree) are never captured. On macOS a
// password field is role AXTextField with *subrole* AXSecureTextField, so the
// subrole must be checked as well as the role.
const R_SECURE_TEXT_FIELD: &str = "AXSecureTextField";
const R_PASSWORD_FIELD: &str = "AXPasswordField";
// Actions
const A_PRESS: &str = "AXPress";
const A_FOCUS: &str = "AXFocus";
const N_FOCUSED_WINDOW_CHANGED: &str = "AXFocusedWindowChanged";
const N_FOCUSED_UI_CHANGED: &str = "AXFocusedUIElementChanged";
const N_WINDOW_CREATED: &str = "AXWindowCreated";

/// Keeps a per-process AX notification source attached to the capture
/// thread's run loop. Dropping it removes the source before releasing the
/// observer.
pub struct AxChangeObserver {
    observer: CFRetained<AXObserver>,
    run_loop: CFRetained<CFRunLoop>,
}

impl Drop for AxChangeObserver {
    fn drop(&mut self) {
        let source = unsafe { self.observer.run_loop_source() };
        let mode = unsafe { kCFRunLoopDefaultMode };
        self.run_loop.remove_source(Some(&source), mode);
    }
}

unsafe extern "C-unwind" fn ax_change_callback(
    _observer: NonNull<AXObserver>,
    _element: NonNull<AXUIElement>,
    _notification: NonNull<CFString>,
    _context: *mut std::ffi::c_void,
) {
    // The observer source wakes `CFRunLoop::run_in_mode`; the capture worker
    // performs the debounced read after the callback returns.
}

pub fn observe_changes(pid: i32) -> Option<AxChangeObserver> {
    let mut raw = std::ptr::null_mut();
    let result =
        unsafe { AXObserver::create(pid, Some(ax_change_callback), NonNull::new(&mut raw)?) };
    if result != AXError::Success {
        return None;
    }
    let observer = unsafe { CFRetained::from_raw(NonNull::new(raw)?) };
    let app = app_element(pid);
    let mut registered = false;
    for notification in [
        N_FOCUSED_WINDOW_CHANGED,
        N_FOCUSED_UI_CHANGED,
        N_WINDOW_CREATED,
    ] {
        if unsafe { observer.add_notification(&app, &kax(notification), std::ptr::null_mut()) }
            == AXError::Success
        {
            registered = true;
        }
    }
    if !registered {
        return None;
    }
    let run_loop = CFRunLoop::current()?;
    let mode = unsafe { kCFRunLoopDefaultMode };
    let source = unsafe { observer.run_loop_source() };
    run_loop.add_source(Some(&source), mode);
    Some(AxChangeObserver { observer, run_loop })
}

pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

fn kax(attr: &'static str) -> CFRetained<CFString> {
    CFString::from_static_str(attr)
}

/// Frontmost application as (pid, localized name).
#[derive(Clone)]
pub struct AppIdentity {
    pub pid: i32,
    pub name: String,
    pub bundle_id: Option<String>,
}

pub fn frontmost_app() -> Option<AppIdentity> {
    let workspace = NSWorkspace::sharedWorkspace();
    let app: Retained<NSRunningApplication> = workspace.frontmostApplication()?;
    let name = app.localizedName()?.to_string();
    let pid = app.processIdentifier();
    let bundle_id = app.bundleIdentifier().map(|id| id.to_string());
    Some(AppIdentity {
        pid,
        name,
        bundle_id,
    })
}

pub fn running_app_identities() -> Vec<AppIdentity> {
    let workspace = NSWorkspace::sharedWorkspace();
    workspace
        .runningApplications()
        .iter()
        .filter_map(|app| {
            Some(AppIdentity {
                pid: app.processIdentifier(),
                name: app.localizedName()?.to_string(),
                bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
            })
        })
        .collect()
}

fn app_element(pid: i32) -> CFRetained<AXUIElement> {
    let element = unsafe { AXUIElement::new_application(pid) };
    // The default AX timeout is ~6s *per call*. A hung target app would stall
    // the capture thread for minutes across one tree walk; fail fast instead.
    unsafe { element.set_messaging_timeout(1.5) };
    element
}

/// Copy an attribute, returning a retained CFType (None on error).
fn copy_attr(el: &AXUIElement, attribute: &CFString) -> Option<CFRetained<CFType>> {
    unsafe {
        let mut value: *const CFType = std::ptr::null();
        let result = el.copy_attribute_value(attribute, NonNull::new(&mut value)?);
        if result != AXError::Success || value.is_null() {
            return None;
        }
        let ptr = value as *mut CFType;
        Some(CFRetained::from_raw(NonNull::new(ptr)?))
    }
}

fn attr_string(el: &AXUIElement, attr: &'static str) -> Option<String> {
    let attribute = kax(attr);
    let retained = copy_attr(el, &attribute)?;
    let string = retained.downcast::<CFString>().ok()?;
    Some(string.to_string())
}

fn attr_ui_element(el: &AXUIElement, attr: &'static str) -> Option<CFRetained<AXUIElement>> {
    let attribute = kax(attr);
    let retained = copy_attr(el, &attribute)?;
    retained.downcast::<AXUIElement>().ok()
}

/// Checked conversion of an AX attribute value to a CFArray. A misbehaving
/// target app can return any CF type for any attribute, so never assume.
fn as_cf_array(retained: CFRetained<CFType>) -> Option<CFRetained<CFArray<CFType>>> {
    let array = retained.downcast::<CFArray>().ok()?;
    Some(unsafe { CFRetained::cast_unchecked(array) })
}

/// Whether an element is a password / secure text input.
fn is_secure_node(el: &AXUIElement, role: Option<&str>) -> bool {
    let secure = |value: &str| value == R_SECURE_TEXT_FIELD || value == R_PASSWORD_FIELD;
    role.is_some_and(secure) || attr_string(el, KSUBROLE).is_some_and(|sub| secure(&sub))
}

fn attr_ui_elements(el: &AXUIElement, attr: &'static str) -> Vec<CFRetained<AXUIElement>> {
    let attribute = kax(attr);
    let Some(retained) = copy_attr(el, &attribute) else {
        return Vec::new();
    };
    let Some(array) = as_cf_array(retained) else {
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|item| item.downcast::<AXUIElement>().ok())
        .collect()
}

fn children(el: &AXUIElement) -> Vec<CFRetained<AXUIElement>> {
    // Prefer the semantic subset Apple says is currently visible. This avoids
    // walking virtualized/off-screen mail and chat history on every snapshot.
    let visible = attr_ui_elements(el, KVISIBLE_CHILDREN);
    if !visible.is_empty() {
        return visible;
    }
    let mut out: Vec<CFRetained<AXUIElement>> = Vec::new();
    for child_attr in [KCHILDREN, KROWS, KCONTENTS] {
        let attribute = kax(child_attr);
        let Some(retained) = copy_attr(el, &attribute) else {
            continue;
        };
        let Some(array) = as_cf_array(retained) else {
            continue;
        };
        for i in 0..array.len() {
            if let Some(item) = array.get(i) {
                if let Ok(element) = item.downcast::<AXUIElement>() {
                    // A table's rows are also its children: dedupe by CFEqual
                    // (element identity), not by pointer — AX hands back a
                    // fresh CF object per fetch, so pointers never match.
                    let duplicate = out.iter().any(|known| {
                        let a: &CFType = known;
                        let b: &CFType = &element;
                        a == b
                    });
                    if !duplicate {
                        out.push(element);
                    }
                }
            }
        }
    }
    out
}

/// Best-effort document URL exposed by browser AX trees. We deliberately do
/// not fall back to the window title: absence of a URL means web capture is
/// blocked rather than guessed.
pub fn document_url(pid: i32) -> Option<String> {
    let window = focused_window(pid)?;
    find_document_url(&window, 0, &mut 500usize)
}

pub fn document_urls(pid: i32) -> Vec<String> {
    let app = app_element(pid);
    let windows = attr_ui_elements(&app, KWINDOWS);
    let mut urls = Vec::new();
    for window in windows {
        if let Some(url) = find_document_url(&window, 0, &mut 800usize) {
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
    }
    urls
}

fn find_document_url(el: &AXUIElement, depth: usize, budget: &mut usize) -> Option<String> {
    if depth > MAX_DEPTH || *budget == 0 {
        return None;
    }
    *budget -= 1;
    // AXDocument names the document a window shows. AXURL, however, is also
    // exposed by every hyperlink, tab and bookmark — trusting the first one
    // found could misreport the page (and defeat the privacy gate), so it is
    // only accepted from the web area itself.
    let mut candidates = vec![KDOCUMENT];
    if attr_string(el, KROLE).as_deref() == Some("AXWebArea") {
        candidates.push(KURL);
    }
    for attr in candidates {
        if let Some(value) = attr_string(el, attr) {
            let value = value.trim();
            if value.contains("://") {
                return Some(value.to_string());
            }
        }
    }
    for child in children(el) {
        if let Some(url) = find_document_url(&child, depth + 1, budget) {
            return Some(url);
        }
    }
    None
}

/// Focused or main window of the given application process.
fn focused_window(pid: i32) -> Option<CFRetained<AXUIElement>> {
    let app = app_element(pid);
    attr_ui_element(&app, KFOCUSED_WINDOW).or_else(|| attr_ui_element(&app, KMAIN_WINDOW))
}

pub fn window_title(pid: i32) -> Option<String> {
    let window = focused_window(pid)?;
    let title = attr_string(&window, KTITLE)?.trim().to_string();
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

const MAX_DEPTH: usize = 14;
const MAX_CHARS: usize = 6000;
const MAX_NODES: usize = 1200;

/// Collect text from only the given AX roles within the focused window's tree,
/// as a newline-joined block. This is the scoped extraction primitive used by
/// per-app capture adapters (mirrors Minimi's role-restricted parsing).
pub fn collect_role_text(pid: i32, roles: &[&str]) -> String {
    let Some(window) = focused_window(pid) else {
        return String::new();
    };
    let mut pieces: Vec<String> = Vec::new();
    let mut char_budget = MAX_CHARS;
    let mut node_budget = MAX_NODES;
    walk_roles(
        &window,
        0,
        roles,
        &mut pieces,
        &mut char_budget,
        &mut node_budget,
    );
    pieces.join("\n")
}

fn walk_roles(
    el: &AXUIElement,
    depth: usize,
    roles: &[&str],
    out: &mut Vec<String>,
    char_budget: &mut usize,
    node_budget: &mut usize,
) {
    if depth > MAX_DEPTH || *char_budget == 0 || *node_budget == 0 {
        return;
    }
    *node_budget -= 1;
    let role = attr_string(el, KROLE);
    // Never read or descend into secure/password fields or their subtree.
    if is_secure_node(el, role.as_deref()) {
        return;
    }
    if let Some(role) = role.as_deref() {
        if roles.contains(&role) {
            // Prefer the element's value; fall back to its title for cells,
            // links, headings, list items etc.
            if let Some(text) = attr_string(el, KVALUE).or_else(|| attr_string(el, KTITLE)) {
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() && trimmed.chars().count() > 1 {
                    let take = (*char_budget).min(trimmed.chars().count());
                    let taken: String = trimmed.chars().take(take).collect();
                    *char_budget -= take;
                    out.push(taken);
                }
            }
        }
    }
    for child in children(el) {
        if *char_budget == 0 || *node_budget == 0 {
            break;
        }
        walk_roles(
            child.as_ref(),
            depth + 1,
            roles,
            out,
            char_budget,
            node_budget,
        );
    }
}

// ---------------------------------------------------------------------------
// Focused element + selected text (read)
// ---------------------------------------------------------------------------

/// The focused UI element of an application process.
pub fn focused_element(pid: i32) -> Option<CFRetained<AXUIElement>> {
    let app = app_element(pid);
    attr_ui_element(&app, KFOCUSED_UI)
}

/// Text of the focused element (e.g. the value of a focused text field/area
/// or the current selection in a document).
pub fn focused_element_text(pid: i32) -> Option<String> {
    let el = focused_element(pid)?;
    if is_secure_element(&el) {
        return None;
    }
    attr_string(&el, KVALUE)
}

/// Currently selected text in the focused element.
pub fn selected_text(pid: i32) -> Option<String> {
    let el = focused_element(pid)?;
    if is_secure_element(&el) {
        return None;
    }
    attr_string(&el, KSELECTED_TEXT)
}

fn is_secure_element(el: &AXUIElement) -> bool {
    is_secure_node(el, attr_string(el, KROLE).as_deref())
}

/// Raise a window so its app becomes visible/frontmost.
pub fn activate_app(name: &str) -> bool {
    std::process::Command::new("open")
        .arg("-a")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Element registry + interaction (write path)
// ---------------------------------------------------------------------------

struct HandleObj {
    element: CFRetained<AXUIElement>,
    created_at: Instant,
    owner_pid: i32,
}
// AXUIElement is a CoreFoundation handle; safe to move across threads here.
unsafe impl Send for HandleObj {}
unsafe impl Sync for HandleObj {}

static REGISTRY: std::sync::OnceLock<Mutex<HashMap<u64, HandleObj>>> = std::sync::OnceLock::new();

fn registry() -> std::sync::MutexGuard<'static, HashMap<u64, HandleObj>> {
    REGISTRY
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const HANDLE_TTL: Duration = Duration::from_secs(30);
const MAX_HANDLES: usize = 256;

fn prune_registry(reg: &mut HashMap<u64, HandleObj>) {
    reg.retain(|_, item| item.created_at.elapsed() <= HANDLE_TTL);
    if reg.len() >= MAX_HANDLES {
        let mut ids: Vec<(u64, Instant)> = reg
            .iter()
            .map(|(id, item)| (*id, item.created_at))
            .collect();
        ids.sort_by_key(|(_, created)| *created);
        for (id, _) in ids.into_iter().take(reg.len() - MAX_HANDLES + 1) {
            reg.remove(&id);
        }
    }
}

fn store_element(el: CFRetained<AXUIElement>, owner_pid: i32) -> u64 {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let handle = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut reg = registry();
    prune_registry(&mut reg);
    reg.insert(
        handle,
        HandleObj {
            element: el,
            created_at: Instant::now(),
            owner_pid,
        },
    );
    handle
}

fn take_element(handle: u64) -> Option<CFRetained<AXUIElement>> {
    let mut reg = registry();
    prune_registry(&mut reg);
    let item = reg.remove(&handle)?;
    (frontmost_app().map(|app| app.pid) == Some(item.owner_pid)).then_some(item.element)
}

/// Find the first UI element in `pid`'s app whose role matches one of
/// `roles` and whose title/value contains `label` (case-insensitive).
/// Returns a registry handle usable with `set_element_value`/`press_element`.
pub fn find_element_by_label(pid: i32, roles: &[&str], label: &str) -> Option<u64> {
    let app = app_element(pid);
    let needle = label.to_lowercase();
    let mut found: Option<CFRetained<AXUIElement>> = None;
    let mut budget = 400usize;
    find_recursive(&app, 0, roles, &needle, &mut found, &mut budget);
    found.map(|element| store_element(element, pid))
}

fn find_recursive(
    el: &AXUIElement,
    depth: usize,
    roles: &[&str],
    needle: &str,
    found: &mut Option<CFRetained<AXUIElement>>,
    budget: &mut usize,
) {
    if depth > 8 || *budget == 0 || found.is_some() {
        return;
    }
    *budget -= 1;
    let role = attr_string(el, KROLE);
    if is_secure_node(el, role.as_deref()) {
        return;
    }
    if let Some(role) = role.as_deref() {
        if roles.contains(&role) {
            let text = attr_string(el, KVALUE)
                .or_else(|| attr_string(el, KTITLE))
                .unwrap_or_default();
            if text.to_lowercase().contains(needle) {
                *found = Some(unsafe {
                    CFRetained::retain(NonNull::new(el as *const _ as *mut _).unwrap())
                });
                return;
            }
        }
    }
    for child in children(el) {
        find_recursive(&child, depth + 1, roles, needle, found, budget);
        if found.is_some() {
            break;
        }
    }
}

/// Set the value of a registered element (e.g. type into a text field).
pub fn set_element_value(handle: u64, text: &str) -> bool {
    let Some(el) = take_element(handle) else {
        return false;
    };
    let attribute = kax(KVALUE);
    let value = CFString::from_str(text);
    set_element_value_impl(&el, &attribute, &value)
}

fn set_element_value_impl(el: &AXUIElement, attribute: &CFString, value: &CFString) -> bool {
    let as_cftype: &CFType = value;
    let result = unsafe { el.set_attribute_value(attribute, as_cftype) };
    result == AXError::Success
}

/// Perform the default "press" action on a registered element.
pub fn press_element(handle: u64) -> bool {
    let Some(el) = take_element(handle) else {
        return false;
    };
    let action = kax(A_PRESS);
    unsafe { el.perform_action(&action) == AXError::Success }
}

/// Focus (make key) a registered element.
pub fn focus_element(handle: u64) -> bool {
    let Some(el) = take_element(handle) else {
        return false;
    };
    let action = kax(A_FOCUS);
    unsafe { el.perform_action(&action) == AXError::Success }
}

// ---------------------------------------------------------------------------
// Element geometry + cursor movement (agent-facing)
// ---------------------------------------------------------------------------

// AXValue stores C structs as raw bytes; two consecutive f64s suffice for a
// CGPoint/CGSize regardless of which CG type crate is in scope.
#[repr(C)]
struct RawPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
struct RawSize {
    width: f64,
    height: f64,
}

fn read_ax_point(el: &AXUIElement, attr: &'static str) -> Option<(f64, f64)> {
    let attribute = kax(attr);
    let retained = copy_attr(el, &attribute)?;
    let axvalue: CFRetained<AXValue> = retained.downcast::<AXValue>().ok()?;
    let mut value = RawPoint { x: 0.0, y: 0.0 };
    let ptr = NonNull::new(&mut value as *mut _ as *mut std::ffi::c_void)?;
    let ok = unsafe { axvalue.value(AXValueType::CGPoint, ptr) };
    ok.then_some((value.x, value.y))
}

fn read_ax_size(el: &AXUIElement, attr: &'static str) -> Option<(f64, f64)> {
    let attribute = kax(attr);
    let retained = copy_attr(el, &attribute)?;
    let axvalue: CFRetained<AXValue> = retained.downcast::<AXValue>().ok()?;
    let mut value = RawSize {
        width: 0.0,
        height: 0.0,
    };
    let ptr = NonNull::new(&mut value as *mut _ as *mut std::ffi::c_void)?;
    let ok = unsafe { axvalue.value(AXValueType::CGSize, ptr) };
    ok.then_some((value.width, value.height))
}

/// On-screen bounds (x, y, width, height) in global screen points of a
/// registered element, derived from AXPosition + AXSize.
pub fn element_bounds(handle: u64) -> Option<(f64, f64, f64, f64)> {
    // Clone the element out so the registry lock is not held across AX IPC
    // (each call can block up to the messaging timeout).
    let (element, owner_pid) = {
        let reg = registry();
        let obj = reg.get(&handle)?;
        if obj.created_at.elapsed() > HANDLE_TTL {
            return None;
        }
        (obj.element.clone(), obj.owner_pid)
    };
    if frontmost_app().map(|app| app.pid) != Some(owner_pid) {
        return None;
    }
    let el: &AXUIElement = &element;
    let (x, y) = read_ax_point(el, KPOSITION)?;
    let (w, h) = read_ax_size(el, KSIZE)?;
    Some((x, y, w, h))
}

/// Current global cursor position in screen points.
pub fn get_cursor_position() -> Option<(f64, f64)> {
    let p = CGEvent::location(None);
    Some((p.x, p.y))
}

/// Humanized cursor glide from the current position to (x, y), interpolated
/// over `steps` with `step_delay_ms` between steps, then optionally a
/// click-and-hold for `hold_ms` after `pre_click_delay_ms`.
pub fn move_cursor_and_click(
    x: f64,
    y: f64,
    steps: u32,
    step_delay_ms: u64,
    pre_click_delay_ms: u64,
    hold_ms: u64,
    do_click: bool,
) -> Result<(), String> {
    let (sx, sy) = get_cursor_position().ok_or("could not read cursor position")?;
    // The arguments come from the webview: bound them so a bad call can't pin
    // a worker thread (and the mouse) for hours.
    let steps = steps.clamp(1, 240);
    let step_delay_ms = step_delay_ms.min(50);
    let pre_click_delay_ms = pre_click_delay_ms.min(2_000);
    let hold_ms = hold_ms.min(2_000);
    if !x.is_finite() || !y.is_finite() {
        return Err("cursor target must be a finite coordinate".into());
    }
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let px = sx + (x - sx) * t;
        let py = sy + (y - sy) * t;
        if let Some(evt) = CGEvent::new_mouse_event(
            None,
            CGEventType::MouseMoved,
            CGPoint { x: px, y: py },
            CGMouseButton::Left,
        ) {
            CGEvent::post(CGEventTapLocation::SessionEventTap, Some(&evt));
        }
        if step_delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(step_delay_ms));
        }
    }
    if pre_click_delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(pre_click_delay_ms));
    }
    if do_click {
        let pos = CGPoint { x, y };
        if let Some(down) =
            CGEvent::new_mouse_event(None, CGEventType::LeftMouseDown, pos, CGMouseButton::Left)
        {
            CGEvent::post(CGEventTapLocation::SessionEventTap, Some(&down));
        }
        if hold_ms > 0 {
            std::thread::sleep(Duration::from_millis(hold_ms));
        }
        if let Some(up) =
            CGEvent::new_mouse_event(None, CGEventType::LeftMouseUp, pos, CGMouseButton::Left)
        {
            CGEvent::post(CGEventTapLocation::SessionEventTap, Some(&up));
        }
    }
    Ok(())
}
