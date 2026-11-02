//! A tiny MCP server's answers, for tests: tools `echo`, `add` and `fail`. The
//! `fake-mcp-server` binary serves them over stdio, and the mock server at `POST /mcp`.

use serde_json::{Value, json};

fn tools() -> Value {
    json!([
        {
            "name": "echo",
            "description": "Says the text back.",
            "inputSchema": {
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }
        },
        {
            "name": "add",
            "description": "Adds two numbers.",
            "inputSchema": {
                "type": "object",
                "properties": { "a": { "type": "number" }, "b": { "type": "number" } },
                "required": ["a", "b"]
            }
        },
        {
            "name": "fail",
            "description": "Always fails.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

fn text(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }] })
}

/// The answer to one JSON-RPC message (`None` for notifications).
pub fn answer(message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let params = &message["params"];
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "fake-mcp", "version": "1.0.0" }
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => {
            let args = &params["arguments"];
            match params["name"].as_str().unwrap_or_default() {
                "echo" => text(format!(
                    "echo: {}",
                    args["text"].as_str().unwrap_or_default()
                )),
                "add" => {
                    let sum = args["a"].as_f64().unwrap_or(0.0) + args["b"].as_f64().unwrap_or(0.0);
                    text(sum.to_string())
                }
                "fail" => json!({
                    "content": [{ "type": "text", "text": "the tool failed on purpose" }],
                    "isError": true
                }),
                other => {
                    return Some(json!({ "jsonrpc": "2.0", "id": id, "error": {
                        "code": -32602, "message": format!("Unknown tool: {other}")
                    } }));
                }
            }
        }
        other => {
            return Some(json!({ "jsonrpc": "2.0", "id": id, "error": {
                "code": -32601, "message": format!("Method not found: {other}")
            } }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}
