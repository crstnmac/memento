//! Optional LLM-backed transformation of raw captures into readable,
//! schema-validated memory records. Raw content remains authoritative.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::Emitter;

use crate::db::{Db, MemoryRow};
use crate::state::MonitorState;

static ENRICHMENT_ACTIVE: AtomicBool = AtomicBool::new(false);

struct EnrichmentGuard;

impl EnrichmentGuard {
    fn acquire() -> Option<Self> {
        ENRICHMENT_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .ok()
            .map(|_| Self)
    }
}

impl Drop for EnrichmentGuard {
    fn drop(&mut self) {
        ENRICHMENT_ACTIVE.store(false, Ordering::Release);
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructuredMemory {
    pub summary: String,
    #[serde(default)]
    pub people: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub commitments: Vec<String>,
    #[serde(default)]
    pub dates: Vec<String>,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub action_items: Vec<StructuredAction>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StructuredAction {
    pub content: String,
    #[serde(default)]
    pub urgent: bool,
    #[serde(default)]
    pub remind_at: Option<i64>,
}

#[derive(Clone)]
pub(crate) struct Config {
    provider: String,
    base_url: String,
    model: String,
    api_key: String,
}

pub fn spawn(app: tauri::AppHandle, state: Arc<MonitorState>) {
    std::thread::spawn(move || {
        while state.running.load(Ordering::Relaxed) {
            // Coding-agent CLIs are full application processes. Keep the
            // background pass deliberately small so capture remains light.
            match enrich_pending(&state, 1) {
                Ok(0) => {}
                Ok(_) => {
                    let _ = app.emit("memories-enriched", ());
                }
                Err(error) => log::warn!("LLM enrichment: {error}"),
            }
            std::thread::sleep(Duration::from_secs(30));
        }
    });
}

/// Enrich up to `limit` memories. The shared DB lock is only held for the
/// short reads/writes — never across the (slow, networked) model call, which
/// would freeze every other command in the app.
pub fn enrich_pending(state: &MonitorState, limit: i64) -> Result<usize, String> {
    let Some(_guard) = EnrichmentGuard::acquire() else {
        return Ok(0);
    };
    let (config, memories) = {
        let db = state.db();
        let Some(config) = load_config(&db)? else {
            return Ok(0);
        };
        (config, db.memories_needing_enrichment(limit)?)
    };
    let mut enriched = 0;
    let mut failures = Vec::new();
    for memory in memories {
        let result: Result<(), String> = (|| {
            let structured = request(&config, &memory)?;
            validate(&structured)?;
            let encoded = serde_json::to_string(&structured).map_err(|e| e.to_string())?;
            let db = state.db();
            db.save_memory_enrichment(
                memory.id,
                &config.provider,
                &config.model,
                stable_hash(&memory.content) as i64,
                &encoded,
                &structured.summary,
            )?;
            for action in &structured.action_items {
                db.insert_action_item_if_missing(
                    memory.id,
                    action.content.trim(),
                    action.urgent,
                    action.remind_at,
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => enriched += 1,
            Err(error) => {
                log::warn!("LLM enrichment failed for memory {}: {error}", memory.id);
                failures.push(format!("memory {}: {error}", memory.id));
            }
        }
    }
    if enriched == 0 && !failures.is_empty() {
        return Err(failures.join("; "));
    }
    Ok(enriched)
}

pub(crate) fn load_config(db: &Db) -> Result<Option<Config>, String> {
    if db.get_setting("llmEnabled")?.as_deref() != Some("true") {
        return Ok(None);
    }
    let provider = db
        .get_setting("llmProvider")?
        .unwrap_or_else(|| "ollama".into());
    let base_url = db
        .get_setting("llmBaseUrl")?
        .unwrap_or_else(|| "http://127.0.0.1:11434/v1".into());
    let model = db
        .get_setting("llmModel")?
        .unwrap_or_else(|| "llama3.2".into());
    let api_key = db.get_setting("llmApiKey")?.unwrap_or_default();
    if !is_cli_provider(&provider)
        && !base_url.starts_with("http://127.0.0.1:")
        && !base_url.starts_with("http://localhost:")
        && !base_url.starts_with("https://")
    {
        return Err("LLM base URL must use HTTPS unless it is loopback".into());
    }
    Ok(Some(Config {
        provider,
        base_url: base_url.trim_end_matches('/').into(),
        model,
        api_key,
    }))
}

pub(crate) fn chat_completion_with(
    config: &Config,
    system: &str,
    user: &str,
) -> Result<String, String> {
    if is_cli_provider(&config.provider) {
        return cli_completion(config, system, user);
    }
    let body = json!({
        "model": config.model,
        "temperature": 0.3,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ]
    });
    let mut request = ureq::post(&format!("{}/chat/completions", config.base_url))
        .set("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(120));
    if !config.api_key.is_empty() {
        request = request.set("Authorization", &format!("Bearer {}", config.api_key));
    }
    let response = request.send_json(body).map_err(|error| match error {
        ureq::Error::Status(code, response) => {
            let detail = response.into_string().unwrap_or_default();
            format!(
                "{} provider returned HTTP {code}: {}",
                config.provider,
                detail.chars().take(300).collect::<String>()
            )
        }
        other => format!("{} provider request failed: {other}", config.provider),
    })?;
    let value: serde_json::Value = response.into_json().map_err(|e| e.to_string())?;
    value
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "LLM response contains no message content".to_string())
}

fn request(config: &Config, memory: &MemoryRow) -> Result<StructuredMemory, String> {
    let system = "Transform captured work activity into a concise structured memory. Do not invent facts. Return JSON only with exactly these keys: summary (string); people, topics, decisions, commitments, dates, entities (arrays of strings); actionItems (array of objects with exactly content:string, urgent:boolean, remindAt:integer Unix epoch milliseconds or null). Create an action item for every explicit unfinished request, obligation, task, promise, or commitment, including phrases such as 'I need to', 'I will', 'please', 'can you', 'remember to', and 'follow up'. Immediate work is still an action until the source says it is complete. Example: 'I need to send the report urgently' must produce {content:'Send the report',urgent:true,remindAt:null}. Do not create actions for completed work, observations, navigation, advertisements, or speculative suggestions. Set remindAt only when the source states a deadline; never guess one.";
    let input = format!(
        "App: {}\nTitle: {}\nCaptured text:\n{}",
        memory.app.as_deref().unwrap_or("Unknown"),
        memory.title,
        memory.content.chars().take(12_000).collect::<String>()
    );
    if is_cli_provider(&config.provider) {
        return parse_structured_content(&cli_completion(config, system, &input)?);
    }
    let body = json!({
        "model": config.model,
        "temperature": 0,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": input }
        ]
    });
    let mut req = ureq::post(&format!("{}/chat/completions", config.base_url))
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(120));
    if !config.api_key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", config.api_key));
    }
    let response = req.send_json(body).map_err(|error| match error {
        ureq::Error::Status(code, response) => {
            let detail = response.into_string().unwrap_or_default();
            format!(
                "{} provider returned HTTP {code}: {}",
                config.provider,
                detail.chars().take(500).collect::<String>()
            )
        }
        other => format!("{} provider request failed: {other}", config.provider),
    })?;
    let value: serde_json::Value = response.into_json().map_err(|e| e.to_string())?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or("LLM response contained no message content")?;
    parse_structured_content(content)
}

fn is_cli_provider(provider: &str) -> bool {
    matches!(provider, "claude-code" | "codex-cli" | "grok-cli")
}

fn resolve_executable(name: &str) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    let mut directories = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        directories.extend([
            home.join(".local/bin"),
            home.join(".bun/bin"),
            home.join(".npm-global/bin"),
        ]);
    }
    directories
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| format!("{name} is not installed or is not available in Memento's PATH"))
}

fn cli_completion(config: &Config, system: &str, user: &str) -> Result<String, String> {
    let prompt = format!("{system}\n\n{user}");
    let (binary, args, prompt_on_stdin): (&str, Vec<&str>, bool) = match config.provider.as_str() {
        "claude-code" => (
            "claude",
            vec!["-p", "--output-format", "json", "--max-turns", "1"],
            true,
        ),
        "codex-cli" => (
            "codex",
            vec![
                "exec",
                "--skip-git-repo-check",
                "--ephemeral",
                "--ignore-rules",
                "--sandbox",
                "read-only",
                "--color",
                "never",
                "-",
            ],
            true,
        ),
        "grok-cli" => (
            "grok",
            vec!["--no-auto-update", "-p", &prompt, "--output-format", "json"],
            false,
        ),
        provider => return Err(format!("Unsupported local coding agent: {provider}")),
    };
    let executable = resolve_executable(binary)?;
    let mut command = Command::new(&executable);
    command
        .args(args)
        .current_dir(std::env::temp_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.stdin(if prompt_on_stdin {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start {binary}: {error}"))?;
    if prompt_on_stdin {
        child
            .stdin
            .take()
            .ok_or_else(|| format!("Could not open {binary} input"))?
            .write_all(prompt.as_bytes())
            .map_err(|error| format!("Could not send prompt to {binary}: {error}"))?;
    }
    let output = wait_with_timeout(child, CLI_TIMEOUT)
        .map_err(|error| format!("{binary} failed: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{binary} exited with {}: {}",
            output.status,
            detail.trim().chars().take(500).collect::<String>()
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| format!("{binary} returned non-UTF-8 output"))?;
    if config.provider == "codex-cli" {
        return Ok(stdout.trim().to_string());
    }
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|error| format!("{binary} returned invalid JSON: {error}"))?;
    ["result", "response", "content", "message"]
        .iter()
        .find_map(|key| value.get(key).and_then(|item| item.as_str()))
        .map(str::to_string)
        .ok_or_else(|| format!("{binary} response contains no result text"))
}

const CLI_TIMEOUT: Duration = Duration::from_secs(180);

/// `Child::wait_with_output` with a deadline: a hung coding-agent CLI must not
/// stall the enrichment worker (and the commands waiting on it) forever.
fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    fn drain<R: Read + Send + 'static>(stream: Option<R>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_end(&mut buffer);
            }
            buffer
        })
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("timed out after {}s", timeout.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    Ok(std::process::Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn parse_structured_content(content: &str) -> Result<StructuredMemory, String> {
    let trimmed = content.trim();
    let json = if trimmed.starts_with("```") {
        trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```JSON"))
            .or_else(|| trimmed.strip_prefix("```"))
            .and_then(|value| value.strip_suffix("```"))
            .unwrap_or(trimmed)
            .trim()
    } else {
        trimmed
    };
    let mut value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("invalid structured memory: {e}"))?;
    // OpenAI-compatible models sometimes honor the outer schema while naming
    // action fields semantically (description/priority/dueDate). Normalize
    // those common variants without allowing their extra prose into storage.
    if let Some(actions) = value
        .get_mut("actionItems")
        .and_then(|item| item.as_array_mut())
    {
        for action in actions {
            let Some(object) = action.as_object_mut() else {
                continue;
            };
            let content = object
                .get("content")
                .cloned()
                .or_else(|| object.get("description").cloned())
                .unwrap_or_else(|| serde_json::Value::String(String::new()));
            let urgent = object
                .get("urgent")
                .and_then(|item| item.as_bool())
                .unwrap_or_else(|| {
                    object
                        .get("priority")
                        .and_then(|item| item.as_str())
                        .is_some_and(|priority| {
                            matches!(
                                priority.to_lowercase().as_str(),
                                "high" | "urgent" | "critical"
                            )
                        })
                });
            let remind_at = object
                .get("remindAt")
                .cloned()
                .or_else(|| {
                    object
                        .get("dueDate")
                        .filter(|item| item.is_number())
                        .cloned()
                })
                .unwrap_or(serde_json::Value::Null);
            object.clear();
            object.insert("content".into(), content);
            object.insert("urgent".into(), serde_json::Value::Bool(urgent));
            object.insert("remindAt".into(), remind_at);
        }
    }
    serde_json::from_value(value).map_err(|e| format!("invalid structured memory: {e}"))
}

fn validate(memory: &StructuredMemory) -> Result<(), String> {
    if memory.summary.trim().is_empty() || memory.summary.chars().count() > 800 {
        return Err("structured summary must contain 1–800 characters".into());
    }
    for list in [
        &memory.people,
        &memory.topics,
        &memory.decisions,
        &memory.commitments,
        &memory.dates,
        &memory.entities,
    ] {
        if list.len() > 50 || list.iter().any(|value| value.chars().count() > 300) {
            return Err("structured memory exceeded schema limits".into());
        }
    }
    if memory.action_items.len() > 50
        || memory
            .action_items
            .iter()
            .any(|action| action.content.trim().is_empty() || action.content.chars().count() > 300)
    {
        return Err("structured action items exceeded schema limits".into());
    }
    Ok(())
}

fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structured_schema_rejects_empty_and_oversized_summaries() {
        let mut memory = StructuredMemory {
            summary: "Useful summary".into(),
            people: vec![],
            topics: vec![],
            decisions: vec![],
            commitments: vec![],
            dates: vec![],
            entities: vec![],
            action_items: vec![],
        };
        assert!(validate(&memory).is_ok());
        memory.summary.clear();
        assert!(validate(&memory).is_err());
    }

    #[test]
    fn structured_content_accepts_markdown_fenced_json() {
        let parsed = parse_structured_content(
            "```json\n{\"summary\":\"Useful\",\"people\":[],\"topics\":[],\"decisions\":[],\"commitments\":[],\"dates\":[],\"entities\":[],\"actionItems\":[]}\n```",
        ).expect("fenced response should parse");
        assert_eq!(parsed.summary, "Useful");
    }

    #[test]
    fn structured_content_normalizes_common_action_aliases() {
        let parsed = parse_structured_content(
            r#"{"summary":"Work remains","people":[],"topics":[],"decisions":[],"commitments":[],"dates":[],"entities":[],"actionItems":[{"description":"Send report","priority":"high","status":"open","dueDate":null}]}"#,
        ).expect("aliased action should parse");
        assert_eq!(parsed.action_items[0].content, "Send report");
        assert!(parsed.action_items[0].urgent);
    }
}
