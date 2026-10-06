//! Single source of truth for "was this captured, and if not, why?".
//!
//! Every reason an app or page is skipped (or saved) is one `Reason`, with a
//! stable machine id for the UI, a plain-language label and a short
//! explanation. The pure gate functions here and in `adapters` decide; the
//! monitor only records and acts on the result.

use serde::Serialize;

use crate::adapters::{self, CaptureAppSelection};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    Paused,
    NeedsPermission,
    OwnApp,
    SensitiveApp,
    SystemApp,
    DeniedByDefault,
    ExcludedByYou,
    BlockedDomain,
    SensitivePage,
    PrivateWindow,
    NotSupported,
    PageUnreadable,
    NothingToRead,
    TooLittleText,
    NearDuplicate,
    AlreadySaved,
    Captured,
}

/// A decision as shown to the user. Built from a `Reason` only, so the copy
/// lives in exactly one place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub reason: Reason,
    pub label: String,
    pub explanation: String,
    /// Where the user can change this, when they can.
    pub hint: Option<String>,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Reason::Paused => "Capture is paused",
            Reason::NeedsPermission => "Needs Accessibility permission",
            Reason::OwnApp => "Memento itself",
            Reason::SensitiveApp => "Private app",
            Reason::SystemApp => "System app",
            Reason::DeniedByDefault => "Not captured by design",
            Reason::ExcludedByYou => "Excluded by you",
            Reason::BlockedDomain => "Blocked website",
            Reason::SensitivePage => "Private page",
            Reason::PrivateWindow => "Private window",
            Reason::NotSupported => "Not a supported app",
            Reason::PageUnreadable => "Couldn't read this page",
            Reason::NothingToRead => "Nothing readable right now",
            Reason::TooLittleText => "Too little text",
            Reason::NearDuplicate => "Same as the last capture",
            Reason::AlreadySaved => "Already saved",
            Reason::Captured => "Saved",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Reason::Paused => "Nothing is captured while capture is paused.",
            Reason::NeedsPermission => "Memento can't read other apps until Accessibility is allowed.",
            Reason::OwnApp => "Memento never captures its own window.",
            Reason::SensitiveApp => "Password managers and wallets are never captured.",
            Reason::SystemApp => "System screens like Finder and System Settings are never captured.",
            Reason::DeniedByDefault => "Developer, creative, finance and meeting apps are left out on purpose.",
            Reason::ExcludedByYou => "You turned capture off for this app.",
            Reason::BlockedDomain => "You blocked this website.",
            Reason::SensitivePage => "Sign-in, account, payment and settings pages are never captured.",
            Reason::PrivateWindow => "Private browsing windows are never captured.",
            Reason::NotSupported => "Memento only reads apps and sites it has a dedicated reader for.",
            Reason::PageUnreadable => "The browser didn't share the page address, so it was skipped for now.",
            Reason::NothingToRead => "The app is supported, but nothing readable was on screen.",
            Reason::TooLittleText => "There wasn't enough text on screen to be worth saving.",
            Reason::NearDuplicate => "The screen is almost the same as the last capture.",
            Reason::AlreadySaved => "This screen is already in your memory.",
            Reason::Captured => "A new memory was saved from this screen.",
        }
    }

    pub fn hint(self) -> Option<&'static str> {
        match self {
            Reason::Paused => Some("Resume from the menu bar icon."),
            Reason::NeedsPermission => Some("Allow Accessibility in the Permissions section above."),
            Reason::ExcludedByYou => Some("Change this in Excluded apps."),
            Reason::BlockedDomain => Some("Change this in Blocked domains."),
            Reason::PageUnreadable | Reason::NothingToRead => Some("It will be tried again shortly."),
            _ => None,
        }
    }

    /// Whether the user could meaningfully change how often this app is read.
    pub fn is_tunable(self) -> bool {
        !matches!(
            self,
            Reason::OwnApp
                | Reason::SensitiveApp
                | Reason::SystemApp
                | Reason::DeniedByDefault
                | Reason::ExcludedByYou
                | Reason::NotSupported
        )
    }

    pub fn decision(self) -> Decision {
        Decision {
            reason: self,
            label: self.label().to_string(),
            explanation: self.explanation().to_string(),
            hint: self.hint().map(str::to_string),
        }
    }
}

// Credential / secret-bearing apps that we intentionally never capture
// (mirrors Minimi's shipped denylist).
const SENSITIVE_APPS: &[&str] = &[
    "1password",
    "bitwarden",
    "lastpass",
    "dashlane",
    "keeper",
    "authy",
    "wallet",
    "keychain",
    "keychain access",
    "safari password",
    "proton pass",
    "1password 8",
    "bitwarden desktop",
    "chrome password",
    "emby",
];

pub fn is_sensitive(app_name: &str) -> bool {
    let lower = app_name.to_lowercase();
    SENSITIVE_APPS.iter().any(|s| lower.contains(s))
}

/// macOS system-UI surfaces: worth navigating, never worth remembering.
const SYSTEM_APPS: &[&str] = &[
    "system settings",
    "system preferences",
    "finder",
    "spotlight",
    "notification center",
    "control center",
    "mission control",
    "activity monitor",
    "disk utility",
    "system information",
    "console",
    "app store",
    "software update",
    "screenshot",
    "font book",
    "migration assistant",
    "network utility",
    "archive utility",
    "digital color meter",
];

pub fn is_system_app(app_name: &str) -> bool {
    let lower = app_name.to_lowercase();
    SYSTEM_APPS.iter().any(|s| lower == *s)
}

/// Apps whose content Minimi's registry treats as never-worth-remembering,
/// adopted here: developer & terminal tools, creative suites, finance/HR
/// portals, and meeting apps (meetings are captured via the microphone, not
/// the screen). Exact display-name match so substrings like "adp" inside
/// "Adobe" can never over-block.
const BLOCKED_ACTIVITY_APPS: &[&str] = &[
    // developer tools
    "visual studio code",
    "code",
    "cursor",
    "xcode",
    "iterm",
    "iterm2",
    "terminal",
    "ghostty",
    "warp",
    "github desktop",
    "postman",
    "intellij idea",
    "pycharm",
    "goland",
    "webstorm",
    "rubymine",
    "datagrip",
    "sublime text",
    // creative tools
    "final cut pro",
    "motion",
    "imovie",
    "premiere pro",
    "after effects",
    "photoshop",
    "illustrator",
    // meetings happen via the microphone pipeline, not screen capture
    "facetime",
    "zoom.us",
    "zoom",
    // finance / hr / health portals
    "quickbooks",
    "mint",
    "workday",
    "bamboohr",
    "gusto",
    "adp",
    "paylocity",
    "rippling",
    "mychart",
    "practo",
];

pub fn is_blocked_activity_app(app_name: &str) -> bool {
    let lower = app_name.to_lowercase();
    BLOCKED_ACTIVITY_APPS.iter().any(|s| lower == *s)
}

/// App-level gates, checked before any content is read. `None` means the app
/// itself may be looked at (the page-level gates in `adapters::plan` follow).
pub fn app_gate(
    pid: i32,
    own_pid: i32,
    name: &str,
    bundle_id: Option<&str>,
    excluded: &[CaptureAppSelection],
) -> Option<Reason> {
    if pid == own_pid {
        Some(Reason::OwnApp)
    } else if is_sensitive(name) {
        Some(Reason::SensitiveApp)
    } else if is_system_app(name) {
        Some(Reason::SystemApp)
    } else if is_blocked_activity_app(name) {
        Some(Reason::DeniedByDefault)
    } else if adapters::is_excluded_identity(bundle_id, excluded) {
        Some(Reason::ExcludedByYou)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn excluded(bundle: &str) -> Vec<CaptureAppSelection> {
        vec![CaptureAppSelection { name: "X".into(), bundle_id: bundle.into() }]
    }

    #[test]
    fn system_ui_apps_are_never_recorded() {
        assert!(is_system_app("System Settings"));
        assert!(is_system_app("Finder"));
        assert!(is_system_app("Activity Monitor"));
        assert!(!is_system_app("Google Chrome"));
        assert!(!is_system_app("Visual Studio Code"));
    }

    #[test]
    fn app_gates_apply_in_order_with_stable_reasons() {
        let none: Vec<CaptureAppSelection> = vec![];
        assert_eq!(app_gate(5, 5, "Slack", None, &none), Some(Reason::OwnApp));
        assert_eq!(app_gate(1, 5, "1Password 8", None, &none), Some(Reason::SensitiveApp));
        assert_eq!(app_gate(1, 5, "Finder", None, &none), Some(Reason::SystemApp));
        assert_eq!(app_gate(1, 5, "Terminal", None, &none), Some(Reason::DeniedByDefault));
        assert_eq!(app_gate(1, 5, "Adobe Acrobat", None, &none), None);
        assert_eq!(
            app_gate(1, 5, "Slack", Some("COM.tinyspeck.slackmacgap"), &excluded("com.tinyspeck.slackmacgap")),
            Some(Reason::ExcludedByYou)
        );
        // Sensitive wins over a user exclusion: the stronger reason is reported.
        assert_eq!(
            app_gate(1, 5, "Bitwarden", Some("x.y"), &excluded("x.y")),
            Some(Reason::SensitiveApp)
        );
    }

    #[test]
    fn every_reason_has_copy_and_actionable_hints_point_somewhere() {
        let all = [
            Reason::Paused, Reason::NeedsPermission, Reason::OwnApp, Reason::SensitiveApp,
            Reason::SystemApp, Reason::DeniedByDefault, Reason::ExcludedByYou,
            Reason::BlockedDomain, Reason::SensitivePage, Reason::PrivateWindow,
            Reason::NotSupported, Reason::PageUnreadable, Reason::NothingToRead,
            Reason::TooLittleText, Reason::NearDuplicate, Reason::AlreadySaved, Reason::Captured,
        ];
        for reason in all {
            assert!(!reason.label().is_empty() && !reason.explanation().is_empty());
        }
        assert_eq!(Reason::ExcludedByYou.hint(), Some("Change this in Excluded apps."));
        assert!(Reason::NotSupported.hint().is_none());
        assert!(!Reason::NotSupported.is_tunable());
        assert!(Reason::Captured.is_tunable());
        let json = serde_json::to_string(&Reason::NeedsPermission.decision()).unwrap();
        assert!(json.contains("\"reason\":\"needsPermission\""));
    }
}
