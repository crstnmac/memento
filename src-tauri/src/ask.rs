//! "Ask" over a date range. A question like "what did I do last Tuesday?" is
//! scoped to that day: retrieval (search, recent memories, meetings,
//! follow-ups) is restricted to the range, and the model is told exactly which
//! range was used. If the range holds nothing we say so without calling the
//! model.

use rusqlite::params;

use crate::db::{self, Db};
use crate::llm;
use crate::localtime::{self as lt, Offset, DAY_MS};
use crate::state::MonitorState;

#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    /// UTC instants, `[start, end)`.
    pub start: i64,
    pub end: i64,
    /// Human description including concrete dates, e.g. "last Tuesday (Tue 29 Sep)".
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub scope: Option<Scope>,
    /// The question with the date phrase removed.
    pub topic: String,
}

const FILLER: &[&str] = &[
    "a", "about", "all", "am", "an", "and", "any", "anything", "are", "at", "be", "did", "do",
    "does", "doing", "done", "for", "from", "had", "has", "have", "how", "i", "in", "is", "it",
    "me", "my", "of", "on", "show", "so", "tell", "that", "the", "there", "to", "was", "were",
    "what", "when", "where", "which", "who", "with", "work", "worked", "working", "up", "us",
    "we", "you", "happened", "summarize", "summary", "recap",
];

const LEADING_PREPOSITIONS: &[&str] = &["on", "in", "during", "from", "for"];

fn norm(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

fn weekday_index(t: &str) -> Option<i64> {
    Some(match t {
        "monday" | "mon" => 0,
        "tuesday" | "tue" | "tues" => 1,
        "wednesday" | "wed" => 2,
        "thursday" | "thu" | "thur" | "thurs" => 3,
        "friday" | "fri" => 4,
        "saturday" | "sat" => 5,
        "sunday" | "sun" => 6,
        _ => return None,
    })
}

fn month_index(t: &str) -> Option<u32> {
    Some(match t {
        "january" | "jan" => 1,
        "february" | "feb" => 2,
        "march" | "mar" => 3,
        "april" | "apr" => 4,
        "may" => 5,
        "june" | "jun" => 6,
        "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" | "sept" => 9,
        "october" | "oct" => 10,
        "november" | "nov" => 11,
        "december" | "dec" => 12,
        _ => return None,
    })
}

fn day_of_month(t: &str) -> Option<u32> {
    let digits = t
        .strip_suffix("st")
        .or_else(|| t.strip_suffix("nd"))
        .or_else(|| t.strip_suffix("rd"))
        .or_else(|| t.strip_suffix("th"))
        .unwrap_or(t);
    if digits.is_empty() || digits.len() > 2 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let d: u32 = digits.parse().ok()?;
    (1..=31).contains(&d).then_some(d)
}

fn year_token(t: &str) -> Option<i64> {
    if t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()) {
        let y: i64 = t.parse().ok()?;
        return (1900..=2200).contains(&y).then_some(y);
    }
    None
}

fn count_word(t: &str) -> Option<i64> {
    if let Ok(n) = t.parse::<i64>() {
        return Some(n);
    }
    Some(match t {
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "twelve" => 12,
        "fourteen" => 14,
        _ => return None,
    })
}

struct Ctx<'a> {
    off: Offset<'a>,
    today: i64,
}

impl Ctx<'_> {
    fn day_scope(&self, day: i64, label: String) -> Scope {
        let (start, end) = lt::day_bounds(day, self.off);
        Scope { start, end, label }
    }

    fn span_scope(&self, first: i64, last: i64, label: String) -> Scope {
        Scope {
            start: lt::start_of_day(first, self.off),
            end: lt::start_of_day(last + 1, self.off),
            label,
        }
    }

    fn part_scope(&self, day: i64, part: &str, label: String) -> Scope {
        let (from, to) = match part {
            "morning" => (0, 12),
            "afternoon" => (12, 18),
            _ => (18, 24),
        };
        let start = lt::at_hour(day, from, self.off);
        let end = if to == 24 {
            lt::start_of_day(day + 1, self.off)
        } else {
            lt::at_hour(day, to, self.off)
        };
        Scope { start, end, label }
    }
}

fn is_part_of_day(t: &str) -> bool {
    matches!(t, "morning" | "afternoon" | "evening")
}

/// Try every supported phrase starting at token `i`. Returns the number of
/// tokens consumed and the resolved scope.
fn match_at(t: &[String], i: usize, cx: &Ctx) -> Option<(usize, Scope)> {
    let get = |k: usize| t.get(i + k).map(String::as_str);
    let today = cx.today;
    let first = t[i].as_str();

    match first {
        "day" if get(1) == Some("before") && get(2) == Some("yesterday") => {
            let d = today - 2;
            return Some((3, cx.day_scope(d, format!("the day before yesterday ({})", lt::short_label(d)))));
        }
        "yesterday" => {
            let d = today - 1;
            if let Some(part) = get(1).filter(|p| is_part_of_day(p)) {
                return Some((
                    2,
                    cx.part_scope(d, part, format!("yesterday {part} ({})", lt::short_label(d))),
                ));
            }
            return Some((1, cx.day_scope(d, format!("yesterday ({})", lt::short_label(d)))));
        }
        "today" => {
            if let Some(part) = get(1).filter(|p| is_part_of_day(p)) {
                return Some((
                    2,
                    cx.part_scope(today, part, format!("today's {part} ({})", lt::short_label(today))),
                ));
            }
            return Some((1, cx.day_scope(today, format!("today ({})", lt::short_label(today)))));
        }
        "tonight" => {
            return Some((
                1,
                cx.part_scope(today, "evening", format!("this evening ({})", lt::short_label(today))),
            ));
        }
        "this" => {
            match get(1) {
                Some(part) if is_part_of_day(part) => {
                    return Some((
                        2,
                        cx.part_scope(today, part, format!("this {part} ({})", lt::short_label(today))),
                    ));
                }
                Some("week") => {
                    let ws = lt::week_start_day(today);
                    return Some((
                        2,
                        cx.span_scope(
                            ws,
                            ws + 6,
                            format!("this week ({} – {})", lt::short_label(ws), lt::short_label(ws + 6)),
                        ),
                    ));
                }
                Some("month") => {
                    let (y, m, _) = lt::civil_from_days(today);
                    return Some((2, month_scope(cx, y, m, "this month")));
                }
                _ => return None,
            };
        }
        "last" | "past" => {
            let second = get(1)?;
            if first == "last" {
                if second == "week" {
                    let ws = lt::week_start_day(today) - 7;
                    return Some((
                        2,
                        cx.span_scope(
                            ws,
                            ws + 6,
                            format!("last week ({} – {})", lt::short_label(ws), lt::short_label(ws + 6)),
                        ),
                    ));
                }
                if second == "month" {
                    let (y, m, _) = lt::civil_from_days(today);
                    let (y, m) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
                    return Some((2, month_scope(cx, y, m, "last month")));
                }
                if let Some(w) = weekday_index(second) {
                    let mut diff = (lt::weekday_mon0(today) - w).rem_euclid(7);
                    if diff == 0 {
                        diff = 7;
                    }
                    let d = today - diff;
                    return Some((
                        2,
                        cx.day_scope(
                            d,
                            format!("last {} ({})", lt::WEEKDAY_NAMES[w as usize], lt::short_label(d)),
                        ),
                    ));
                }
            }
            if first == "past" && matches!(second, "week") {
                return Some((2, trailing_days(cx, 7, "the past week".into())));
            }
            // "past 3 days" / "last two weeks"
            let n = count_word(second)?;
            let unit = get(2)?;
            let days = match unit {
                "day" | "days" => n,
                "week" | "weeks" => n.checked_mul(7)?,
                _ => return None,
            };
            if !(1..=366).contains(&days) {
                return None;
            }
            let label = format!("the {first} {second} {unit}");
            return Some((3, trailing_days(cx, days, label)));
        }
        _ => {}
    }

    // 2026-03-03
    if let Some(day) = lt::parse_ymd(first).filter(|_| first.matches('-').count() == 2) {
        return Some((1, cx.day_scope(day, format!("{} ({})", lt::format_ymd(day), lt::short_label(day)))));
    }

    // "March 3", "March 3rd, 2025"
    if let Some(month) = month_index(first) {
        let d = day_of_month(get(1)?)?;
        let (year, used) = match get(2).and_then(year_token) {
            Some(y) => (Some(y), 3),
            None => (None, 2),
        };
        let day = resolve_date(cx, year, month, d)?;
        return Some((used, cx.day_scope(day, format!("{} ({})", lt::format_ymd(day), lt::short_label(day)))));
    }

    // "3 March", "3rd of March 2025"
    if let Some(d) = day_of_month(first) {
        let mut k = 1;
        if get(k) == Some("of") {
            k += 1;
        }
        let month = month_index(get(k)?)?;
        k += 1;
        let (year, used) = match get(k).and_then(year_token) {
            Some(y) => (Some(y), k + 1),
            None => (None, k),
        };
        let day = resolve_date(cx, year, month, d)?;
        return Some((used, cx.day_scope(day, format!("{} ({})", lt::format_ymd(day), lt::short_label(day)))));
    }

    None
}

/// Without an explicit year, the most recent occurrence that is not in the future.
fn resolve_date(cx: &Ctx, year: Option<i64>, month: u32, d: u32) -> Option<i64> {
    match year {
        Some(y) => lt::is_valid_civil(y, month, d).then(|| lt::days_from_civil(y, month, d)),
        None => {
            let (y, _, _) = lt::civil_from_days(cx.today);
            if lt::is_valid_civil(y, month, d) && lt::days_from_civil(y, month, d) <= cx.today {
                return Some(lt::days_from_civil(y, month, d));
            }
            lt::is_valid_civil(y - 1, month, d).then(|| lt::days_from_civil(y - 1, month, d))
        }
    }
}

fn month_scope(cx: &Ctx, y: i64, m: u32, name: &str) -> Scope {
    let first = lt::days_from_civil(y, m, 1);
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let next = lt::days_from_civil(ny, nm, 1);
    cx.span_scope(
        first,
        next - 1,
        format!("{name} ({} {y})", lt::MONTH_NAMES[m as usize - 1]),
    )
}

fn trailing_days(cx: &Ctx, days: i64, label: String) -> Scope {
    let first = cx.today - (days - 1);
    cx.span_scope(
        first,
        cx.today,
        format!("{label} ({} – today)", lt::short_label(first)),
    )
}

/// Find a date scope in `question` and strip its phrase from the remaining topic.
pub fn parse_question(question: &str, now: i64, off: Offset) -> Parsed {
    let cx = Ctx {
        off,
        today: lt::day_number(now, off),
    };
    let original: Vec<&str> = question.split_whitespace().collect();
    let tokens: Vec<String> = original.iter().map(|t| norm(t)).collect();
    for i in 0..tokens.len() {
        if tokens[i].is_empty() {
            continue;
        }
        if let Some((len, scope)) = match_at(&tokens, i, &cx) {
            let mut from = i;
            if from > 0 && LEADING_PREPOSITIONS.contains(&tokens[from - 1].as_str()) {
                from -= 1;
            }
            let topic = original
                .iter()
                .enumerate()
                .filter(|(k, _)| *k < from || *k >= i + len)
                .map(|(_, t)| *t)
                .collect::<Vec<_>>()
                .join(" ");
            return Parsed {
                scope: Some(scope),
                topic,
            };
        }
    }
    Parsed {
        scope: None,
        topic: question.trim().to_string(),
    }
}

/// Significant search words from a topic (empty for "what did I do?").
fn keywords(topic: &str) -> Vec<String> {
    topic
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2 && !FILLER.contains(&w.as_str()))
        .collect()
}

fn fts_expression(words: &[String]) -> Option<String> {
    let last = words.len().checked_sub(1)?;
    Some(
        words
            .iter()
            .enumerate()
            .map(|(i, w)| if i == last { format!("\"{w}\"*") } else { format!("\"{w}\"") })
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

fn fmt_local(ms: i64, off: Offset) -> String {
    let local = ms + off(ms);
    let day = local.div_euclid(DAY_MS);
    let minutes = local.rem_euclid(DAY_MS) / 60_000;
    format!("{} {:02}:{:02}", lt::short_label(day), minutes / 60, minutes % 60)
}

fn snippet(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(max).collect()
}

fn memory_line(m: &db::MemoryRow, max: usize, off: Offset) -> String {
    format!(
        "- [{}] {} ({}) :: {}\n",
        m.app.as_deref().unwrap_or(m.source.as_str()),
        m.title,
        fmt_local(m.created_at, off),
        snippet(&m.content, max)
    )
}

fn memories_where(
    db: &Db,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<db::MemoryRow>, String> {
    let mut stmt = db.conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(args, |row| {
            Ok(db::MemoryRow {
                id: row.get(0)?,
                source: row.get(1)?,
                app: row.get(2)?,
                title: row.get(3)?,
                content: row.get(4)?,
                created_at: row.get(5)?,
                is_meeting: row.get::<_, i64>(6)? != 0,
                open_loop: row.get::<_, i64>(7)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub struct Prompt {
    pub user: String,
    /// False when a scoped question found nothing at all in its range.
    pub has_data: bool,
    /// The same findings as plain text for the user, used when no language
    /// model is connected (or it fails): what Memento remembers, not a guess.
    pub local: String,
}

/// One memory as a short, readable bullet.
fn local_line(m: &db::MemoryRow, off: Offset) -> String {
    format!(
        "• {} · {} — {}\n  {}\n",
        fmt_local(m.created_at, off),
        m.app.as_deref().unwrap_or(m.source.as_str()),
        m.title,
        snippet(&m.content, 140)
    )
}

/// Gather everything the model may use. Pure DB work, safe to run under the
/// lock; the caller releases the lock before the model call.
pub fn build_prompt(
    db: &Db,
    question: &str,
    parsed: &Parsed,
    screen: Option<&str>,
    now: i64,
    off: Offset,
) -> Result<Prompt, String> {
    let mut user = String::new();
    let mut local = String::new();
    let mut has_data = true;

    let Some(scope) = &parsed.scope else {
        // Unscoped: screen context, relevant, recent, open follow-ups.
        if let Some(screen) = screen {
            user.push_str(screen);
        }
        // Search by the significant words, any of which may match: searching
        // the whole sentence required every word ("what", "did", "ask"…) to
        // appear in one memory, so ordinary questions found nothing.
        let by_keywords = fts_expression(&keywords(&parsed.topic))
            .and_then(|expr| {
                memories_where(
                    db,
                    "SELECT m.id, m.source, m.app, m.title, m.content, m.created_at, m.is_meeting, m.open_loop
                     FROM memories_fts JOIN memories m ON m.id = memories_fts.rowid
                     WHERE memories_fts MATCH ?1
                     ORDER BY bm25(memories_fts, 1.0, 1.5) LIMIT 5",
                    &[&expr],
                )
                .ok()
            })
            .unwrap_or_default();
        let rows = if by_keywords.is_empty() {
            db.search_text(question, 5).unwrap_or_default()
        } else {
            by_keywords
        };
        if !rows.is_empty() {
            user.push_str("\nRelevant memories:\n");
            for row in &rows {
                user.push_str(&memory_line(row, 400, off));
            }
        }
        let recent = db.list_memories(5, 0)?;
        if !recent.is_empty() {
            user.push_str("\nMost recent memories:\n");
            for row in &recent {
                user.push_str(&memory_line(row, 300, off));
            }
        }
        let open = db.list_action_items("open")?;
        if !open.is_empty() {
            user.push_str("\nOpen follow-ups:\n");
            for item in open.iter().take(5) {
                user.push_str(&format!(
                    "- {}{}\n",
                    item.content,
                    if item.is_urgent { " (urgent)" } else { "" }
                ));
            }
        }
        // Plain-text answer: best matches, else what was most recent.
        if !rows.is_empty() {
            local.push_str("Best matches:\n");
            rows.iter().for_each(|row| local.push_str(&local_line(row, off)));
        } else if !recent.is_empty() {
            local.push_str("Nothing matched those words, so here is what you captured most recently:\n");
            recent.iter().for_each(|row| local.push_str(&local_line(row, off)));
        }
        if !open.is_empty() {
            local.push_str("\nOpen follow-ups:\n");
            for item in open.iter().take(5) {
                local.push_str(&format!("• {}{}\n", item.content, if item.is_urgent { " (urgent)" } else { "" }));
            }
        }
        user.push_str(&format!("\nQuestion: {question}"));
        return Ok(Prompt { user, has_data, local });
    };

    let (start, end) = (scope.start, scope.end);
    let total: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE created_at >= ?1 AND created_at < ?2",
            params![start, end],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    let mut stmt = db
        .conn
        .prepare(
            "SELECT COALESCE(app, source), COUNT(*) FROM memories
             WHERE created_at >= ?1 AND created_at < ?2
             GROUP BY 1 ORDER BY 2 DESC LIMIT 6",
        )
        .map_err(|e| e.to_string())?;
    let apps = stmt
        .query_map(params![start, end], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    let mut stmt = db
        .conn
        .prepare(
            "SELECT id, title, kind, started_at, duration_ms FROM conversations
             WHERE started_at >= ?1 AND started_at < ?2 ORDER BY started_at LIMIT 8",
        )
        .map_err(|e| e.to_string())?;
    let convs = stmt
        .query_map(params![start, end], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    let mut stmt = db
        .conn
        .prepare(
            "SELECT content, status, is_urgent, created_at FROM action_items
             WHERE (created_at >= ?1 AND created_at < ?2)
                OR (completed_at >= ?1 AND completed_at < ?2)
             ORDER BY created_at DESC LIMIT 12",
        )
        .map_err(|e| e.to_string())?;
    let follow_ups = stmt
        .query_map(params![start, end], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? != 0,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    if total == 0 && convs.is_empty() && follow_ups.is_empty() {
        has_data = false;
    }

    user.push_str(&format!(
        "Date range for this question: {} — from {} to {} (local time). Today is {}.\nOnly the memories below fall inside that range; nothing outside it was searched.\n",
        scope.label,
        fmt_local(start, off),
        fmt_local(end - 1, off),
        lt::short_label(lt::day_number(now, off)),
    ));
    if let Some(screen) = screen {
        user.push_str(&format!("\n{screen}"));
    }
    if total > 0 {
        user.push_str(&format!("\nMemories captured in range: {total}"));
        if !apps.is_empty() {
            user.push_str(" (");
            user.push_str(
                &apps
                    .iter()
                    .map(|(a, n)| format!("{a} {n}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            user.push(')');
        }
        user.push('\n');
    }

    let mut seen: Vec<i64> = Vec::new();
    let mut relevant_rows: Vec<db::MemoryRow> = Vec::new();
    let words = keywords(&parsed.topic);
    if let Some(expr) = fts_expression(&words) {
        let rows = memories_where(
            db,
            "SELECT m.id, m.source, m.app, m.title, m.content, m.created_at, m.is_meeting, m.open_loop
             FROM memories_fts JOIN memories m ON m.id = memories_fts.rowid
             WHERE memories_fts MATCH ?1 AND m.created_at >= ?2 AND m.created_at < ?3
             ORDER BY bm25(memories_fts, 1.0, 1.5) LIMIT 8",
            &[&expr, &start, &end],
        )
        // A malformed match expression must not sink the whole question.
        .unwrap_or_default();
        if !rows.is_empty() {
            user.push_str("\nRelevant memories in range:\n");
            for row in &rows {
                seen.push(row.id);
                user.push_str(&memory_line(row, 400, off));
            }
            relevant_rows = rows;
        }
    }
    let recent = memories_where(
        db,
        "SELECT id, source, app, title, content, created_at, is_meeting, open_loop
         FROM memories WHERE created_at >= ?1 AND created_at < ?2
         ORDER BY created_at DESC LIMIT 14",
        &[&start, &end],
    )?;
    let recent: Vec<_> = recent.into_iter().filter(|m| !seen.contains(&m.id)).take(8).collect();
    if !recent.is_empty() {
        user.push_str("\nMost recent memories in range:\n");
        for row in &recent {
            user.push_str(&memory_line(row, 300, off));
        }
    }
    if !convs.is_empty() {
        user.push_str("\nMeetings and voice notes in range:\n");
        for (_, title, kind, started, duration) in &convs {
            let minutes = duration.map(|d| format!(", {} min", (d / 60_000).max(1))).unwrap_or_default();
            user.push_str(&format!("- {title} ({kind}, {}{minutes})\n", fmt_local(*started, off)));
        }
    }
    if !follow_ups.is_empty() {
        user.push_str("\nFollow-ups created or resolved in range:\n");
        for (content, status, urgent) in &follow_ups {
            user.push_str(&format!(
                "- {content} [{status}]{}\n",
                if *urgent { " (urgent)" } else { "" }
            ));
        }
    }
    // Plain-text answer for the same range.
    local.push_str(&format!(
        "{total} {} from {}",
        if total == 1 { "memory" } else { "memories" },
        scope.label
    ));
    if !apps.is_empty() {
        let list = apps.iter().map(|(a, n)| format!("{a} {n}")).collect::<Vec<_>>().join(", ");
        local.push_str(&format!(" ({list})"));
    }
    local.push_str(".\n");
    if !relevant_rows.is_empty() {
        local.push_str("\nBest matches:\n");
        relevant_rows.iter().for_each(|row| local.push_str(&local_line(row, off)));
    } else if !recent.is_empty() {
        local.push_str("\nMost recent:\n");
        recent.iter().take(5).for_each(|row| local.push_str(&local_line(row, off)));
    }
    if !convs.is_empty() {
        local.push_str("\nMeetings and voice notes:\n");
        for (_, title, kind, started, duration) in &convs {
            let minutes = duration.map(|d| format!(", {} min", (d / 60_000).max(1))).unwrap_or_default();
            local.push_str(&format!("• {title} ({kind}, {}{minutes})\n", fmt_local(*started, off)));
        }
    }
    if !follow_ups.is_empty() {
        local.push_str("\nFollow-ups:\n");
        for (content, status, urgent) in &follow_ups {
            local.push_str(&format!("• {content} [{status}]{}\n", if *urgent { " (urgent)" } else { "" }));
        }
    }
    user.push_str(&format!("\nQuestion: {question}"));
    Ok(Prompt { user, has_data, local })
}

fn none_found(scope: &Scope) -> String {
    format!(
        "I don't have any memories from {}. Either nothing was captured then, or capture was paused.",
        scope.label
    )
}

const SYSTEM: &str = "You are Memento, a private, local memory assistant for the user's workday. Answer briefly and concretely using only the provided memories, meetings, and follow-ups. If a date range is given, it is the exact range that was searched: you may mention it in your answer, and must not claim anything about times outside it. Prefer what the memory actually says; never invent facts; say plainly when the memory has nothing relevant.";

pub fn answer(state: &MonitorState, question: &str) -> Result<String, String> {
    answer_at(state, question, db::now_ms(), &lt::system_offset)
}

fn answer_at(
    state: &MonitorState,
    question: &str,
    now: i64,
    off: Offset,
) -> Result<String, String> {
    let parsed = parse_question(question, now, off);
    // Screen context is only relevant when the question is about now.
    let about_now = parsed.scope.as_ref().is_none_or(|s| s.end > now);
    let screen = about_now.then(screen_context);

    // Gather under the lock, then release it before the (slow) model call.
    let (config, prompt) = {
        let db = state.db();
        let config = llm::load_config(&db);
        let prompt = build_prompt(&db, question, &parsed, screen.as_deref(), now, off)?;
        (config, prompt)
    };
    if let (Some(scope), false) = (&parsed.scope, prompt.has_data) {
        return Ok(none_found(scope));
    }
    // Searching Memento's own data needs no model, so a missing or failing
    // model degrades to "here is what I found" instead of an error.
    match config {
        Ok(Some(config)) => match llm::chat_completion_with(&config, SYSTEM, &prompt.user) {
            Ok(text) => Ok(text),
            Err(error) => Ok(local_answer(
                &prompt.local,
                &format!("The language model didn't respond ({}), so here is what I found in your memory.", brief(&error)),
                "Check the model in Settings → Memory & vault → Smart summaries.",
            )),
        },
        Ok(None) => Ok(local_answer(
            &prompt.local,
            "No language model is connected, so I can't write an answer — here is what I found in your memory.",
            "Connect a model in Settings → Memory & vault → Smart summaries to get written answers.",
        )),
        Err(error) => Ok(local_answer(
            &prompt.local,
            &format!("The language model isn't set up correctly ({}), so here is what I found in your memory.", brief(&error)),
            "Check the model in Settings → Memory & vault → Smart summaries.",
        )),
    }
}

/// Error text short enough for a chat bubble.
fn brief(error: &str) -> String {
    let flat = error.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 110 {
        format!("{}…", flat.chars().take(110).collect::<String>())
    } else {
        flat
    }
}

/// What the overlay shows when there is no model answer: why, what was found,
/// and how to get proper answers.
fn local_answer(body: &str, reason: &str, tip: &str) -> String {
    let body = body.trim_end();
    let body = if body.is_empty() {
        "Nothing in your memory matches that yet."
    } else {
        body
    };
    format!("{reason}\n\n{body}\n\n{tip}")
}

fn screen_context() -> String {
    let focused = crate::focused_capture();
    let mut out = format!(
        "On screen right now:\nApp: {}\nWindow title: {}\n",
        focused.app.as_deref().unwrap_or("unknown"),
        focused.window_title.as_deref().unwrap_or("unknown"),
    );
    if let Some(selection) = focused.selected_text.as_deref().filter(|s| !s.trim().is_empty()) {
        out.push_str(&format!("Selected text: {}\n", snippet(selection, 800)));
    } else if let Some(text) = focused.focused_text.as_deref().filter(|s| !s.trim().is_empty()) {
        out.push_str(&format!("Focused text: {}\n", snippet(text, 800)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::localtime::{days_from_civil as dfc, testutil::dst_zone, HOUR_MS};

    fn utc(_: i64) -> i64 {
        0
    }

    /// Monday 2026-10-05 12:00 UTC.
    fn monday() -> i64 {
        dfc(2026, 10, 5) * DAY_MS + 12 * HOUR_MS
    }

    fn scope_days(q: &str, now: i64) -> Option<(i64, i64)> {
        parse_question(q, now, &utc).scope.map(|s| {
            ((s.start / DAY_MS) - dfc(1970, 1, 1), (s.end - s.start) / DAY_MS)
        })
    }

    fn range(q: &str, now: i64) -> Option<(String, String)> {
        parse_question(q, now, &utc).scope.map(|s| {
            (
                lt::format_ymd(s.start.div_euclid(DAY_MS)),
                lt::format_ymd((s.end - 1).div_euclid(DAY_MS)),
            )
        })
    }

    fn r(a: &str, b: &str) -> Option<(String, String)> {
        Some((a.into(), b.into()))
    }

    #[test]
    fn today_yesterday() {
        let now = monday();
        assert_eq!(range("what did I do today?", now), r("2026-10-05", "2026-10-05"));
        assert_eq!(range("What happened yesterday", now), r("2026-10-04", "2026-10-04"));
        assert_eq!(range("the day before yesterday", now), r("2026-10-03", "2026-10-03"));
        assert_eq!(scope_days("today", now).unwrap().1, 1);
    }

    #[test]
    fn weeks_start_on_monday() {
        // Today is Monday: this week is today..Sunday; last week is the prior Mon..Sun.
        let now = monday();
        assert_eq!(range("this week", now), r("2026-10-05", "2026-10-11"));
        assert_eq!(range("last week", now), r("2026-09-28", "2026-10-04"));
        // Sunday still belongs to the week that began on Monday.
        let sunday = dfc(2026, 10, 11) * DAY_MS + HOUR_MS;
        assert_eq!(range("this week", sunday), r("2026-10-05", "2026-10-11"));
        assert_eq!(range("last week", sunday), r("2026-09-28", "2026-10-04"));
    }

    #[test]
    fn month_rollovers() {
        let jan = dfc(2026, 1, 3) * DAY_MS;
        assert_eq!(range("last month", jan), r("2025-12-01", "2025-12-31"));
        assert_eq!(range("this month", jan), r("2026-01-01", "2026-01-31"));
        // Last week crossing a year boundary: Sat 2026-01-03 is in the week of Mon 2025-12-29.
        assert_eq!(range("last week", jan), r("2025-12-22", "2025-12-28"));
        assert_eq!(range("this week", jan), r("2025-12-29", "2026-01-04"));
        let leap = dfc(2024, 3, 1) * DAY_MS;
        assert_eq!(range("yesterday", leap), r("2024-02-29", "2024-02-29"));
    }

    #[test]
    fn last_weekday() {
        let now = monday(); // Monday
        // Today is Monday: "last Monday" is a week ago, not today.
        assert_eq!(range("last Monday", now), r("2026-09-28", "2026-09-28"));
        assert_eq!(range("what did I do last Tuesday?", now), r("2026-09-29", "2026-09-29"));
        assert_eq!(range("last sunday", now), r("2026-10-04", "2026-10-04"));
        assert_eq!(range("emails on last friday", now), r("2026-10-02", "2026-10-02"));
    }

    #[test]
    fn past_n_days() {
        let now = monday();
        assert_eq!(range("past 3 days", now), r("2026-10-03", "2026-10-05"));
        assert_eq!(range("last two weeks", now), r("2026-09-22", "2026-10-05"));
        assert_eq!(range("past week", now), r("2026-09-29", "2026-10-05"));
        assert_eq!(range("past 0 days", now), None);
        assert_eq!(range("past 9999 days", now), None);
    }

    #[test]
    fn explicit_dates() {
        let now = monday();
        assert_eq!(range("what happened on March 3?", now), r("2026-03-03", "2026-03-03"));
        assert_eq!(range("3rd of march", now), r("2026-03-03", "2026-03-03"));
        assert_eq!(range("on 12 Dec", now), r("2025-12-12", "2025-12-12")); // future -> last year
        assert_eq!(range("March 3, 2024", now), r("2024-03-03", "2024-03-03"));
        assert_eq!(range("on 2026-02-27", now), r("2026-02-27", "2026-02-27"));
        assert_eq!(range("february 30", now), None);
    }

    #[test]
    fn parts_of_day() {
        let now = monday();
        let s = parse_question("what did I work on this morning", now, &utc).scope.unwrap();
        assert_eq!(s.start, dfc(2026, 10, 5) * DAY_MS);
        assert_eq!(s.end - s.start, 12 * HOUR_MS);
        let s = parse_question("yesterday afternoon", now, &utc).scope.unwrap();
        assert_eq!(s.start, dfc(2026, 10, 4) * DAY_MS + 12 * HOUR_MS);
        assert_eq!(s.end - s.start, 6 * HOUR_MS);
    }

    #[test]
    fn ambiguous_phrases_have_no_range() {
        let now = monday();
        for q in [
            "what did I do on tuesday",
            "recently",
            "this tuesday",
            "last time we spoke",
            "the march meeting",
            "this year",
            "what about the may release",
            "",
        ] {
            assert_eq!(parse_question(q, now, &utc).scope, None, "{q}");
        }
        assert_eq!(parse_question("  budget talk ", now, &utc).topic, "budget talk");
    }

    #[test]
    fn phrase_is_stripped_from_topic() {
        let now = monday();
        let p = parse_question("What did Priya say about pricing on last Tuesday?", now, &utc);
        assert_eq!(p.topic, "What did Priya say about pricing");
        assert_eq!(keywords(&p.topic), vec!["priya", "say", "pricing"]);
        let p = parse_question("what did I do yesterday", now, &utc);
        assert!(keywords(&p.topic).is_empty());
    }

    #[test]
    fn dst_day_spans_23_hours() {
        // New York 2026-03-08: spring forward at 07:00 UTC.
        let change = dfc(2026, 3, 8) * DAY_MS + 7 * HOUR_MS;
        let zone = dst_zone(change);
        let now = dfc(2026, 3, 9) * DAY_MS + 15 * HOUR_MS; // Monday afternoon local
        let s = parse_question("yesterday", now, &zone).scope.unwrap();
        assert_eq!(s.end - s.start, 23 * HOUR_MS);
        assert_eq!(s.start, dfc(2026, 3, 8) * DAY_MS + 5 * HOUR_MS);
        let s = parse_question("today", now, &zone).scope.unwrap();
        assert_eq!(s.start, dfc(2026, 3, 9) * DAY_MS + 4 * HOUR_MS);
        assert_eq!(s.end - s.start, 24 * HOUR_MS);
        // Late evening local still counts as the same local day (UTC date differs).
        let late = dfc(2026, 3, 10) * DAY_MS + 2 * HOUR_MS; // 22:00 local on the 9th
        let s = parse_question("today", late, &zone).scope.unwrap();
        assert!(s.start <= late && late < s.end);
        assert_eq!(s.start, dfc(2026, 3, 9) * DAY_MS + 4 * HOUR_MS);
    }

    fn insert(db: &Db, title: &str, app: &str, content: &str, at: i64) -> i64 {
        db.conn
            .execute(
                "INSERT INTO memories (source, app, title, content, created_at) VALUES ('capture', ?1, ?2, ?3, ?4)",
                params![app, title, content, at],
            )
            .unwrap();
        db.conn.last_insert_rowid()
    }

    fn fixture_db() -> Db {
        let db = db::open_in_memory_for_test().unwrap();
        db.insert_memory("capture", Some("Slack"), "#design-review", "Priya: can you send the revised mockups by Friday?", false, false, None).unwrap();
        db.insert_memory("note", Some("Memento"), "Dentist", "Call the dentist to move the appointment", false, true, None).unwrap();
        db
    }

    #[test]
    fn the_local_answer_lists_matching_memories_and_follow_ups() {
        let db = fixture_db();
        db.insert_action_item_if_missing(1, "Send the revised mockups to Priya", true, None).unwrap();
        let now = db::now_ms();
        let parsed = parse_question("what did Priya ask for", now, &utc);
        let prompt = build_prompt(&db, "what did Priya ask for", &parsed, None, now, &utc).unwrap();
        assert!(prompt.local.contains("Best matches:"), "{}", prompt.local);
        assert!(prompt.local.contains("#design-review"), "{}", prompt.local);
        assert!(prompt.local.contains("revised mockups"));
        assert!(prompt.local.contains("Open follow-ups:") && prompt.local.contains("(urgent)"));
    }

    #[test]
    fn a_scoped_local_answer_states_the_range_and_counts() {
        let db = fixture_db();
        let now = db::now_ms();
        let parsed = parse_question("what did I do today", now, &utc);
        let prompt = build_prompt(&db, "what did I do today", &parsed, None, now, &utc).unwrap();
        assert!(prompt.local.starts_with("2 memories from today"), "{}", prompt.local);
        assert!(prompt.local.contains("Slack 1") && prompt.local.contains("Memento 1"));
        assert!(prompt.local.contains("Most recent:"));
    }

    #[test]
    fn with_no_model_the_answer_is_what_memory_holds_not_an_error() {
        let state = MonitorState::new(fixture_db());
        let text = answer(&state, "what did Priya ask for").expect("must not be an error");
        assert!(text.starts_with("No language model is connected"), "{text}");
        assert!(text.contains("revised mockups"), "{text}");
        assert!(text.contains("Settings → Memory & vault → Smart summaries"), "{text}");
    }

    #[test]
    fn nothing_found_says_so_plainly() {
        let state = MonitorState::new(db::open_in_memory_for_test().unwrap());
        let text = answer(&state, "what did Priya ask for").unwrap();
        assert!(text.contains("Nothing in your memory matches that yet."), "{text}");
    }

    #[test]
    fn long_model_errors_are_shortened_for_the_bubble() {
        let long = "x ".repeat(200);
        assert!(brief(&long).chars().count() <= 111);
        assert!(brief("short  error\n here").contains("short error here"));
    }

    #[test]
    fn retrieval_is_limited_to_range() {
        let db = db::open_in_memory_for_test().unwrap();
        let now = monday();
        let yesterday = now - DAY_MS;
        insert(&db, "Pricing doc", "Notion", "pricing tiers draft", yesterday);
        insert(&db, "Pricing old", "Notion", "pricing from last month", now - 20 * DAY_MS);
        insert(&db, "Lunch", "Slack", "lunch plans", now - HOUR_MS);

        let q = "what about pricing yesterday";
        let parsed = parse_question(q, now, &utc);
        let p = build_prompt(&db, q, &parsed, None, now, &utc).unwrap();
        assert!(p.has_data);
        assert!(p.user.contains("Pricing doc"));
        assert!(!p.user.contains("Pricing old"));
        assert!(!p.user.contains("Lunch"));
        assert!(p.user.contains("yesterday (Sun 4 Oct)"));
        assert!(p.user.contains("Date range for this question"));
    }

    #[test]
    fn empty_range_reports_no_data() {
        let db = db::open_in_memory_for_test().unwrap();
        let now = monday();
        insert(&db, "Something", "Notes", "x", now - 20 * DAY_MS);
        let q = "what did I do yesterday?";
        let parsed = parse_question(q, now, &utc);
        let p = build_prompt(&db, q, &parsed, None, now, &utc).unwrap();
        assert!(!p.has_data);
        assert!(none_found(parsed.scope.as_ref().unwrap()).starts_with("I don't have any memories from yesterday"));
    }

    #[test]
    fn meetings_and_followups_count_as_data() {
        let db = db::open_in_memory_for_test().unwrap();
        let now = monday();
        let y = now - DAY_MS;
        db.conn
            .execute(
                "INSERT INTO conversations (title, started_at, duration_ms, kind) VALUES ('Standup', ?1, 600000, 'meeting')",
                params![y],
            )
            .unwrap();
        let q = "yesterday";
        let parsed = parse_question(q, now, &utc);
        let p = build_prompt(&db, q, &parsed, None, now, &utc).unwrap();
        assert!(p.has_data);
        assert!(p.user.contains("Standup (meeting"));
        assert!(p.user.contains("10 min"));
    }

    #[test]
    fn unscoped_question_keeps_old_behaviour() {
        let db = db::open_in_memory_for_test().unwrap();
        insert(&db, "Roadmap", "Notion", "roadmap review", monday());
        let q = "roadmap";
        let parsed = parse_question(q, monday(), &utc);
        assert!(parsed.scope.is_none());
        let p = build_prompt(&db, q, &parsed, Some("On screen right now:\n"), monday(), &utc).unwrap();
        assert!(p.user.contains("Roadmap"));
        assert!(!p.user.contains("Date range"));
    }
}
