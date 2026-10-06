//! The local MCP tool surface — the "LLM brain" primitives that both the
//! in-app `mcp_call` command and the local HTTP MCP server delegate to.

use serde_json::json;

use crate::db;
use crate::db::Db;

pub const PROTOCOL_VERSION: &str = "2025-03-26";

/// Execute one of the local MCP tools against the database and return a
/// structured JSON result, sealing against malformed arguments.
pub fn call_tool(
    db: &Db,
    tool: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    match tool {
        "search_memory" => {
            let query = args
                .get("query")
                .and_then(|q| q.as_str())
                .unwrap_or("")
                .to_string();
            let limit = args.get("limit").and_then(|l| l.as_i64()).unwrap_or(10);
            let results = db.search_text(&query, limit)?;
            let json: Vec<serde_json::Value> = results.iter().map(memory_to_json).collect();
            Ok(json!({ "results": json }))
        }
        "list_active_threads" => {
            let limit = args.get("limit").and_then(|l| l.as_i64()).unwrap_or(20);
            let threads = db.list_threads(limit)?;
            let open: Vec<serde_json::Value> = threads
                .iter()
                .filter(|t| t.open_count > 0)
                .map(|t| {
                    json!({
                        "threadId": t.id,
                        "title": t.title,
                        "app": t.app,
                        "openItems": t.open_count,
                        "itemCount": t.item_count,
                        "derivative": t.derivative,
                    })
                })
                .collect();
            Ok(json!({ "activeThreads": open }))
        }
        "get_latest_context" => {
            let limit = args.get("limit").and_then(|l| l.as_i64()).unwrap_or(10);
            let memories = db.list_memories(limit, 0)?;
            let views: Vec<serde_json::Value> = memories.iter().map(memory_to_json).collect();
            Ok(json!({ "context": views }))
        }
        "meeting_memory" => {
            let limit = args.get("limit").and_then(|l| l.as_i64()).unwrap_or(10);
            let conversations = db.list_conversations(limit)?;
            let meetings: Vec<serde_json::Value> = conversations
                .iter()
                .filter(|c| c.transcript.is_some())
                .map(|c| {
                    json!({
                        "conversationId": c.id,
                        "title": c.title,
                        "startedAt": c.started_at,
                        "durationMs": c.duration_ms,
                        "transcript": c.transcript,
                    })
                })
                .collect();
            Ok(json!({ "meetings": meetings }))
        }
        "conversation_timeline" => {
            let conversation_id = args.get("conversationId").and_then(|value| value.as_i64());
            if let Some(conversation_id) = conversation_id {
                let limit = args
                    .get("limit")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(100);
                let events = db.list_conversation_events(conversation_id, limit)?;
                Ok(json!({ "conversationId": conversation_id, "events": events }))
            } else {
                let limit = args
                    .get("limit")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(50);
                let conversations = db.list_activity_conversations(limit)?;
                Ok(json!({ "conversations": conversations }))
            }
        }
        "query_agent_items" => {
            let status = args
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("open");
            let items = db.list_action_items(status)?;
            let json: Vec<serde_json::Value> = items
                .iter()
                .map(|i| {
                    json!({
                        "id": i.id,
                        "content": i.content,
                        "status": i.status,
                        "urgent": i.is_urgent,
                        "unread": i.is_unread,
                        "remindAt": i.remind_at,
                        "remindDate": i.remind_date,
                        "remindSlot": i.remind_slot,
                        "createdAt": i.created_at,
                    })
                })
                .collect();
            Ok(json!({ "items": json }))
        }
        "get_thread_eras" => {
            let thread_id = args.get("threadId").and_then(|t| t.as_i64()).unwrap_or(0);
            let versions = db.list_thread_versions(thread_id, 50)?;
            let json: Vec<serde_json::Value> = versions
                .iter()
                .map(|v| {
                    let items: Vec<serde_json::Value> =
                        serde_json::from_str(&v.content).unwrap_or_default();
                    json!({
                        "versionId": v.id,
                        "parentVersionId": v.parent_version_id,
                        "createdAt": v.created_at,
                        "frozen": v.is_frozen,
                        "wordCount": v.word_count,
                        "itemCount": items.len(),
                    })
                })
                .collect();
            Ok(json!({ "eras": json }))
        }
        _ => Err(format!("unknown MCP tool: {tool}")),
    }
}

fn memory_to_json(m: &db::MemoryRow) -> serde_json::Value {
    json!({
        "id": m.id,
        "source": m.source,
        "app": m.app,
        "title": m.title,
        "content": m.content,
        "createdAt": m.created_at,
        "isMeeting": m.is_meeting,
        "openLoop": m.open_loop,
    })
}
