//! Local follow-up detection. No LLM required.
//!
//! Works sentence by sentence on whole words, so "Chi will drive" or
//! "Wi-Fi will be down" are not commitments. Two kinds of language count:
//!
//! - **requests** ("can you send…", "please follow up…", "remember to…") — real
//!   follow-ups for the user wherever they appear;
//! - **first-person promises** ("I'll send…", "I need to…", "we have to…") — only
//!   when the text is the user's own words (notes, voice notes, meetings). On a
//!   captured screen the visible conversation is mostly other people, so a
//!   promise there is not the user's follow-up.
//!
//! A deadline or urgency is taken only from the *same sentence* as the
//! commitment, never from elsewhere on the page.

use std::collections::HashSet;

use crate::localtime::{self as lt, Offset};

/// Whose words the text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speaker {
    /// The user's own writing or speech: notes, voice notes, meetings.
    Own,
    /// A captured screen: a mix of the user and other people.
    Mixed,
}

/// A detected commitment, with optional priority and scheduling signals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedAction {
    pub content: String,
    pub urgent: bool,
    pub remind_at: Option<i64>,
}

const MAX_ACTIONS: usize = 8;
const MAX_CHARS: usize = 140;

/// First-person promises (Own only).
const PROMISES: &[&[&str]] = &[
    &["i", "will"],
    &["i'll"],
    &["i", "need", "to"],
    &["i", "have", "to"],
    &["i", "must"],
    &["i", "am", "going", "to"],
    &["i'm", "going", "to"],
    &["i", "gotta"],
    &["we", "need", "to"],
    &["we", "have", "to"],
    &["we", "will"],
    &["we'll"],
];

/// Requests and reminders (any source).
const REQUESTS: &[&[&str]] = &[
    &["can", "you"],
    &["could", "you"],
    &["would", "you"],
    &["will", "you"],
    &["please"],
    &["i", "need", "you", "to"],
    &["you", "need", "to"],
    &["make", "sure", "to"],
    &["make", "sure", "you"],
    &["remember", "to"],
    &["don't", "forget", "to"],
    &["dont", "forget", "to"],
    &["do", "not", "forget", "to"],
    &["remind", "me", "to"],
    &["reminder", "to"],
    &["follow", "up", "on"],
    &["follow", "up", "with"],
    &["follow", "up", "about"],
];

/// Words that start a clause but are not the action itself.
const FILLERS: &[&str] = &[
    "also", "just", "then", "really", "definitely", "still", "first", "now", "quickly", "go",
    "ahead", "and", "so", "kindly",
];

const URGENT: &[&[&str]] = &[
    &["asap"],
    &["urgent"],
    &["urgently"],
    &["immediately"],
    &["right", "away"],
    &["critical"],
    &["as", "soon", "as"],
];

/// Words whose appearance before the trigger makes the sentence hypothetical.
const HYPOTHETICAL: &[&str] = &["if", "unless", "whether", "when"];

const WEEKDAYS: [&str; 7] = [
    "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday",
];

const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september",
    "october", "november", "december",
];

// ------------------------------------------------------------- tokenizing

struct Word {
    /// Byte offset of the word in the sentence.
    start: usize,
    lower: String,
}

fn words(sentence: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '’' || c == '-';
    for (i, c) in sentence.char_indices() {
        match (is_word(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                push_word(&mut out, sentence, s, i);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        push_word(&mut out, sentence, s, sentence.len());
    }
    out
}

fn push_word(out: &mut Vec<Word>, sentence: &str, start: usize, end: usize) {
    let raw = &sentence[start..end];
    let trimmed = raw.trim_matches(|c| c == '-' || c == '\'' || c == '’');
    if trimmed.is_empty() {
        return;
    }
    let offset = raw.find(trimmed).unwrap_or(0);
    let lower = trimmed.to_lowercase().replace('’', "'");
    out.push(Word { start: start + offset, lower });
}

/// Word range `[start, end)` of the first occurrence of `phrase` at or after `from`.
fn find_phrase(words: &[Word], phrase: &[&str], from: usize) -> Option<(usize, usize)> {
    if phrase.is_empty() || words.len() < phrase.len() || from > words.len() - phrase.len() {
        return None;
    }
    (from..=words.len() - phrase.len()).find_map(|i| {
        phrase
            .iter()
            .enumerate()
            .all(|(k, w)| words[i + k].lower == *w)
            .then_some((i, i + phrase.len()))
    })
}

fn has_phrase(words: &[Word], phrases: &[&[&str]]) -> bool {
    phrases.iter().any(|p| find_phrase(words, p, 0).is_some())
}

// ------------------------------------------------------------- sentences

const ABBREVIATIONS: &[&str] = &["mr", "mrs", "ms", "dr", "vs", "sr", "jr", "st", "etc", "no"];

/// Splits on line breaks and sentence ends; "Mr. Smith" and "3.5" stay whole.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let mut start = 0usize;
        for (idx, &(pos, c)) in chars.iter().enumerate() {
            if !matches!(c, '.' | '!' | '?') {
                continue;
            }
            let next = chars.get(idx + 1).map(|&(_, n)| n);
            if next.is_some_and(|n| !n.is_whitespace() && !matches!(n, '"' | '\'' | ')' | '”')) {
                continue; // "3.5", "e.g.", "file.txt"
            }
            if c == '.' {
                let before = line[start..pos]
                    .rsplit(|ch: char| !ch.is_alphanumeric())
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                if ABBREVIATIONS.contains(&before.as_str()) {
                    continue;
                }
            }
            let end = pos + c.len_utf8();
            let piece = line[start..end].trim();
            if !piece.is_empty() {
                out.push(piece.to_string());
            }
            start = end;
        }
        let rest = line[start..].trim();
        if !rest.is_empty() {
            out.push(rest.to_string());
        }
    }
    out
}

// ---------------------------------------------------------------- detect

/// Candidate follow-ups in `text`. `now` and `off` (local UTC offset) are
/// injected so scheduling is testable.
pub fn detect(text: &str, speaker: Speaker, now: i64, off: Offset) -> Vec<DetectedAction> {
    let mut found: Vec<DetectedAction> = Vec::new();
    for sentence in sentences(text) {
        if found.len() >= MAX_ACTIONS {
            break;
        }
        let Some(action) = detect_in_sentence(&sentence, speaker, now, off) else {
            continue;
        };
        if !found.iter().any(|known| is_similar(&known.content, &action.content)) {
            found.push(action);
        }
    }
    found
}

fn detect_in_sentence(
    sentence: &str,
    speaker: Speaker,
    now: i64,
    off: Offset,
) -> Option<DetectedAction> {
    let tokens = words(sentence);
    let question = sentence.trim_end().ends_with('?');

    // Earliest trigger wins; requests are valid anywhere, promises only in the
    // user's own words, and a question only counts if it is a request.
    let mut best: Option<(usize, usize)> = None;
    let mut consider = |phrases: &[&[&str]]| {
        for phrase in phrases {
            if let Some(hit) = find_phrase(&tokens, phrase, 0) {
                if best.is_none_or(|b| hit.0 < b.0) {
                    best = Some(hit);
                }
            }
        }
    };
    consider(REQUESTS);
    if speaker == Speaker::Own && !question {
        consider(PROMISES);
    }
    let (trigger_start, trigger_end) = best?;

    // "If I have time I'll…" is not a commitment.
    if tokens[..trigger_start]
        .iter()
        .any(|w| HYPOTHETICAL.contains(&w.lower.as_str()))
    {
        return None;
    }
    // The trigger must actually be followed by an action.
    let after = tokens.get(trigger_end..)?;
    let skip = after
        .iter()
        .take_while(|w| FILLERS.contains(&w.lower.as_str()))
        .count();
    let first = after.get(skip)?;
    if matches!(
        first.lower.as_str(),
        "not" | "never" | "be" | "have" | "been" | "do" | "ever"
    ) || first.lower.ends_with("n't")
    {
        return None; // "I will not…", "I will be there", "I'll have been…"
    }

    let mut clause = sentence[first.start..].trim().to_string();
    // Cut trailing asides: "…report, thanks!" / "…it but I'm busy".
    let lowered = clause.to_lowercase();
    for stop in [
        " but ", " because ", " otherwise ", " so that ", " thanks", " thank you", " cheers",
    ] {
        if let Some(idx) = lowered.find(stop) {
            clause.truncate(idx);
            break;
        }
    }
    let clause = clause
        .trim_end_matches(['.', '!', '?', ',', ';', ':', ' '])
        .trim();
    if clause.split_whitespace().count() < 2 {
        return None;
    }
    let content = capitalize(&truncate_words(clause, MAX_CHARS));

    Some(DetectedAction {
        content,
        urgent: has_phrase(&tokens, URGENT),
        remind_at: schedule(&tokens, now, off),
    })
}

fn truncate_words(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let trimmed = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!(
        "{}…",
        trimmed.trim_end_matches(|c: char| !c.is_alphanumeric())
    )
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// -------------------------------------------------------------- scheduling

fn weekday_index(word: &str) -> Option<i64> {
    WEEKDAYS.iter().position(|d| *d == word).map(|i| i as i64)
}

/// A reminder time from the deadline wording in this one sentence; only ever
/// in the future.
fn schedule(tokens: &[Word], now: i64, off: Offset) -> Option<i64> {
    let has = |w: &str| tokens.iter().any(|t| t.lower == w);
    let has_all = |ws: &[&str]| find_phrase(tokens, ws, 0).is_some();
    let today = lt::day_number(now, off);
    let wd_today = lt::weekday_mon0(today);
    let at = |day: i64, hour: i64| lt::at_hour(day, hour, off);
    let future = |t: i64| (t > now).then_some(t);

    if has("tomorrow") {
        let hour = if has("afternoon") {
            14
        } else if has("evening") {
            18
        } else {
            9
        };
        return future(at(today + 1, hour));
    }
    if has("tonight") {
        return future(at(today, 20));
    }
    if has("eod")
        || has("cob")
        || has_all(&["end", "of", "day"])
        || has_all(&["end", "of", "the", "day"])
    {
        return future(at(today, 17));
    }
    if has_all(&["this", "afternoon"]) {
        return future(at(today, 15));
    }
    if has("eow") || has_all(&["end", "of", "week"]) || has_all(&["end", "of", "the", "week"]) {
        let day = today + (4 - wd_today).rem_euclid(7);
        return future(at(day, 17)).or_else(|| future(at(day + 7, 17)));
    }
    if has_all(&["next", "week"]) {
        return future(at(today + (7 - wd_today), 9));
    }
    if let Some(day) = calendar_date(tokens, today) {
        return future(at(day, 9));
    }
    for (i, token) in tokens.iter().enumerate() {
        let Some(target) = weekday_index(&token.lower) else {
            continue;
        };
        let next = i > 0 && tokens[i - 1].lower == "next";
        let mut ahead = (target - wd_today).rem_euclid(7);
        if next && ahead == 0 {
            ahead = 7;
        }
        // Said on the day itself: due by the end of today, if that's ahead.
        return if ahead == 0 {
            future(at(today, 17))
        } else {
            future(at(today + ahead, 9))
        };
    }
    None
}

/// "October 23", "Oct 23rd", "the 21st", "by the 15th" → a local day number
/// in the future (this month, else next).
fn calendar_date(tokens: &[Word], today: i64) -> Option<i64> {
    let (year, month, dom) = lt::civil_from_days(today);
    let day_number = |s: &str| -> Option<u32> {
        let digits = s.trim_end_matches(|c: char| c.is_alphabetic());
        let suffix = &s[digits.len()..];
        if !matches!(suffix, "" | "st" | "nd" | "rd" | "th") {
            return None;
        }
        digits.parse::<u32>().ok().filter(|d| (1..=31).contains(d))
    };
    // "<month> <day>"
    for pair in tokens.windows(2) {
        let month_idx = MONTHS.iter().position(|m| {
            *m == pair[0].lower || (pair[0].lower.len() == 3 && m.starts_with(&pair[0].lower))
        });
        if let (Some(mi), Some(d)) = (month_idx, day_number(&pair[1].lower)) {
            let m = mi as u32 + 1;
            let mut y = year;
            if (m, d) < (month, dom) {
                y += 1;
            }
            if lt::is_valid_civil(y, m, d) {
                return Some(lt::days_from_civil(y, m, d));
            }
        }
    }
    // "the <ordinal>" — must carry an ordinal suffix so a bare "the 5" is not a date.
    for pair in tokens.windows(2) {
        if pair[0].lower != "the" {
            continue;
        }
        let ordinal = ["st", "nd", "rd", "th"]
            .iter()
            .any(|suffix| pair[1].lower.ends_with(suffix));
        if let (true, Some(d)) = (ordinal, day_number(&pair[1].lower)) {
            let (mut y, mut m) = (year, month);
            if d < dom {
                m += 1;
                if m > 12 {
                    m = 1;
                    y += 1;
                }
            }
            if lt::is_valid_civil(y, m, d) {
                return Some(lt::days_from_civil(y, m, d));
            }
        }
    }
    None
}

// --------------------------------------------------------------- similarity

/// Lower-cased content words with simple stemming, used to compare follow-ups.
pub fn normalized_tokens(text: &str) -> HashSet<String> {
    const STOP: &[&str] = &[
        "the", "a", "an", "to", "by", "for", "with", "and", "i", "we", "this", "that", "on",
        "of", "in", "at", "it", "me", "my", "our", "you", "your",
    ];
    text.to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| token.len() > 1 && !STOP.contains(token))
        .map(|token| match token {
            "sent" => "send".to_string(),
            "shipped" => "ship".to_string(),
            "finished" | "completed" | "done" | "resolved" => "complete".to_string(),
            _ => token.trim_end_matches('s').to_string(),
        })
        .collect()
}

/// Whether two follow-ups say the same thing ("Send the report" and "Send the
/// report to Priya by Friday"). Used to stop re-captures of the same screen,
/// and the model plus these rules, from piling up duplicates.
pub fn is_similar(a: &str, b: &str) -> bool {
    let (a, b) = (normalized_tokens(a), normalized_tokens(b));
    let smaller = a.len().min(b.len());
    if smaller == 0 {
        return false;
    }
    if smaller == 1 {
        return a == b;
    }
    let overlap = a.intersection(&b).count();
    let union = a.len() + b.len() - overlap;
    overlap >= 2
        && (overlap as f32 / smaller as f32 >= 0.75 || overlap as f32 / union as f32 >= 0.6)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Wednesday 2026-10-07, 10:00 UTC, in a UTC "local" zone.
    fn now() -> i64 {
        lt::days_from_civil(2026, 10, 7) * lt::DAY_MS + 10 * lt::HOUR_MS
    }
    fn utc(_: i64) -> i64 {
        0
    }
    fn contents(text: &str, speaker: Speaker) -> Vec<String> {
        detect(text, speaker, now(), &utc)
            .into_iter()
            .map(|a| a.content)
            .collect()
    }
    fn local(day: (i64, u32, u32), hour: i64) -> i64 {
        lt::days_from_civil(day.0, day.1, day.2) * lt::DAY_MS + hour * lt::HOUR_MS
    }

    #[test]
    fn finds_requests_and_promises_in_a_meeting() {
        let transcript = "Karen, can you follow up with the infrastructure team and get a decision by Thursday? \
            I'd like it in writing, so please send me the confirmation email. \
            Yes, I will email them this afternoon and copy you. \
            I'll draft the runbook and share it by Wednesday morning.";
        assert_eq!(
            contents(transcript, Speaker::Own),
            vec![
                "Follow up with the infrastructure team and get a decision by Thursday",
                "Send me the confirmation email",
                "Email them this afternoon and copy you",
                "Draft the runbook and share it by Wednesday morning",
            ]
        );
    }

    #[test]
    fn words_not_substrings() {
        for text in [
            "Chi will drive and Wifi will be down.",
            "Hi will you be there?",
            "Theodore presented the roadmap.",
            "The address to the office changed.",
        ] {
            assert_eq!(contents(text, Speaker::Own), Vec::<String>::new(), "{text}");
        }
    }

    #[test]
    fn other_peoples_promises_on_a_captured_screen_are_not_yours() {
        let screen = "Priya: I'll update the checklist tomorrow.\nPriya: can you send the revised mockups by Friday?";
        assert_eq!(
            contents(screen, Speaker::Mixed),
            vec!["Send the revised mockups by Friday"]
        );
        // The same promise in the user's own note is theirs.
        assert_eq!(
            contents("I'll update the checklist tomorrow.", Speaker::Own),
            vec!["Update the checklist tomorrow"]
        );
    }

    #[test]
    fn negation_hypotheticals_attendance_and_past_are_skipped() {
        for text in [
            "I will not send it.",
            "I'll never ship that.",
            "If I have time I'll send the report.",
            "I will be there at noon.",
            "I sent the report yesterday.",
            "Will I send it?",
        ] {
            assert_eq!(contents(text, Speaker::Own), Vec::<String>::new(), "{text}");
        }
    }

    #[test]
    fn urgency_comes_only_from_urgent_wording() {
        let urgent = detect("Please send the invoice ASAP.", Speaker::Own, now(), &utc);
        assert!(urgent[0].urgent);
        for text in [
            "See you soon, I'll send the invoice.",
            "I'll send the invoice today.",
        ] {
            let found = detect(text, Speaker::Own, now(), &utc);
            assert!(!found[0].urgent, "{text}");
        }
    }

    #[test]
    fn deadlines_come_from_the_same_sentence() {
        let found = detect(
            "I'll send the report by Friday. Separately, I will call Sam.",
            Speaker::Own,
            now(),
            &utc,
        );
        assert_eq!(found[0].remind_at, Some(local((2026, 10, 9), 9)));
        // The Friday in the first sentence does not leak into the second.
        assert_eq!(found[1].remind_at, None);
    }

    #[test]
    fn schedules_common_phrases() {
        let at = |text: &str| detect(text, Speaker::Own, now(), &utc)[0].remind_at;
        assert_eq!(at("I need to file this tomorrow."), Some(local((2026, 10, 8), 9)));
        assert_eq!(
            at("I need to file this tomorrow afternoon."),
            Some(local((2026, 10, 8), 14))
        );
        assert_eq!(at("I need to file this by EOD."), Some(local((2026, 10, 7), 17)));
        assert_eq!(
            at("I need to file this by end of the week."),
            Some(local((2026, 10, 9), 17))
        );
        assert_eq!(at("I need to file this next week."), Some(local((2026, 10, 12), 9)));
        assert_eq!(at("I need to file this by Monday."), Some(local((2026, 10, 12), 9)));
        assert_eq!(
            at("I need to file this by October 23."),
            Some(local((2026, 10, 23), 9))
        );
        assert_eq!(
            at("I need to file this on the 21st."),
            Some(local((2026, 10, 21), 9))
        );
        assert_eq!(
            at("I need to file this by the 5th."),
            Some(local((2026, 11, 5), 9))
        );
        assert_eq!(at("I need to file this."), None);
        // "EOD" when 17:00 has already passed is not scheduled in the past.
        let late = lt::days_from_civil(2026, 10, 7) * lt::DAY_MS + 18 * lt::HOUR_MS;
        assert_eq!(
            detect("I need to file this by EOD.", Speaker::Own, late, &utc)[0].remind_at,
            None
        );
        // On the day itself, a weekday deadline means the end of today.
        assert_eq!(
            at("I need to file this by Wednesday."),
            Some(local((2026, 10, 7), 17))
        );
    }

    #[test]
    fn abbreviations_and_decimals_do_not_split_sentences() {
        assert_eq!(
            contents("Please ask Mr. Smith about the 3.5 release notes.", Speaker::Mixed),
            vec!["Ask Mr. Smith about the 3.5 release notes"]
        );
    }

    #[test]
    fn requests_work_with_chat_prefixes_and_fillers() {
        assert_eq!(
            contents(
                "Dana: hey, could you also just resend the invoice thanks!",
                Speaker::Mixed
            ),
            vec!["Resend the invoice"]
        );
        assert_eq!(
            contents("Remember to renew the parking permit.", Speaker::Mixed),
            vec!["Renew the parking permit"]
        );
    }

    #[test]
    fn repeats_inside_one_text_collapse() {
        let text = "Please send the report. Please send the report to Priya.";
        assert_eq!(contents(text, Speaker::Mixed).len(), 1);
    }

    #[test]
    fn long_clauses_are_trimmed_at_a_word() {
        let text = format!(
            "I need to {}.",
            "review the quarterly planning document ".repeat(10)
        );
        let found = contents(&text, Speaker::Own);
        assert!(found[0].chars().count() <= MAX_CHARS + 1, "{}", found[0]);
        assert!(found[0].ends_with('…'));
    }

    #[test]
    fn similarity_merges_rephrasings_but_not_different_tasks() {
        assert!(is_similar(
            "Send the launch report",
            "Send the launch report to Priya by Friday"
        ));
        assert!(is_similar(
            "Review the Q4 roadmap draft",
            "Review Q4 roadmap draft and reply with comments"
        ));
        assert!(!is_similar("Send the launch report", "Send the revised mockups"));
        assert!(!is_similar("Call Sam", "Call the dentist"));
        assert!(is_similar("Drive", "drive"));
        assert!(!is_similar("", "anything"));
    }
}
