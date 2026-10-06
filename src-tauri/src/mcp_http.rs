//! Local MCP server — the "LLM brain" surface, served over SSE on loopback.
//!
//! Mirrors Minimi's second MCP server: fully on-device, no backend
//! round-trip. An external LLM (Claude Desktop via `mcpStdio.cjs`, or any
//! EventSource-capable MCP client) connects to `GET /sse`, receives an
//! `endpoint` event with a session id, then sends JSON-RPC by `POST /messages`.
//! All tools run against the local SQLite database.

use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::json;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::local_tools;
use crate::state::MonitorState;

pub struct McpServerState {
    running: AtomicBool,
    loop_flag: Arc<AtomicBool>,
    port: Mutex<u16>,
    handle: Mutex<Option<JoinHandle<()>>>,
    token: Mutex<String>,
}

impl Default for McpServerState {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServerState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            loop_flag: Arc::new(AtomicBool::new(false)),
            port: Mutex::new(3961),
            handle: Mutex::new(None),
            token: Mutex::new(String::new()),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn port(&self) -> u16 {
        *self.port.lock().unwrap()
    }

    /// Bind the SSE server on 127.0.0.1:port and serve MCP over HTTP.
    /// Binding happens here (not on the worker thread) so a port conflict is
    /// reported to the caller instead of leaving the UI showing "running".
    /// Starting with a different port restarts the server on that port.
    pub fn start(&self, state: Arc<MonitorState>, port: u16, token: String) -> Result<(), String> {
        if self.is_running() {
            if self.port() == port {
                return Ok(());
            }
            self.stop()?;
        }
        let addr = format!("127.0.0.1:{port}");
        let server =
            Server::http(&addr).map_err(|e| format!("could not listen on {addr}: {e}"))?;
        self.running.store(true, Ordering::SeqCst);
        *self.port.lock().unwrap() = port;
        *self.token.lock().unwrap() = token.clone();
        self.loop_flag.store(true, Ordering::SeqCst);
        let loop_flag = Arc::clone(&self.loop_flag);
        let handle = std::thread::spawn(move || serve(server, state, token, loop_flag));
        *self.handle.lock().unwrap() = Some(handle);
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        if !self.running.swap(false, Ordering::SeqCst) {
            return Ok(());
        }
        self.loop_flag.store(false, Ordering::SeqCst);
        // Kick the accept loop out of recv with a throwaway connection, then
        // join so the port is released before a possible restart.
        let port = *self.port.lock().unwrap();
        let _ = std::net::TcpStream::connect(("127.0.0.1", port));
        if let Some(handle) = self.handle.lock().unwrap().take() {
            let _ = handle.join();
        }
        Ok(())
    }
}

const MAX_BODY_BYTES: u64 = 1024 * 1024;

type Sessions = Arc<Mutex<HashMap<String, mpsc::Sender<Vec<u8>>>>>;

fn serve(server: Server, state: Arc<MonitorState>, token: String, running: Arc<AtomicBool>) {
    log::info!("mcp: local MCP server listening");
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));

    while running.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(300)) {
            Ok(Some(request)) => dispatch(request, &state, &sessions, &token),
            Ok(None) => {}
            Err(e) => {
                if running.load(Ordering::SeqCst) {
                    log::warn!("mcp: accept error: {e}");
                }
            }
        }
    }
    log::info!("mcp: local MCP server stopped");
}

fn dispatch(request: Request, state: &Arc<MonitorState>, sessions: &Sessions, token: &str) {
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or("").to_string();
    match (request.method(), path.as_str()) {
        (_, "/") => respond_json(
            request,
            200,
            json!({ "name": "poppy-local-mcp", "ok": true }),
        ),
        (Method::Get, "/sse") => {
            if authenticated(&request, token) {
                handle_sse(request, sessions, token)
            } else {
                unauthorized(request)
            }
        }
        (Method::Post, "/messages") => {
            if authenticated(&request, token) {
                handle_messages(request, state, sessions)
            } else {
                unauthorized(request)
            }
        }
        _ => {
            let _ = request.respond(Response::empty(StatusCode(404)));
        }
    }
}

/// EventSource stream: hand out an endpoint + session id, then keep the
/// stream alive with idle pings. Each SSE connection runs on its own thread
/// so it can't block new tool calls.
fn handle_sse(request: Request, sessions: &Sessions, token: &str) {
    let sessions = Arc::clone(sessions);
    let token = token.to_string();
    std::thread::spawn(move || {
        let session = format!(
            "s-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let initial =
            format!("event: endpoint\ndata: /messages?session_id={session}&token={token}\n\n");

        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        sessions.lock().unwrap().insert(session.clone(), tx.clone());
        let headers = vec![
            Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..]).unwrap(),
            Header::from_bytes(&b"Cache-Control"[..], &b"no-cache"[..]).unwrap(),
            Header::from_bytes(&b"Connection"[..], &b"keep-alive"[..]).unwrap(),
        ];
        // No known length: tiny_http streams this chunked, keeping the
        // EventSource connection open until the client disconnects.
        let response = Response::new(
            StatusCode(200),
            headers,
            SseReader {
                rx,
                pending: Vec::new(),
                offset: 0,
                session: session.clone(),
                sessions,
            },
            None,
            None,
        );
        // Send the endpoint event first, then stream keepalives via respond.
        let _ = tx.send(initial.into_bytes());
        let _ = request.respond(response);
        // tx is dropped here — after respond returns the client disconnected.
    });
}

fn handle_messages(mut request: Request, state: &Arc<MonitorState>, sessions: &Sessions) {
    let session_id = query_value(request.url(), "session_id").map(str::to_string);
    let mut body = String::new();
    // Tool calls are tiny; cap the body so a stray client can't exhaust memory.
    let read = request
        .as_reader()
        .take(MAX_BODY_BYTES)
        .read_to_string(&mut body);
    if read.is_err() || body.len() as u64 >= MAX_BODY_BYTES {
        let _ = request.respond(Response::empty(StatusCode(413)));
        return;
    }
    let msg: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            deliver(
                request,
                session_id.as_deref(),
                sessions,
                json!({"jsonrpc":"2.0","error":{"code":-32700,"message":"Parse error"}}),
                400,
            );
            return;
        }
    };
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

    match method {
        "initialize" => {
            let result = json!({
                "protocolVersion": local_tools::PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "poppy-local-mcp", "version": env!("CARGO_PKG_VERSION") },
            });
            deliver_result(request, session_id.as_deref(), sessions, id, result);
        }
        "notifications/initialized" | "notifications/cancelled" => {
            let _ = request.respond(Response::empty(StatusCode(202)));
        }
        "ping" => deliver_result(request, session_id.as_deref(), sessions, id, json!({})),
        "tools/list" => deliver_result(
            request,
            session_id.as_deref(),
            sessions,
            id,
            json!({ "tools": tools() }),
        ),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            let args = msg
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let outcome = {
                let db = state.db();
                local_tools::call_tool(&db, name, &args)
            };
            match outcome {
                Ok(value) => {
                    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                    deliver_result(
                        request,
                        session_id.as_deref(),
                        sessions,
                        id,
                        json!({
                            "content": [ { "type": "text", "text": text } ],
                            "isError": false
                        }),
                    );
                }
                Err(e) => deliver_result(
                    request,
                    session_id.as_deref(),
                    sessions,
                    id,
                    json!({
                        "content": [ { "type": "text", "text": format!("Error: {e}") } ],
                        "isError": true
                    }),
                ),
            }
        }
        _ => deliver(
            request,
            session_id.as_deref(),
            sessions,
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Method not found: {method}") } }),
            200,
        ),
    }
}

fn deliver_result(
    request: Request,
    session: Option<&str>,
    sessions: &Sessions,
    id: Option<serde_json::Value>,
    result: serde_json::Value,
) {
    let mut body = json!({ "jsonrpc": "2.0", "result": result });
    if let Some(id) = id {
        body["id"] = id;
    }
    deliver(request, session, sessions, body, 200);
}

fn deliver(
    request: Request,
    session: Option<&str>,
    sessions: &Sessions,
    body: serde_json::Value,
    direct_status: u16,
) {
    if let Some(session) = session {
        let text = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
        let event = format!("event: message\ndata: {text}\n\n").into_bytes();
        let sender = sessions.lock().unwrap().get(session).cloned();
        match sender {
            Some(sender) if sender.send(event).is_ok() => {
                let _ = request.respond(Response::empty(StatusCode(202)));
            }
            _ => respond_json(
                request,
                404,
                json!({ "error": "unknown or closed MCP session" }),
            ),
        }
    } else {
        respond_json(request, direct_status, body);
    }
}

fn query_value<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    url.split_once('?')?.1.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then_some(value)
    })
}

fn authenticated(request: &Request, token: &str) -> bool {
    let query_token = query_value(request.url(), "token");
    let bearer = request
        .headers()
        .iter()
        .find_map(|header| {
            header
                .field
                .equiv("Authorization")
                .then(|| header.value.as_str())
        })
        .and_then(|value| value.strip_prefix("Bearer "));
    !token.is_empty()
        && (query_token.is_some_and(|candidate| secure_eq(candidate, token))
            || bearer.is_some_and(|candidate| secure_eq(candidate, token)))
}

fn secure_eq(candidate: &str, expected: &str) -> bool {
    if candidate.len() != expected.len() {
        return false;
    }
    candidate
        .bytes()
        .zip(expected.bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn unauthorized(request: Request) {
    let response = Response::from_string("{\"error\":\"unauthorized\"}")
        .with_status_code(StatusCode(401))
        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap())
        .with_header(Header::from_bytes(&b"WWW-Authenticate"[..], &b"Bearer"[..]).unwrap());
    let _ = request.respond(response);
}

fn respond_json(request: Request, status: u16, body: serde_json::Value) {
    let text = serde_json::to_string(&body).unwrap_or_else(|_| "{}".into());
    let response = Response::from_string(text)
        .with_status_code(StatusCode(status))
        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
    let _ = request.respond(response);
}

/// A tiny reader that yields SSE records and idle pings so the stream stays
/// open. Backed by a channel; the sender lives as long as the connection.
struct SseReader {
    rx: mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
    offset: usize,
    session: String,
    sessions: Sessions,
}

impl Drop for SseReader {
    fn drop(&mut self) {
        self.sessions.lock().unwrap().remove(&self.session);
    }
}

impl std::io::Read for SseReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.offset >= self.pending.len() {
            self.pending = match self.rx.recv_timeout(Duration::from_secs(12)) {
                Ok(bytes) => bytes,
                // SSE comments are standards-compliant keepalives.
                Err(mpsc::RecvTimeoutError::Timeout) => b": keepalive\n\n".to_vec(),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
            };
            self.offset = 0;
        }
        let remaining = &self.pending[self.offset..];
        let n = remaining.len().min(buf.len());
        buf[..n].copy_from_slice(&remaining[..n]);
        self.offset += n;
        Ok(n)
    }
}

fn tools() -> serde_json::Value {
    json!([
    {
        "name": "search_memory",
        "description": "Keyword search over captured local memory. Returns matching memories with content and timestamps.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "limit": { "type": "number", "description": "Max results (default 10)" }
            },
            "required": ["query"]
        }
    },
    {
        "name": "list_active_threads",
        "description": "List active work threads with open loops, for understanding what the user has been doing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "limit": { "type": "number", "description": "Max threads (default 20)" }
            }
        }
    },
    {
        "name": "get_latest_context",
        "description": "The most recent captured context — recent memories across all apps.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "limit": { "type": "number", "description": "Max memories (default 10)" }
            }
        }
    },
    {
        "name": "meeting_memory",
        "description": "Recent meeting transcripts and their action items.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "limit": { "type": "number", "description": "Max meetings (default 10)" }
            }
        }
    },
    {
        "name": "query_agent_items",
        "description": "Query the user's action items (open loops) by status.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["open", "resolved", "dismissed"], "description": "Status filter (default open)" }
            }
        }
    },
    {
        "name": "conversation_timeline",
        "description": "List incremental chat conversations, or retrieve timestamped new-message deltas for one conversation.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "conversationId": { "type": "number", "description": "Optional conversation ID; omit to list conversations" },
                "limit": { "type": "number", "description": "Maximum conversations or events" }
            }
        }
    },
    {
        "name": "get_thread_eras",
        "description": "Hourly snapshots (eras) of a thread's state — reconstruct what was on screen at a given time.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "threadId": { "type": "number", "description": "Thread (root memory) id" }
            },
            "required": ["threadId"]
        }
    }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_sse_session_endpoint_queries() {
        let url = "/messages?session_id=s-123&token=abc123";
        assert_eq!(query_value(url, "session_id"), Some("s-123"));
        assert_eq!(query_value(url, "token"), Some("abc123"));
        assert_eq!(query_value(url, "missing"), None);
    }

    #[test]
    fn access_tokens_require_an_exact_match() {
        assert!(secure_eq("0123456789abcdef", "0123456789abcdef"));
        assert!(!secure_eq("0123456789abcdee", "0123456789abcdef"));
        assert!(!secure_eq("short", "0123456789abcdef"));
    }
}
