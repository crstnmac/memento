//! Local meeting-state detection using multiple independent signals.
//!
//! App presence alone is not enough: Teams/Slack/Discord may run all day.
//! We inspect browser meeting URLs across every AX window and require native
//! call-control text (leave/hang-up plus mute/participants) before declaring
//! a meeting active.

use crate::ax;

const CONTROL_ROLES: &[&str] = &["AXButton", "AXStaticText", "AXMenuItem"];

pub fn meeting_active() -> bool {
    ax::running_app_identities().into_iter().any(|app| {
        if is_browser(&app.name, app.bundle_id.as_deref()) {
            let meeting_url = ax::document_urls(app.pid)
                .iter()
                .any(|url| is_active_meeting_url(url));
            let controls = ax::collect_role_text(app.pid, CONTROL_ROLES).to_lowercase();
            return meeting_url || has_active_call_controls(&controls);
        }
        if !is_meeting_capable(&app.name, app.bundle_id.as_deref()) {
            return false;
        }
        let controls = ax::collect_role_text(app.pid, CONTROL_ROLES).to_lowercase();
        has_active_call_controls(&controls)
    })
}

fn is_browser(name: &str, bundle_id: Option<&str>) -> bool {
    let name = name.to_lowercase();
    let bundle = bundle_id.unwrap_or("").to_lowercase();
    ["chrome", "safari", "edge", "arc", "brave", "firefox"]
        .iter()
        .any(|value| name.contains(value) || bundle.contains(value))
}

fn is_meeting_capable(name: &str, bundle_id: Option<&str>) -> bool {
    let identity = format!(
        "{} {}",
        name.to_lowercase(),
        bundle_id.unwrap_or("").to_lowercase()
    );
    [
        "teams",
        "zoom",
        "facetime",
        "slack",
        "discord",
        "webex",
        "gotomeeting",
    ]
    .iter()
    .any(|value| identity.contains(value))
}

fn has_active_call_controls(text: &str) -> bool {
    let text = text.to_lowercase();
    let terminal = [
        "leave",
        "leave call",
        "leave meeting",
        "hang up",
        "end call",
    ]
    .iter()
    .any(|value| text.contains(value));
    let supporting = [
        "mute",
        "unmute",
        "camera",
        "participants",
        "people",
        "share screen",
    ]
    .iter()
    .filter(|value| text.contains(**value))
    .count();
    terminal && supporting >= 1
}

fn is_active_meeting_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    let Some((_, rest)) = lower.split_once("://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let path = rest.strip_prefix(host).unwrap_or("");
    (host == "meet.google.com" && path.trim_matches('/').len() >= 5)
        || (host.ends_with("teams.microsoft.com")
            && ["meetup-join", "/meet/", "/call/"]
                .iter()
                .any(|part| path.contains(part)))
        || (host.ends_with("zoom.us") && ["/wc/", "/j/"].iter().any(|part| path.contains(part)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_urls_require_an_active_room_path() {
        assert!(is_active_meeting_url(
            "https://meet.google.com/abc-defg-hij"
        ));
        assert!(!is_active_meeting_url("https://meet.google.com/"));
        assert!(is_active_meeting_url(
            "https://teams.microsoft.com/l/meetup-join/abc"
        ));
        assert!(is_active_meeting_url("https://acme.zoom.us/wc/123/join"));
    }

    #[test]
    fn native_controls_require_terminal_and_supporting_signal() {
        assert!(has_active_call_controls("Mute Camera Participants Leave"));
        assert!(!has_active_call_controls("Mute Camera Participants"));
        assert!(!has_active_call_controls("Leave workspace"));
    }
}
