# macOS application capture: recommended architecture

Research date: 2026-08-28. Sources are limited to Apple and Tauri documentation/source.

## Recommendation

Implement capture as a native, event-driven macOS service in the Tauri Rust process:

1. Use `NSWorkspace.shared.runningApplications` to build the app picker. Keep the bundle identifier as the persisted key, the localized name and icon as presentation data, and the PID only as the identifier for the current process instance.
2. Subscribe to `NSWorkspace.shared.notificationCenter` for app launch, termination, and activation. On activation, use the notification's `NSRunningApplication` (or `frontmostApplication`) and reject apps that are not enabled by bundle ID.
3. For the enabled frontmost app, create an application `AXUIElement` from its PID and attach an `AXObserver`. Subscribe at minimum to focused-window, focused-element, and value-change notifications. Read a snapshot after a short debounce rather than continuously walking every app.
4. Read only the focused window's accessibility tree. Prefer visible children over all children where supported, and collect semantic text attributes such as title, value, description/help, and selected text. Bound traversal by node count, depth, and elapsed time, deduplicate strings, and treat unsupported attributes or unresponsive applications as expected failures.
5. Tear down the old observer whenever the frontmost process changes or terminates. Keep a slow reconciliation timer only as a recovery mechanism for missed/unsupported notifications, not as the primary capture mechanism.

This separation also gives the settings UI a reliable model: an installed/running app catalog keyed by bundle ID, plus an explicit set of enabled bundle IDs. A display-name match or a hard-coded list of executable names is not stable enough.

## API choices and evidence

### Enumerating and identifying GUI apps

Apple says `NSWorkspace.runningApplications` returns the running apps, while `frontmostApplication` returns the app that receives key events. [`NSWorkspace`](https://developer.apple.com/documentation/appkit/nsworkspace)

`NSRunningApplication` exposes `bundleIdentifier`, `localizedName`, `icon`, `bundleURL`, `processIdentifier`, and `activationPolicy`. Apple characterizes the bundle identifier as fixed and time-varying properties as race-prone; it also says an `NSRunningApplication` remains valid after termination, although most properties then lose significance. [`NSRunningApplication`](https://developer.apple.com/documentation/appkit/nsrunningapplication)

Use the bundle identifier for settings and capture rules. Use the localized name only for display: Apple explicitly says it depends on the current localization and is suitable for presentation. [`localizedName`](https://developer.apple.com/documentation/appkit/nsrunningapplication/localizedname)

Use the PID to construct the AX application element/observer for the current run, not as a durable identity. Apple notes that not all applications have a PID and explicitly advises against relying on it to compare processes. [`processIdentifier`](https://developer.apple.com/documentation/appkit/nsrunningapplication/processidentifier)

For an app picker intended to show normal GUI apps, filter primarily to `.regular` activation policy, while deliberately deciding whether `.accessory` apps belong in the product. Apple's `.regular` definition is an ordinary app that appears in the Dock and may have UI; accessory apps may have UI but do not appear in the Dock or menu bar. [`NSApplicationActivationPolicy.regular`](https://developer.apple.com/documentation/appkit/nsapplication/activationpolicy-swift.enum/regular)

### Detecting activation without polling

Observe `NSWorkspace.didActivateApplicationNotification` on `NSWorkspace.shared.notificationCenter`. Apple warns that registration on a different notification center will not receive it. The notification carries the affected `NSRunningApplication` under `applicationUserInfoKey`. [`didActivateApplicationNotification`](https://developer.apple.com/documentation/appkit/nsworkspace/didactivateapplicationnotification)

The same workspace notification center provides launch, termination, hide/unhide, deactivation, active-Space, sleep, and wake notifications. These should update cached state and observer lifecycle rather than repeatedly enumerating all processes. [`NSWorkspace`](https://developer.apple.com/documentation/appkit/nsworkspace)

### Detecting focused window/UI changes

Create an AX application element with `AXUIElementCreateApplication(pid)`. Apple defines accessibility elements as the interface for reading an application's UI hierarchy and says they send notifications describing state changes. [`AXUIElement`](https://developer.apple.com/documentation/applicationservices/axuielement)

Create one `AXObserver` for the selected process, register notifications with `AXObserverAddNotification`, and add `AXObserverGetRunLoopSource(observer)` to a live run loop. Apple describes `AXObserverCreate` as creating an observer whose callback receives registered notifications. [`AXObserverCreate`](https://developer.apple.com/documentation/applicationservices/1460133-axobservercreate)

Useful notifications are:

- `kAXFocusedWindowChangedNotification` — the focused window changed. [Apple reference](https://developer.apple.com/documentation/applicationservices/kaxfocusedwindowchangednotification)
- `kAXFocusedUIElementChangedNotification` — the focused accessibility object changed. [Apple reference](https://developer.apple.com/documentation/applicationservices/kaxfocuseduielementchangednotification)
- `kAXValueChangedNotification`, selected-children changes, and layout changes where supported. [Apple notification catalog](https://developer.apple.com/documentation/applicationservices/carbon_accessibility/notifications)

Notification support varies by element/application, so failure to register a particular notification should degrade gracefully. A low-frequency reconciliation snapshot (for example after wake/Space changes or every tens of seconds while active) is reasonable, but high-frequency polling should not be the normal path.

### Reading visible text

Get `kAXFocusedWindowAttribute` from the application element; Apple defines it as the accessibility object for the application's currently focused window. [`kAXFocusedWindowAttribute`](https://developer.apple.com/documentation/applicationservices/kaxfocusedwindowattribute)

Use `AXUIElementCopyAttributeValue` or, when collecting several known attributes on one node, `AXUIElementCopyMultipleAttributeValues`. Apple's API reference also documents common errors such as invalid elements, disabled Accessibility, unsupported APIs, and `kAXErrorCannotComplete` when the target app is unresponsive or messaging fails. [`AXUIElement.h`](https://developer.apple.com/documentation/applicationservices/axuielement_h)

Traverse `kAXVisibleChildrenAttribute` when available. Apple defines it as the first-order children visible to a sighted user and specifically notes that it excludes scrolled-out or obscured children. Fall back to `kAXChildrenAttribute` only when necessary. [`kAXVisibleChildrenAttribute`](https://developer.apple.com/documentation/applicationservices/kaxvisiblechildrenattribute)

The standard attribute catalog includes role, title, value, help, children, visible children, main/focused window, and focused UI element. Query supported attributes rather than assuming every role implements every attribute. [`Accessibility attributes`](https://developer.apple.com/documentation/applicationservices/carbon_accessibility/attributes)

Accessibility is semantic, not a guaranteed complete document extraction API. Some apps expose sparse trees, virtualized content, custom controls, or no useful values. Capture code should report partial/unsupported results rather than treating them as permission denial.

## Permissions and packaging

Call `AXIsProcessTrustedWithOptions` to determine Accessibility authorization. Passing `kAXTrustedCheckOptionPrompt: true` asks macOS to inform an untrusted user asynchronously, but Apple states that prompting does not change the function's immediate return value. Use prompting only from a user action; use the same API without prompting for status refresh. [`AXIsProcessTrustedWithOptions`](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions)

Accessibility authorization and Screen & System Audio Recording are separate. Semantic AX tree reading requires Accessibility. Only request screen-capture authorization if the product actually captures pixels/window imagery; Apple exposes `CGRequestScreenCaptureAccess()` for that workflow. [`CGRequestScreenCaptureAccess`](https://developer.apple.com/documentation/coregraphics/cgrequestscreencaptureaccess())

The app must keep a stable bundle identifier and code-signing identity between launches/builds so macOS can associate privacy grants with the intended app. Launch the `.app` bundle rather than an unbundled child binary during development.

Do not enable App Sandbox for this capture architecture without redesigning it. Apple lists use of Accessibility APIs in assistive apps among activities incompatible with App Sandbox. [`Protecting user data with App Sandbox`](https://developer.apple.com/documentation/security/protecting-user-data-with-app-sandbox)

Tauri capabilities do **not** grant macOS privacy consent. They constrain which windows/webviews may invoke Tauri core/plugin commands. Tauri says capabilities are assigned by window label and permissions from multiple matching capabilities merge; capability files should therefore expose only the commands each UI window needs. [`Tauri capabilities`](https://v2.tauri.app/security/capabilities/)

Native Rust commands registered directly with `invoke_handler` are allowed to all windows by default unless the app provides an `AppManifest::commands` list and corresponding permission definitions. Capture/start/stop/settings commands should be declared and scoped if an untrusted or limited window exists. [`Tauri capabilities`](https://v2.tauri.app/security/capabilities/) and [`Tauri permissions`](https://v2.tauri.app/security/permissions/)

## Efficient service shape

```text
NSWorkspace activation/launch/terminate notification
                    |
                    v
       enabled bundle ID gate + PID change
                    |
                    v
       replace per-process AXObserver
                    |
          focused window / element /
             value-change callbacks
                    |
                    v
        short debounce + bounded AX read
                    |
                    v
       normalize, deduplicate, persist/event
```

Operational safeguards:

- Serialize AX reads onto a dedicated run-loop/thread; never block Tauri's main/UI thread.
- Coalesce bursts (typing can generate many value changes), and skip a snapshot when app, focused window, and normalized content have not changed.
- Set a finite AX messaging timeout and handle `kAXErrorCannotComplete` without retry storms.
- Stop capture when the session resigns active or screens sleep; refresh state after wake.
- Exclude Memento itself, password managers, authentication/payment windows, secure text fields, incognito/private contexts where detectable, and any user-disabled bundle ID.
- Keep permissions status separate from capture health: `authorized`, `not authorized`, `target unsupported`, and `temporarily unreadable` are different states and should be shown differently in Settings.

## What not to use as the primary mechanism

- Repeated shelling out to `osascript`, `ps`, or System Events. It adds process overhead, introduces Apple Events/Automation consent and quoting problems, and duplicates first-party APIs available in-process.
- Matching apps by localized name, executable name, or window title. Persist bundle IDs.
- A tight timer that enumerates all windows/apps and recursively scans every accessibility tree. Use workspace/AX notifications and scan only the enabled frontmost app's focused window.
- Screen capture permission for semantic text alone. Request each macOS privacy permission only for the feature that needs it.
- Treating Tauri ACL entries as TCC permission declarations. Both layers must be correct, but they solve different problems.
