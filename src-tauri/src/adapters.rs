//! Per-app capture adapters.
//!
//! Mirrors Minimi's adapter architecture: a central dispatcher matches the
//! focused app to a specific handler, each producing a normalized text payload
//! that feeds the same memory pipeline. Adapters fail closed: a parser that
//! cannot produce safe content does not fall through to a generic extractor.

use crate::ax;
use crate::decision::Reason;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureAppSelection {
    pub name: String,
    pub bundle_id: String,
}

pub fn effective_excluded_apps(raw: Option<&str>) -> Vec<CaptureAppSelection> {
    raw.and_then(|value| serde_json::from_str::<Vec<CaptureAppSelection>>(value).ok())
        .map(dedup_selections)
        .unwrap_or_default()
}

pub fn effective_blocked_domains(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|domain| normalize_domain(&domain))
        .fold(Vec::new(), |mut domains, domain| {
            if !domains.contains(&domain) {
                domains.push(domain);
            }
            domains
        })
}

pub fn normalize_domain(value: &str) -> Option<String> {
    let mut value = value.trim().to_lowercase();
    if let Some((_, rest)) = value.split_once("://") {
        value = rest.to_string();
    }
    // authority = [userinfo@]host[:port]; keep only the host so
    // "https://user:pw@chase.com" is still recognised as chase.com.
    let authority = value.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .split(':')
        .next()?
        .trim_matches('.');
    if host.is_empty() || host.contains(char::is_whitespace) || !host.contains('.') {
        return None;
    }
    Some(host.to_string())
}

pub fn is_blocked_domain(url: Option<&str>, blocked: &[String]) -> bool {
    let Some(host) = url.and_then(normalize_domain) else {
        return false;
    };
    blocked
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

pub fn is_excluded_identity(bundle_id: Option<&str>, excluded: &[CaptureAppSelection]) -> bool {
    let Some(bundle) = bundle_id else {
        return false;
    };
    excluded
        .iter()
        .any(|app| app.bundle_id.eq_ignore_ascii_case(bundle))
}

/// An app or service Memento can read, and so can find follow-ups from. The
/// user can switch each one off independently of the capture exclusions.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowUpSource {
    /// Stable id stored in settings and on memories (never a display name).
    pub id: &'static str,
    pub name: &'static str,
    /// "chat" | "email" | "meeting" | "productivity"
    pub category: &'static str,
    /// Installed-app bundle ids whose icon represents this source. Empty for
    /// pure web services and recordings (the UI falls back to a category icon).
    pub bundle_ids: &'static [&'static str],
}

macro_rules! source {
    ($id:expr, $name:expr, $category:expr, [$($bundle:expr),*]) => {
        FollowUpSource { id: $id, name: $name, category: $category, bundle_ids: &[$($bundle),*] }
    };
}

/// Only sources that have a capture adapter (or a recording pipeline) — the
/// list never offers an app Memento cannot read.
pub const FOLLOW_UP_SOURCES: &[FollowUpSource] = &[
    source!("whatsapp", "WhatsApp", "chat", ["net.whatsapp.WhatsApp"]),
    source!("slack", "Slack", "chat", ["com.tinyspeck.slackmacgap"]),
    source!("discord", "Discord", "chat", ["com.hnc.Discord"]),
    source!("teams", "Microsoft Teams", "chat", ["com.microsoft.teams2", "com.microsoft.teams"]),
    source!("linkedin", "LinkedIn", "chat", []),
    source!("outlook", "Outlook", "email", ["com.microsoft.Outlook"]),
    source!("apple-mail", "Apple Mail", "email", ["com.apple.mail"]),
    source!("gmail", "Gmail", "email", []),
    source!("spark", "Spark", "email", ["com.readdle.SparkDesktop", "com.readdle.SparkDesktop.appstore", "com.readdle.SparkDesktop-setapp", "com.readdle.smartemail-macos"]),
    source!("meeting", "Meetings", "meeting", []),
    source!("voice-note", "Voice notes", "meeting", []),
    source!("apple-notes", "Apple Notes", "productivity", ["com.apple.Notes"]),
    source!("obsidian", "Obsidian", "productivity", ["md.obsidian"]),
    source!("notion", "Notion", "productivity", ["notion.id"]),
    source!("word", "Microsoft Word", "productivity", ["com.microsoft.Word"]),
    source!("pages", "Pages", "productivity", ["com.apple.iWork.Pages"]),
    source!("todoist", "Todoist", "productivity", ["com.todoist.mac.Todoist"]),
    source!("omnifocus", "OmniFocus", "productivity", ["com.omnigroup.OmniFocus4"]),
    source!("microsoft-todo", "Microsoft To Do", "productivity", ["com.microsoft.to-do-mac"]),
    source!("toggl", "Toggl Track", "productivity", ["com.toggl.daneel"]),
    source!("calendar", "Calendar", "productivity", ["com.apple.iCal"]),
    source!("fantastical", "Fantastical", "productivity", ["com.flexibits.fantastical2.mac"]),
];

/// Source id for a memory produced by `kind` (native app and web service of
/// the same product share one id, so there is a single switch per product).
fn service_id(kind: AdapterKind, bundle_id: Option<&str>, app: &str) -> Option<&'static str> {
    Some(match kind {
        AdapterKind::AppleMail => "apple-mail",
        AdapterKind::OutlookNative | AdapterKind::OutlookWeb => "outlook",
        AdapterKind::TeamsNative | AdapterKind::TeamsWeb => "teams",
        AdapterKind::SlackNative | AdapterKind::SlackWeb => "slack",
        AdapterKind::DiscordNative | AdapterKind::DiscordWeb => "discord",
        AdapterKind::WhatsAppNative | AdapterKind::WhatsAppWeb => "whatsapp",
        AdapterKind::SparkNative => "spark",
        AdapterKind::GmailWeb => "gmail",
        AdapterKind::LinkedInWeb => "linkedin",
        AdapterKind::SupportedNative => return supported_native_service(bundle_id, app),
    })
}

fn supported_native_service(bundle_id: Option<&str>, app: &str) -> Option<&'static str> {
    let bundle = bundle_id.unwrap_or("").to_lowercase();
    let by_bundle = match bundle.as_str() {
        "com.apple.notes" => Some("apple-notes"),
        "md.obsidian" => Some("obsidian"),
        "notion.id" => Some("notion"),
        "com.microsoft.word" => Some("word"),
        "com.apple.iwork.pages" => Some("pages"),
        "com.todoist.mac.todoist" => Some("todoist"),
        "com.omnigroup.omnifocus4" => Some("omnifocus"),
        "com.microsoft.to-do-mac" => Some("microsoft-todo"),
        "com.toggl.daneel" => Some("toggl"),
        "com.apple.ical" => Some("calendar"),
        "com.flexibits.fantastical2.mac" => Some("fantastical"),
        _ => None,
    };
    if by_bundle.is_some() || !bundle.is_empty() {
        return by_bundle;
    }
    // No bundle id (rare): fall back to the localized name.
    let name = lower(app);
    [
        ("notes", "apple-notes"),
        ("obsidian", "obsidian"),
        ("notion", "notion"),
        ("word", "word"),
        ("pages", "pages"),
        ("todoist", "todoist"),
        ("omnifocus", "omnifocus"),
        ("toggl", "toggl"),
        ("calendar", "calendar"),
        ("fantastical", "fantastical"),
    ]
    .into_iter()
    .find(|(needle, _)| name == *needle || name.contains(needle))
    .map(|(_, id)| id)
}

/// Normalized output of any capture adapter, fed into the shared memory
/// pipeline (dedup → store). The app identity is owned by the monitor.
pub struct NormalizedCapture {
    pub title: String,
    pub content: String,
    pub is_conversation: bool,
    /// Follow-up source id (see `FOLLOW_UP_SOURCES`) this capture came from.
    pub service: Option<&'static str>,
}

fn dedup_selections(apps: Vec<CaptureAppSelection>) -> Vec<CaptureAppSelection> {
    let mut seen = std::collections::HashSet::new();
    apps.into_iter()
        .filter(|app| seen.insert(app.bundle_id.to_lowercase()))
        .collect()
}

/// One concrete parser per Minimi-supported native app or web service.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AdapterKind {
    AppleMail,
    OutlookNative,
    TeamsNative,
    SlackNative,
    DiscordNative,
    WhatsAppNative,
    SparkNative,
    /// Notes / docs / tasks / calendar apps from Minimi's registry
    /// (Apple Notes, Obsidian, Notion, Word, Pages, Todoist, OmniFocus,
    /// MS To Do, Toggl, Calendar, Fantastical) — captured with shared
    /// static-text extraction.
    SupportedNative,
    GmailWeb,
    OutlookWeb,
    LinkedInWeb,
    SlackWeb,
    DiscordWeb,
    TeamsWeb,
    WhatsAppWeb,
}

// AX roles collected per adapter (role-restricted parsing, per Minimi).
const R_STATIC_TEXT: &str = "AXStaticText";
const R_HEADING: &str = "AXHeading";
const R_LINK: &str = "AXLink";
const R_LIST_ITEM: &str = "AXListItem";
const R_CELL: &str = "AXCell";
const R_ROW: &str = "AXRow";
const R_GROUP: &str = "AXGroup";

/// Browser display names (matched as a whole name or a "<name> …" prefix —
/// never as a substring, which made "arc" match "Search", "Archive", "Marc").
const BROWSER_NAMES: &[&str] = &[
    "google chrome",
    "safari",
    "microsoft edge",
    "arc",
    "brave browser",
    "firefox",
];
const BROWSER_BUNDLES: &[&str] = &[
    "com.google.chrome",
    "com.apple.safari",
    "com.microsoft.edgemac",
    "company.thebrowser.browser",
    "com.brave.browser",
    "org.mozilla.firefox",
];

pub fn is_browser(app: &str, bundle_id: Option<&str>) -> bool {
    let name = app.trim().to_lowercase();
    if BROWSER_NAMES
        .iter()
        .any(|browser| name == *browser || name.starts_with(&format!("{browser} ")))
    {
        return true;
    }
    bundle_id.is_some_and(|bundle| {
        let bundle = bundle.to_lowercase();
        BROWSER_BUNDLES
            .iter()
            .any(|id| bundle == *id || bundle.starts_with(&format!("{id}.")))
    })
}
const TITLE_MAX: usize = 90;

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Page-level privacy gate applied before reading any AX content. Supported
/// communication hosts remain capturable, but authentication, account,
/// security, payment, administration, settings, and private-browser surfaces
/// fail closed.
pub fn sensitive_context_reason(
    app: &str,
    bundle_id: Option<&str>,
    document_url: Option<&str>,
    window_title: Option<&str>,
) -> Option<Reason> {
    let title = window_title.unwrap_or("").trim().to_lowercase();
    if is_browser(app, bundle_id)
        && [
            "incognito",
            "private browsing",
            "private window",
            "inprivate",
        ]
        .iter()
        .any(|marker| title.contains(marker))
    {
        return Some(Reason::PrivateWindow);
    }

    let generic_surface = [
        "account settings",
        "preferences",
        "privacy settings",
        "security settings",
        "sign in",
        "sign-in",
        "two-factor authentication",
        "user settings",
    ];
    if generic_surface.iter().any(|surface| {
        title == *surface
            || title.starts_with(&format!("{surface} —"))
            || title.starts_with(&format!("{surface} -"))
    }) {
        return Some(Reason::SensitivePage);
    }

    let Some(url) = document_url else {
        return None;
    };
    let url = url.trim().to_lowercase();
    // Whole-domain blocklist adopted from Minimi: banking, auth providers,
    // password managers, HR/payroll, and health portals never get captured
    // regardless of the supported web app they would otherwise match.
    const SENSITIVE_WEB_DOMAINS: &[&str] = &[
        "chase.com",
        "wellsfargo.com",
        "bankofamerica.com",
        "citibank.com",
        "capitalone.com",
        "hdfcbank.com",
        "icicibank.com",
        "axisbank.com",
        "sbi.co.in",
        "paytm.com",
        "phonepe.com",
        "gpay.app",
        "razorpay.com",
        "stripe.com",
        "plaid.com",
        "accounts.google.com",
        "login.microsoftonline.com",
        "login.live.com",
        "login.yahoo.com",
        "appleid.apple.com",
        "okta.com",
        "auth0.com",
        "1password.com",
        "bitwarden.com",
        "lastpass.com",
        "dashlane.com",
        "keeper.io",
        "workday.com",
        "bamboohr.com",
        "gusto.com",
        "adp.com",
        "paylocity.com",
        "rippling.com",
        "mychart.com",
        "kaiserpermanente.org",
        "practo.com",
    ];
    if let Some(host) = normalize_domain(url.as_str()) {
        if SENSITIVE_WEB_DOMAINS
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
        {
            return Some(Reason::SensitivePage);
        }
    }
    if [
        "access_token=",
        "auth_token=",
        "password=",
        "secret=",
        "id_token=",
    ]
    .iter()
    .any(|parameter| url.contains(parameter))
    {
        return Some(Reason::SensitivePage);
    }
    let route = url.split_once("://").map(|(_, rest)| rest).unwrap_or(&url);
    let route = route
        .find(['/', '?', '#'])
        .map(|index| &route[index..])
        .unwrap_or("/");
    const SENSITIVE_ROUTES: &[&str] = &[
        "/account",
        "/admin",
        "/auth",
        "/authorize",
        "/billing",
        "/challenge",
        "/checkout",
        "/enterprise-grid",
        "/login",
        "/mail/options",
        "/manage",
        "/oauth",
        "/password",
        "/payment",
        "/preferences",
        "/premium/products",
        "/psettings",
        "/register",
        "/reset-password",
        "/reset_password",
        "/forgot-password",
        "/forgot_password",
        "/change-password",
        "/2fa",
        "/totp",
        "/otp",
        "/verify-email",
        "/verify_email",
        "/security",
        "/settings",
        "/signin",
        "/sign-in",
        "/signup",
        "/sso",
        "/wallet",
        "#settings",
        "%2fsettings",
        "%2fpassword",
        "%2fsecurity",
    ];
    SENSITIVE_ROUTES
        .iter()
        .any(|marker| route_has_marker(route, marker))
        .then_some(Reason::SensitivePage)
}

#[allow(dead_code)]
pub fn is_sensitive_capture_context(
    app: &str,
    bundle_id: Option<&str>,
    document_url: Option<&str>,
    window_title: Option<&str>,
) -> bool {
    sensitive_context_reason(app, bundle_id, document_url, window_title).is_some()
}

/// `marker` occurs in `route` and ends at a path/query boundary, so "/auth"
/// matches "/auth/login" but not "/author" and "/account" not "/accounting".
fn route_has_marker(route: &str, marker: &str) -> bool {
    route.match_indices(marker).any(|(start, _)| {
        route[start + marker.len()..]
            .chars()
            .next()
            .is_none_or(|next| matches!(next, '/' | '?' | '#' | '&' | '=' | '.' | '%'))
    })
}

/// Route the frontmost app to its adapter, or None when the app/website is
/// not on the capture allowlist.
fn classify(
    app: &str,
    bundle_id: Option<&str>,
    document_url: Option<&str>,
) -> Option<AdapterKind> {
    let bundle = bundle_id.unwrap_or("").to_lowercase();
    if is_browser(app, bundle_id) {
        // Browsers only capture on supported web apps — generic browsing is
        // not recorded.
        return web_adapter_for_url(document_url?);
    }
    match bundle.as_str() {
        "com.apple.mail" => Some(AdapterKind::AppleMail),
        "com.microsoft.outlook" => Some(AdapterKind::OutlookNative),
        "com.microsoft.teams" | "com.microsoft.teams2" => Some(AdapterKind::TeamsNative),
        "com.tinyspeck.slackmacgap" => Some(AdapterKind::SlackNative),
        "com.hnc.discord" => Some(AdapterKind::DiscordNative),
        "net.whatsapp.whatsapp" => Some(AdapterKind::WhatsAppNative),
        // Spark ships under several bundle ids (standalone / Setapp / App
        // Store) plus the legacy smartemail id.
        "com.readdle.smartemail-macos" | "com.readdle.smartemail-mac" => {
            Some(AdapterKind::SparkNative)
        }
        "com.readdle.sparkdesktop"
        | "com.readdle.sparkdesktop-setapp"
        | "com.readdle.sparkdesktop.appstore" => Some(AdapterKind::SparkNative),
        // Productivity surfaces adopted from Minimi's adapter registry.
        "com.apple.notes" | "md.obsidian" | "notion.id" | "com.microsoft.word"
        | "com.apple.iwork.pages" | "com.todoist.mac.todoist"
        | "com.omnigroup.omnifocus4" | "com.microsoft.to-do-mac"
        | "com.toggl.daneel" | "com.apple.ical" | "com.flexibits.fantastical2.mac" => {
            Some(AdapterKind::SupportedNative)
        }
        _ if bundle.is_empty() => classify_native_name(app),
        // The allowlist ends here: unknown bundle ids are not captured.
        _ => None,
    }
}

fn classify_native_name(app: &str) -> Option<AdapterKind> {
    let app = lower(app);
    if app == "mail" {
        Some(AdapterKind::AppleMail)
    } else if app.contains("outlook") {
        Some(AdapterKind::OutlookNative)
    } else if app.contains("teams") {
        Some(AdapterKind::TeamsNative)
    } else if app.contains("slack") {
        Some(AdapterKind::SlackNative)
    } else if app.contains("discord") {
        Some(AdapterKind::DiscordNative)
    } else if app.contains("whatsapp") {
        Some(AdapterKind::WhatsAppNative)
    } else if app.contains("spark") {
        Some(AdapterKind::SparkNative)
    } else if app.contains("notes")
        || app.contains("obsidian")
        || app.contains("notion")
        || app.contains("todoist")
        || app.contains("omnifocus")
        || app.contains("toggl")
        || app.contains("fantastical")
        || app == "calendar"
        || app == "pages"
        || app == "word"
    {
        Some(AdapterKind::SupportedNative)
    } else {
        None
    }
}

fn web_adapter_for_url(url: &str) -> Option<AdapterKind> {
    let lower = url.trim().to_lowercase();
    let after_scheme = lower.split_once("://").map(|(_, rest)| rest)?;
    let host = after_scheme
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or("");
    let matches = |allowed: &str| host == allowed || host.ends_with(&format!(".{allowed}"));
    if matches("mail.google.com") {
        Some(AdapterKind::GmailWeb)
    } else if matches("outlook.office.com")
        || matches("outlook.live.com")
        || matches("outlook.office365.com")
    {
        Some(AdapterKind::OutlookWeb)
    } else if matches("linkedin.com") {
        Some(AdapterKind::LinkedInWeb)
    } else if matches("slack.com") {
        Some(AdapterKind::SlackWeb)
    } else if matches("discord.com") {
        Some(AdapterKind::DiscordWeb)
    } else if matches("teams.microsoft.com") || matches("teams.live.com") {
        Some(AdapterKind::TeamsWeb)
    } else if matches("web.whatsapp.com") {
        Some(AdapterKind::WhatsAppWeb)
    } else {
        None
    }
}

/// Page-level gates, decided from already-read facts (no AX access): blocked
/// domain, sensitive page / private window, then the allowlist. `Err` carries
/// the stable reason the page is skipped. Capture is an allowlist: only apps
/// and websites with a dedicated adapter produce memories.
pub fn plan(
    app: &str,
    bundle_id: Option<&str>,
    document_url: Option<&str>,
    window_title: Option<&str>,
    blocked_domains: &[String],
) -> Result<AdapterKind, Reason> {
    if is_blocked_domain(document_url, blocked_domains) {
        return Err(Reason::BlockedDomain);
    }
    if let Some(reason) = sensitive_context_reason(app, bundle_id, document_url, window_title) {
        return Err(reason);
    }
    match classify(app, bundle_id, document_url) {
        Some(kind) => Ok(kind),
        // A browser that never reported a page address can't be matched to a
        // supported site; that is "couldn't read", not "unsupported".
        None if is_browser(app, bundle_id) && document_url.is_none() => Err(Reason::PageUnreadable),
        None => Err(Reason::NotSupported),
    }
}

/// Read the page URL for browsers (walks the AX tree: hundreds of IPC calls,
/// so native apps skip it).
pub fn read_document_url(pid: i32, app: &str, bundle_id: Option<&str>) -> Option<String> {
    if is_browser(app, bundle_id) {
        ax::document_url(pid)
    } else {
        None
    }
}

/// Run the adapter chosen by `plan`. None means nothing readable.
pub fn extract(
    kind: AdapterKind,
    pid: i32,
    app: &str,
    bundle_id: Option<&str>,
    window_title: Option<&str>,
) -> Option<NormalizedCapture> {
    let service = service_id(kind, bundle_id, app);
    let mut capture = match kind {
        AdapterKind::AppleMail => capture_apple_mail(pid, window_title),
        AdapterKind::OutlookNative => capture_outlook_native(pid, window_title),
        AdapterKind::TeamsNative => capture_teams_native(pid, window_title),
        AdapterKind::SlackNative => capture_slack_native(pid, window_title),
        AdapterKind::DiscordNative => capture_discord_native(pid, window_title),
        AdapterKind::WhatsAppNative => capture_whatsapp_native(pid, window_title),
        AdapterKind::SparkNative => capture_spark_native(pid, window_title),
        AdapterKind::SupportedNative => capture_supported_native(pid, window_title),
        AdapterKind::GmailWeb => capture_gmail_web(pid, window_title),
        AdapterKind::OutlookWeb => capture_outlook_web(pid, window_title),
        AdapterKind::LinkedInWeb => capture_linkedin_web(pid, window_title),
        AdapterKind::SlackWeb => capture_slack_web(pid, window_title),
        AdapterKind::DiscordWeb => capture_discord_web(pid, window_title),
        AdapterKind::TeamsWeb => capture_teams_web(pid, window_title),
        AdapterKind::WhatsAppWeb => capture_whatsapp_web(pid, window_title),
    }?;
    capture.service = service;
    Some(capture)
}

/// Decide + extract in one call (the pre-refactor entry point). The monitor
/// uses the split form so it can record why a page was skipped.
#[allow(dead_code)]
pub fn dispatch(
    pid: i32,
    app: &str,
    bundle_id: Option<&str>,
    window_title: Option<&str>,
    blocked_domains: &[String],
) -> Option<NormalizedCapture> {
    let document_url = read_document_url(pid, app, bundle_id);
    let kind = plan(app, bundle_id, document_url.as_deref(), window_title, blocked_domains).ok()?;
    extract(kind, pid, app, bundle_id, window_title)
}

// ---- per-app parsers ----

const EMAIL_ROLES: &[&str] = &[R_HEADING, R_STATIC_TEXT, R_LINK, R_ROW, R_CELL];
const CHAT_ROLES: &[&str] = &[R_HEADING, R_STATIC_TEXT, R_LIST_ITEM, R_ROW, R_CELL];
const FEED_ROLES: &[&str] = &[R_HEADING, R_STATIC_TEXT, R_LINK, R_LIST_ITEM, R_GROUP];

fn capture_apple_mail(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        EMAIL_ROLES,
        12,
        false,
        &["mailboxes", "favorites", "flagged", "junk"],
    )
}
fn capture_outlook_native(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        EMAIL_ROLES,
        12,
        false,
        &["new mail", "focused", "other", "sent items"],
    )
}
fn capture_spark_native(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        EMAIL_ROLES,
        12,
        false,
        &["smart inbox", "gatekeeper", "newsletters", "notifications"],
    )
}

/// Shared capture for the notes / docs / tasks / calendar apps adopted from
/// Minimi's registry. Their AX trees are text-heavy; keep the chrome filter
/// small and generic.
fn capture_supported_native(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        EMAIL_ROLES,
        24,
        false,
        &["new note", "new task", "search", "settings", "preferences"],
    )
}
fn capture_teams_native(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        CHAT_ROLES,
        16,
        true,
        &["activity", "chat", "teams", "calendar", "calls", "apps"],
    )
}
fn capture_slack_native(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        CHAT_ROLES,
        16,
        true,
        &["threads", "drafts & sent", "later", "more"],
    )
}
fn capture_discord_native(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        CHAT_ROLES,
        16,
        true,
        &["friends", "nitro", "shop", "add a server"],
    )
}
fn capture_whatsapp_native(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        CHAT_ROLES,
        16,
        true,
        &["new chat", "archived", "starred messages", "settings"],
    )
}
fn capture_gmail_web(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        EMAIL_ROLES,
        12,
        false,
        &["compose", "inbox", "starred", "snoozed", "sent", "drafts"],
    )
}
fn capture_outlook_web(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        EMAIL_ROLES,
        12,
        false,
        &["new mail", "focused", "other", "calendar", "people"],
    )
}
fn capture_linkedin_web(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        FEED_ROLES,
        20,
        true,
        &["home", "my network", "jobs", "messaging", "notifications"],
    )
}
fn capture_slack_web(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        CHAT_ROLES,
        16,
        true,
        &["threads", "drafts & sent", "later", "more"],
    )
}
fn capture_discord_web(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        CHAT_ROLES,
        16,
        true,
        &["friends", "nitro", "shop", "add a server"],
    )
}
fn capture_teams_web(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        CHAT_ROLES,
        16,
        true,
        &["activity", "chat", "teams", "calendar", "calls", "apps"],
    )
}
fn capture_whatsapp_web(pid: i32, title: Option<&str>) -> Option<NormalizedCapture> {
    capture_filtered(
        pid,
        title,
        CHAT_ROLES,
        16,
        true,
        &["new chat", "archived", "starred messages", "settings"],
    )
}

fn capture_filtered(
    pid: i32,
    window_title: Option<&str>,
    roles: &[&str],
    min_chars: usize,
    is_conversation: bool,
    chrome: &[&str],
) -> Option<NormalizedCapture> {
    let raw = ax::collect_role_text(pid, roles);
    build_filtered(window_title, raw, min_chars, is_conversation, chrome)
}

// ---- normalization ----

/// Build a `NormalizedCapture` from raw extracted text: dedupe consecutive
/// lines, derive a title, and drop captures below a minimum length.
fn build_filtered(
    window_title: Option<&str>,
    raw: String,
    min_chars: usize,
    is_conversation: bool,
    chrome: &[&str],
) -> Option<NormalizedCapture> {
    let content = normalize(&raw, chrome);
    if content.chars().count() < min_chars {
        return None;
    }
    let title = window_title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| first_line(&content));
    Some(NormalizedCapture {
        title,
        content,
        is_conversation,
        service: None,
    })
}

/// Trim each line, drop blanks, and collapse consecutive duplicates.
fn normalize(raw: &str, chrome: &[&str]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut last: Option<String> = None;
    for line in raw.lines() {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if chrome.iter().any(|item| lower == *item) {
            continue;
        }
        if last.as_deref() == Some(line.as_str()) {
            continue;
        }
        last = Some(line.clone());
        out.push(line);
    }
    out.join("\n")
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim().to_string();
    if line.chars().count() > TITLE_MAX {
        line.chars().take(TITLE_MAX).collect::<String>() + "…"
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_reports_a_stable_reason_for_every_skip() {
        let none: Vec<String> = vec![];
        let blocked = vec!["slack.com".to_string()];
        let chrome = Some("com.google.Chrome");
        assert_eq!(
            plan("Google Chrome", chrome, Some("https://acme.slack.com/client"), Some("x"), &blocked),
            Err(Reason::BlockedDomain)
        );
        assert_eq!(
            plan("Google Chrome", chrome, Some("https://chase.com/"), Some("x"), &none),
            Err(Reason::SensitivePage)
        );
        assert_eq!(
            plan("Google Chrome", chrome, Some("https://acme.slack.com/login"), Some("x"), &none),
            Err(Reason::SensitivePage)
        );
        assert_eq!(
            plan("Google Chrome", chrome, Some("https://acme.slack.com/"), Some("Foo — Incognito"), &none),
            Err(Reason::PrivateWindow)
        );
        assert_eq!(
            plan("Google Chrome", chrome, Some("https://example.com/"), Some("x"), &none),
            Err(Reason::NotSupported)
        );
        assert_eq!(
            plan("Google Chrome", chrome, None, Some("x"), &none),
            Err(Reason::PageUnreadable)
        );
        assert_eq!(
            plan("Google Chrome", chrome, Some("https://acme.slack.com/client"), Some("x"), &none),
            Ok(AdapterKind::SlackWeb)
        );
        assert_eq!(
            plan("Slack", Some("com.tinyspeck.slackmacgap"), None, None, &none),
            Ok(AdapterKind::SlackNative)
        );
        assert_eq!(plan("Preview", Some("com.apple.Preview"), None, None, &none), Err(Reason::NotSupported));
    }

    #[test]
    fn privacy_gates_win_over_the_allowlist() {
        // A supported host behind a private window or sensitive route never plans.
        let none: Vec<String> = vec![];
        assert!(plan("Safari", Some("com.apple.Safari"), Some("https://mail.google.com/mail/options"), Some("Inbox"), &none).is_err());
        assert!(plan("Safari", Some("com.apple.Safari"), None, Some("Private Browsing"), &none) == Err(Reason::PrivateWindow));
    }

    #[test]
    fn follow_up_sources_are_unique_and_cover_every_adapter() {
        let mut ids: Vec<_> = FOLLOW_UP_SOURCES.iter().map(|source| source.id).collect();
        ids.sort_unstable();
        let total = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), total, "duplicate source ids");
        // Every id an adapter can emit must be a listed, switchable source.
        for kind in [
            AdapterKind::AppleMail, AdapterKind::OutlookNative, AdapterKind::OutlookWeb,
            AdapterKind::TeamsNative, AdapterKind::TeamsWeb, AdapterKind::SlackNative,
            AdapterKind::SlackWeb, AdapterKind::DiscordNative, AdapterKind::DiscordWeb,
            AdapterKind::WhatsAppNative, AdapterKind::WhatsAppWeb, AdapterKind::SparkNative,
            AdapterKind::GmailWeb, AdapterKind::LinkedInWeb,
        ] {
            let id = service_id(kind, None, "").expect("adapter has a source id");
            assert!(FOLLOW_UP_SOURCES.iter().any(|source| source.id == id), "{id} not listed");
        }
        for bundle in [
            "com.apple.notes", "md.obsidian", "notion.id", "com.microsoft.word",
            "com.apple.iwork.pages", "com.todoist.mac.todoist", "com.omnigroup.omnifocus4",
            "com.microsoft.to-do-mac", "com.toggl.daneel", "com.apple.ical",
            "com.flexibits.fantastical2.mac",
        ] {
            let id = supported_native_service(Some(bundle), "").expect("bundle maps");
            assert!(FOLLOW_UP_SOURCES.iter().any(|source| source.id == id), "{id} not listed");
        }
        assert_eq!(supported_native_service(None, "Notes"), Some("apple-notes"));
    }

    #[test]
    fn browser_detection_is_not_a_substring_match() {
        assert!(is_browser("Arc", None));
        assert!(is_browser("Google Chrome", Some("com.google.Chrome")));
        assert!(is_browser("Chrome Canary", Some("com.google.Chrome.canary")));
        assert!(!is_browser("Search", None));
        assert!(!is_browser("Archive Utility", None));
        assert!(!is_browser("Marc's Notes", None));
    }

    #[test]
    fn user_info_cannot_hide_a_blocked_host() {
        assert_eq!(
            normalize_domain("https://user:pw@chase.com:8443/x").as_deref(),
            Some("chase.com")
        );
        assert!(is_blocked_domain(
            Some("https://a@bank.example.com/"),
            &["example.com".to_string()]
        ));
    }

    #[test]
    fn sensitive_routes_match_whole_segments_only() {
        assert!(route_has_marker("/auth/login", "/auth"));
        assert!(route_has_marker("/settings?tab=x", "/settings"));
        assert!(!route_has_marker("/author/jane", "/auth"));
        assert!(!route_has_marker("/accounting/q3", "/account"));
    }

    #[test]
    fn exclusions_default_to_capture_everywhere_and_deduplicate() {
        assert!(effective_excluded_apps(None).is_empty());
        assert!(effective_excluded_apps(Some("not-json")).is_empty());
        assert_eq!(
            effective_excluded_apps(Some("[{\"name\":\"Slack\",\"bundleId\":\"com.tinyspeck.slackmacgap\"},{\"name\":\"Slack\",\"bundleId\":\"COM.TINYSPECK.SLACKMACGAP\"}]")),
            vec![CaptureAppSelection {
                name: "Slack".into(),
                bundle_id: "com.tinyspeck.slackmacgap".into(),
            }]
        );
    }

    #[test]
    fn web_gate_uses_hostname_boundaries() {
        assert_eq!(
            web_adapter_for_url("https://mail.google.com/mail/u/0/"),
            Some(AdapterKind::GmailWeb)
        );
        assert_eq!(
            web_adapter_for_url("https://app.slack.com/client/T/C"),
            Some(AdapterKind::SlackWeb)
        );
        assert_eq!(
            web_adapter_for_url("https://web.whatsapp.com/"),
            Some(AdapterKind::WhatsAppWeb)
        );
        assert_eq!(
            web_adapter_for_url("https://example.com/linkedin.com/report"),
            None
        );
        assert_eq!(
            web_adapter_for_url("https://linkedin.com.evil.example/"),
            None
        );
    }

    #[test]
    fn sensitive_pages_on_supported_hosts_fail_closed() {
        let chrome = Some("com.google.chrome");
        assert!(is_sensitive_capture_context(
            "Google Chrome",
            chrome,
            Some("https://mail.google.com/mail/u/0/#settings/accounts"),
            Some("Settings - Gmail"),
        ));
        assert!(is_sensitive_capture_context(
            "Google Chrome",
            chrome,
            Some("https://www.linkedin.com/psettings/two-step-verification"),
            Some("LinkedIn"),
        ));
        assert!(is_sensitive_capture_context(
            "Google Chrome",
            chrome,
            Some("https://app.slack.com/account/settings?auth_token=secret"),
            Some("Slack"),
        ));
        assert!(is_sensitive_capture_context(
            "Google Chrome",
            chrome,
            Some("https://mail.google.com/mail/u/0/#inbox"),
            Some("Inbox - Incognito"),
        ));
        assert!(!is_sensitive_capture_context(
            "Google Chrome",
            chrome,
            Some("https://mail.google.com/mail/u/0/#inbox/abc"),
            Some("Project update - Gmail"),
        ));
    }

    #[test]
    fn native_settings_are_blocked_without_scanning_message_keywords() {
        assert!(is_sensitive_capture_context(
            "Slack",
            Some("com.tinyspeck.slackmacgap"),
            None,
            Some("User Settings - Slack"),
        ));
        assert!(!is_sensitive_capture_context(
            "Slack",
            Some("com.tinyspeck.slackmacgap"),
            None,
            Some("security review discussion - Slack"),
        ));
    }

    #[test]
    fn blocked_domains_cover_subdomains_but_not_lookalikes() {
        let blocked = effective_blocked_domains(Some("[\"https://Example.com/private\"]"));
        assert_eq!(blocked, vec!["example.com"]);
        assert!(is_blocked_domain(
            Some("https://mail.example.com/inbox"),
            &blocked
        ));
        assert!(!is_blocked_domain(
            Some("https://example.com.evil.test"),
            &blocked
        ));
    }

    #[test]
    fn app_exclusions_use_stable_bundle_ids_case_insensitively() {
        let excluded = vec![CaptureAppSelection {
            name: "Slack".into(),
            bundle_id: "com.tinyspeck.slackmacgap".into(),
        }];
        assert!(is_excluded_identity(
            Some("COM.TINYSPECK.SLACKMACGAP"),
            &excluded
        ));
        assert!(!is_excluded_identity(Some("com.example.slack"), &excluded));
    }

    #[test]
    fn routes_each_native_bundle_to_its_adapter() {
        let cases = [
            ("Mail", "com.apple.mail", AdapterKind::AppleMail),
            (
                "Outlook",
                "com.microsoft.outlook",
                AdapterKind::OutlookNative,
            ),
            ("Teams", "com.microsoft.teams2", AdapterKind::TeamsNative),
            (
                "Slack",
                "com.tinyspeck.slackmacgap",
                AdapterKind::SlackNative,
            ),
            ("Discord", "com.hnc.discord", AdapterKind::DiscordNative),
            (
                "WhatsApp",
                "net.whatsapp.whatsapp",
                AdapterKind::WhatsAppNative,
            ),
            (
                "Spark",
                "com.readdle.smartemail-macos",
                AdapterKind::SparkNative,
            ),
            (
                "Spark",
                "com.readdle.SparkDesktop",
                AdapterKind::SparkNative,
            ),
            // Minimi-registry productivity apps.
            ("Notes", "com.apple.Notes", AdapterKind::SupportedNative),
            ("Obsidian", "md.obsidian", AdapterKind::SupportedNative),
            ("Notion", "notion.id", AdapterKind::SupportedNative),
            ("Todoist", "com.todoist.mac.Todoist", AdapterKind::SupportedNative),
            (
                "Fantastical",
                "com.flexibits.fantastical2.mac",
                AdapterKind::SupportedNative,
            ),
        ];
        for (name, bundle, expected) in cases {
            assert_eq!(classify(name, Some(bundle), None), Some(expected));
        }
        // Unknown bundles are not on the allowlist.
        assert_eq!(classify("Terminal", Some("com.apple.Terminal"), None), None);
    }

    #[test]
    fn routes_each_supported_web_service() {
        let cases = [
            ("https://mail.google.com/", AdapterKind::GmailWeb),
            ("https://outlook.office.com/mail/", AdapterKind::OutlookWeb),
            ("https://outlook.office365.com/mail/", AdapterKind::OutlookWeb),
            ("https://www.linkedin.com/feed/", AdapterKind::LinkedInWeb),
            ("https://app.slack.com/", AdapterKind::SlackWeb),
            ("https://discord.com/channels/x/y", AdapterKind::DiscordWeb),
            ("https://teams.microsoft.com/", AdapterKind::TeamsWeb),
            ("https://teams.live.com/", AdapterKind::TeamsWeb),
            ("https://web.whatsapp.com/", AdapterKind::WhatsAppWeb),
        ];
        for (url, expected) in cases {
            assert_eq!(
                classify("Google Chrome", Some("com.google.chrome"), Some(url)),
                Some(expected)
            );
        }
        // Generic browsing is not captured.
        assert_eq!(
            classify(
                "Google Chrome",
                Some("com.google.chrome"),
                Some("https://example.com/article")
            ),
            None
        );
    }

    #[test]
    fn sensitive_web_domains_fail_closed() {
        let chrome = Some("com.google.chrome");
        for url in [
            "https://accounts.google.com/o/oauth2/auth",
            "https://www.paytm.com/wallet",
            "https://app.razorpay.com/dashboard",
        ] {
            assert!(is_sensitive_capture_context(
                "Google Chrome",
                chrome,
                Some(url),
                Some("Page"),
            ));
        }
    }
}
