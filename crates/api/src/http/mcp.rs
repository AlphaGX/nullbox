//! MCP server for AI agents at `/mcp` (Streamable HTTP, JSON responses). Behind a bearer
//! token from `MCP_TOKENS`, like every other door to customer data. Tools call `TicketService`.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::AppState;

const SUPPORTED_VERSIONS: [&str; 4] = ["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26"];
/// Replies sent through MCP appear in the app under this name.
const AGENT_NAME: &str = "Nullbox agent";

fn authorized(headers: &HeaderMap, state: &AppState) -> bool {
    let Some(presented) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
    else {
        return false;
    };
    // Compare digests, so the time taken says nothing about how much of a token matched.
    let presented = Sha256::digest(presented.as_bytes());
    state.config.mcp_tokens.iter().any(|token| Sha256::digest(token.as_bytes()) == presented)
}

fn unauthorized() -> Response {
    let mut res =
        (StatusCode::UNAUTHORIZED, Json(json!({ "code": "unauthorized", "message": "bearer token required" })))
            .into_response();
    res.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static(r#"Bearer realm="nullbox-mcp""#));
    res
}

pub async fn get(headers: HeaderMap, State(state): State<AppState>) -> Response {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

pub async fn post(headers: HeaderMap, State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return Json(error(Value::Null, -32700, "parse error")).into_response();
    };
    // A batch is answered as a batch; notifications get no answer.
    if let Value::Array(items) = message {
        let mut out = Vec::new();
        for item in items {
            if let Some(reply) = handle(&state, item).await {
                out.push(reply);
            }
        }
        return if out.is_empty() {
            StatusCode::ACCEPTED.into_response()
        } else {
            Json(Value::Array(out)).into_response()
        };
    }
    let is_initialize = message.get("method").and_then(Value::as_str) == Some("initialize");
    match handle(&state, message).await {
        None => StatusCode::ACCEPTED.into_response(),
        Some(reply) => {
            let mut res = Json(reply).into_response();
            if is_initialize {
                let session = Uuid::now_v7().to_string();
                if let Ok(value) = HeaderValue::from_str(&session) {
                    res.headers_mut().insert("mcp-session-id", value);
                }
            }
            res
        }
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

async fn handle(state: &AppState, message: Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let id = id?; // notifications (no id) need no answer
    let reply = match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
            let version = SUPPORTED_VERSIONS.iter().find(|v| **v == requested).copied().unwrap_or("2025-11-25");
            result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "nullbox", "title": "Nullbox help desk", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Nullbox help desk. Use list_tickets to see the inbox, get_ticket to read a ticket with its messages, and reply_to_ticket to answer the customer."
                }),
            )
        }
        "ping" => result(id, json!({})),
        "tools/list" => result(id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call(state, name, &args).await {
                Ok(value) => result(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default() }],
                        "structuredContent": value,
                        "isError": false
                    }),
                ),
                Err(message) => {
                    result(id, json!({ "content": [{ "type": "text", "text": message }], "isError": true }))
                }
            }
        }
        _ => error(id, -32601, "method not found"),
    };
    Some(reply)
}

fn tools() -> Value {
    json!([
        {
            "name": "list_tickets",
            "title": "List tickets",
            "description": "List tickets in the help desk inbox. Without a status, all active tickets (open, in_progress, waiting_on_customer).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["open", "in_progress", "waiting_on_customer", "resolved", "closed"] }
                }
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "get_ticket",
            "title": "Get ticket",
            "description": "Read one ticket with all its messages, including internal notes. Takes the ticket ID (a UUID).",
            "inputSchema": {
                "type": "object",
                "properties": { "ticket_id": { "type": "string", "description": "The ticket's ID (UUID)" } },
                "required": ["ticket_id"]
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "reply_to_ticket",
            "title": "Reply to ticket",
            "description": "Answer the customer on a ticket. Set internal to true for a note only workers can see.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ticket_id": { "type": "string", "description": "The ticket's ID (UUID)" },
                    "body": { "type": "string", "description": "The reply text" },
                    "internal": { "type": "boolean", "default": false }
                },
                "required": ["ticket_id", "body"]
            }
        }
    ])
}

fn ticket_id(args: &Value) -> Result<Uuid, String> {
    args.get("ticket_id")
        .and_then(Value::as_str)
        .ok_or("ticket_id is required")?
        .trim()
        .parse()
        .map_err(|_| "ticket_id must be a UUID".to_owned())
}

async fn call(state: &AppState, name: &str, args: &Value) -> Result<Value, String> {
    let tickets = &state.tickets;
    let to_value = |v: Result<Value, serde_json::Error>| v.map_err(|e| e.to_string());
    match name {
        "list_tickets" => {
            let status = match args.get("status").and_then(Value::as_str) {
                Some(s) => Some(serde_json::from_value(json!(s)).map_err(|_| format!("unknown status {s}"))?),
                None => None,
            };
            let list = tickets.list(status).await.map_err(|e| e.to_string())?;
            to_value(serde_json::to_value(list)).map(|tickets| json!({ "tickets": tickets }))
        }
        "get_ticket" => {
            let thread = tickets.worker_thread(ticket_id(args)?).await.map_err(|e| e.to_string())?;
            to_value(serde_json::to_value(thread))
        }
        "reply_to_ticket" => {
            let body = args.get("body").and_then(Value::as_str).ok_or("body is required")?;
            let internal = args.get("internal").and_then(Value::as_bool).unwrap_or(false);
            let message = tickets
                .worker_message(AGENT_NAME, ticket_id(args)?, body, internal)
                .await
                .map_err(|e| e.to_string())?;
            to_value(serde_json::to_value(message))
        }
        other => Err(format!("unknown tool {other}")),
    }
}
