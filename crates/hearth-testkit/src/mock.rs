//! A scriptable OpenAI-compatible model server.
//!
//! Each chat request consumes the next [`Step`] of a [`Script`]. A step can check what Hearth
//! sent (`expect`) and replies the way llama-server does, streamed in its exact SSE format.
//! Faults (HTTP errors, dropped connections, odd framing) and hold points (the reply pauses until
//! the test releases it) make awkward cases reproducible.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Script {
    pub model: String,
    pub n_ctx: u64,
    pub vision: bool,
    pub steps: Vec<Step>,
    /// Answers to `GET /search?q=…&format=json`, like a SearXNG server (`{ title, url, content }`).
    pub search: Vec<Value>,
    /// Web pages served at `/pages/<name>` (HTML).
    pub pages: HashMap<String, String>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            model: "mock-model".into(),
            n_ctx: 32_768,
            vision: false,
            steps: Vec::new(),
            search: Vec::new(),
            pages: HashMap::new(),
        }
    }
}

impl Script {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a step that replies without checking the request.
    pub fn reply(mut self, reply: Reply) -> Self {
        self.steps.push(Step {
            name: None,
            expect: Vec::new(),
            reply,
        });
        self
    }

    /// Adds a step that first checks the request.
    pub fn expect_then(mut self, expect: Vec<Expect>, reply: Reply) -> Self {
        self.steps.push(Step {
            name: None,
            expect,
            reply,
        });
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub name: Option<String>,
    pub expect: Vec<Expect>,
    pub reply: Reply,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallSpec {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
    /// Sent instead of `arguments`, byte for byte (for malformed-JSON cases).
    #[serde(default)]
    pub raw_arguments: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamOpts {
    /// Characters per content, reasoning or argument chunk.
    pub chunk_chars: usize,
    pub delay_ms: u64,
    /// Pause before the finishing chunk until the test calls `release(name)`.
    pub hold: Option<String>,
}

impl Default for StreamOpts {
    fn default() -> Self {
        Self {
            chunk_chars: 4,
            delay_ms: 0,
            hold: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Reply {
    pub content: Option<String>,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCallSpec>,
    pub finish_reason: Option<String>,
    pub stream: StreamOpts,
    pub usage: Option<Value>,
    pub timings: Option<Value>,
    pub fault: Option<Fault>,
}

impl Reply {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            ..Self::default()
        }
    }

    pub fn tool_call(name: impl Into<String>, arguments: Value) -> Self {
        Self::default().and_tool_call(name, arguments)
    }

    pub fn fault(fault: Fault) -> Self {
        Self {
            fault: Some(fault),
            ..Self::default()
        }
    }

    pub fn and_tool_call(mut self, name: impl Into<String>, arguments: Value) -> Self {
        self.tool_calls.push(ToolCallSpec {
            name: name.into(),
            arguments,
            raw_arguments: None,
            id: None,
        });
        self
    }

    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        self.reasoning = Some(reasoning.into());
        self
    }

    pub fn with_fault(mut self, fault: Fault) -> Self {
        self.fault = Some(fault);
        self
    }

    pub fn chunked(mut self, chars: usize) -> Self {
        self.stream.chunk_chars = chars.max(1);
        self
    }

    pub fn delayed(mut self, ms: u64) -> Self {
        self.stream.delay_ms = ms;
        self
    }

    pub fn held(mut self, name: impl Into<String>) -> Self {
        self.stream.hold = Some(name.into());
        self
    }

    pub fn usage(mut self, prompt_tokens: u64, completion_tokens: u64) -> Self {
        self.usage = Some(json!({
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens,
        }));
        self
    }

    pub fn timings(mut self, timings: Value) -> Self {
        self.timings = Some(timings);
        self
    }
}

/// Ways a reply can go wrong.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    /// Answer with this status and JSON body instead of a reply.
    Http { status: u16, body: Value },
    /// llama-server's "request exceeds the available context size" error.
    ContextExceeded { n_ctx: u64, n_prompt: u64 },
    /// llama-server's 503 while the model loads.
    Loading,
    /// Drop the connection after this many SSE events.
    DisconnectAfterEvents(usize),
    /// Send these SSE events verbatim (each is a whole `data: …` payload).
    RawSse(Vec<String>),
    /// Leave out the final `data: [DONE]`.
    NoDone,
    /// Use `\r\n` line endings.
    CrlfSse,
    /// Split every event in two, inside a multi-byte character where there is one.
    SplitUtf8,
}

/// Checks on the request a step receives. A failed check answers HTTP 500
/// `mock expectation failed: …`, so the test sees what went wrong.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expect {
    SystemContains(String),
    LastUserContains(String),
    LastToolResultContains(String),
    HasTool(String),
    NoTool(String),
    /// A dot path into the request JSON and its expected value.
    Param(String, Value),
    MessageCount {
        min: Option<usize>,
        max: Option<usize>,
    },
    ImageCount(usize),
    /// No assistant message before the last user message carries reasoning.
    NoReasoningBeforeLastUser,
}

struct Shared {
    script: Mutex<Script>,
    cursor: Mutex<usize>,
    requests: Mutex<Vec<Value>>,
    released: Mutex<HashSet<String>>,
    disconnects: AtomicUsize,
    /// Answer like llama-server while it loads a model (503 everywhere).
    loading: std::sync::atomic::AtomicBool,
}

async fn health(State(shared): State<Arc<Shared>>) -> Response {
    if shared.loading.load(std::sync::atomic::Ordering::SeqCst) {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "Loading model");
    }
    Json(json!({ "status": "ok" })).into_response()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A running mock server on `127.0.0.1`. Stops when dropped.
pub struct MockServer {
    /// Server root, e.g. `http://127.0.0.1:53211`.
    pub url: String,
    pub port: u16,
    shared: Arc<Shared>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl MockServer {
    pub async fn start(script: Script) -> Self {
        Self::start_on(script, 0)
            .await
            .expect("mock server could not start")
    }

    /// Starts on `port` (0 picks a free one).
    pub async fn start_on(script: Script, port: u16) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            script: Mutex::new(script),
            cursor: Mutex::new(0),
            requests: Mutex::new(Vec::new()),
            released: Mutex::new(HashSet::new()),
            disconnects: AtomicUsize::new(0),
            loading: std::sync::atomic::AtomicBool::new(false),
        });
        let app = Router::new()
            .route("/v1/chat/completions", post(chat))
            .route("/v1/models", get(models))
            .route("/props", get(props))
            .route("/health", get(health))
            .route("/tokenize", post(tokenize))
            .route("/search", get(search))
            .route("/mcp", post(mcp))
            .route("/pages/{name}", get(page))
            .route("/__mock/script", post(set_script))
            .route("/__mock/release/{hold}", post(release))
            .route("/__mock/requests", get(requests))
            .route("/__mock/reset", post(reset))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        let port = listener.local_addr()?.port();
        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        Ok(Self {
            url: format!("http://127.0.0.1:{port}"),
            port,
            shared,
            shutdown: Some(tx),
        })
    }

    /// While loading, `/health` answers 503 like llama-server's does.
    pub fn set_loading(&self, loading: bool) {
        self.shared
            .loading
            .store(loading, std::sync::atomic::Ordering::SeqCst);
    }

    /// The OpenAI-compatible API base (`…/v1`).
    pub fn api_base(&self) -> String {
        format!("{}/v1", self.url)
    }

    /// Every chat request received so far, as JSON.
    pub fn requests(&self) -> Vec<Value> {
        lock(&self.shared.requests).clone()
    }

    pub fn last_request(&self) -> Option<Value> {
        lock(&self.shared.requests).last().cloned()
    }

    /// Lets replies held at `hold` continue.
    pub fn release(&self, hold: &str) {
        lock(&self.shared.released).insert(hold.to_owned());
    }

    pub fn set_script(&self, script: Script) {
        *lock(&self.shared.script) = script;
        *lock(&self.shared.cursor) = 0;
    }

    /// How many streamed replies the client abandoned before they finished.
    pub fn disconnects(&self) -> usize {
        self.shared.disconnects.load(Ordering::SeqCst)
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": { "code": status.as_u16(), "message": message, "type": "mock_error" } })))
        .into_response()
}

async fn chat(State(shared): State<Arc<Shared>>, Json(body): Json<Value>) -> Response {
    lock(&shared.requests).push(body.clone());
    let (step, model) = {
        let script = lock(&shared.script);
        let mut cursor = lock(&shared.cursor);
        match script.steps.get(*cursor) {
            Some(step) => {
                *cursor += 1;
                (step.clone(), script.model.clone())
            }
            None => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "mock script exhausted");
            }
        }
    };
    if let Err(problem) = check_expectations(&step.expect, &body) {
        let name = step.name.as_deref().unwrap_or("unnamed step");
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("mock expectation failed ({name}): {problem}"),
        );
    }
    match &step.reply.fault {
        Some(Fault::Http { status, body }) => {
            let status = StatusCode::from_u16(*status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            return (status, Json(body.clone())).into_response();
        }
        Some(Fault::ContextExceeded { n_ctx, n_prompt }) => {
            let message = format!(
                "the request exceeds the available context size ({n_prompt} tokens, the limit is {n_ctx})"
            );
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": {
                    "code": 400, "type": "exceed_context_size_error", "message": message,
                    "n_prompt_tokens": n_prompt, "n_ctx": n_ctx,
                } })),
            )
                .into_response();
        }
        Some(Fault::Loading) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": { "code": 503, "message": "Loading model", "type": "unavailable_error" } })),
            )
                .into_response();
        }
        _ => {}
    }
    let prompt_chars: usize = body
        .get("messages")
        .and_then(Value::as_array)
        .map(|messages| {
            messages
                .iter()
                .map(|m| text_of(m.get("content")).len())
                .sum()
        })
        .unwrap_or(0);
    let streamed = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    if streamed {
        stream_reply(shared, step.reply, model, prompt_chars)
    } else {
        Json(completion(&step.reply, &model, prompt_chars)).into_response()
    }
}

fn arguments_of(call: &ToolCallSpec) -> String {
    call.raw_arguments.clone().unwrap_or_else(|| {
        if call.arguments.is_null() {
            "{}".into()
        } else {
            call.arguments.to_string()
        }
    })
}

fn finish_reason(reply: &Reply) -> String {
    reply.finish_reason.clone().unwrap_or_else(|| {
        if reply.tool_calls.is_empty() {
            "stop"
        } else {
            "tool_calls"
        }
        .into()
    })
}

fn usage_of(reply: &Reply, prompt_chars: usize) -> Value {
    reply.usage.clone().unwrap_or_else(|| {
        let completion_chars = reply.content.as_deref().map_or(0, str::len)
            + reply.reasoning.as_deref().map_or(0, str::len)
            + reply.tool_calls.iter().map(|c| arguments_of(c).len()).sum::<usize>();
        let prompt = (prompt_chars / 4).max(1) as u64;
        let completion = (completion_chars / 4).max(1) as u64;
        json!({ "prompt_tokens": prompt, "completion_tokens": completion, "total_tokens": prompt + completion })
    })
}

fn completion(reply: &Reply, model: &str, prompt_chars: usize) -> Value {
    let tool_calls: Vec<Value> = reply
        .tool_calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            json!({
                "id": call.id.clone().unwrap_or_else(|| format!("call_{index}")),
                "type": "function",
                "function": { "name": call.name, "arguments": arguments_of(call) },
            })
        })
        .collect();
    let mut message =
        json!({ "role": "assistant", "content": reply.content.clone().unwrap_or_default() });
    if let Some(reasoning) = &reply.reasoning {
        message["reasoning_content"] = json!(reasoning);
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }
    let mut out = json!({
        "id": "chatcmpl-mock", "object": "chat.completion", "created": 0, "model": model,
        "choices": [{ "index": 0, "message": message, "finish_reason": finish_reason(reply) }],
        "usage": usage_of(reply, prompt_chars),
    });
    if let Some(timings) = &reply.timings {
        out["timings"] = timings.clone();
    }
    out
}

enum Frame {
    Data(Value),
    Raw(String),
    Hold(String),
    Done,
}

fn pieces(text: &str, size: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(size.max(1))
        .map(|c| c.iter().collect())
        .collect()
}

fn frames(reply: &Reply, model: &str, prompt_chars: usize) -> Vec<Frame> {
    if let Some(Fault::RawSse(raw)) = &reply.fault {
        return raw.iter().cloned().map(Frame::Raw).collect();
    }
    let chunk = |delta: Value, finish: Option<String>| {
        Frame::Data(json!({
            "id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": 0, "model": model,
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
        }))
    };
    let size = reply.stream.chunk_chars;
    let mut out = vec![chunk(json!({ "role": "assistant", "content": null }), None)];
    if let Some(reasoning) = &reply.reasoning {
        out.extend(
            pieces(reasoning, size)
                .into_iter()
                .map(|p| chunk(json!({ "reasoning_content": p }), None)),
        );
    }
    if let Some(content) = &reply.content {
        out.extend(
            pieces(content, size)
                .into_iter()
                .map(|p| chunk(json!({ "content": p }), None)),
        );
    }
    for (index, call) in reply.tool_calls.iter().enumerate() {
        let id = call.id.clone().unwrap_or_else(|| format!("call_{index}"));
        out.push(chunk(
            json!({ "tool_calls": [{ "index": index, "id": id, "type": "function",
                    "function": { "name": call.name, "arguments": "" } }] }),
            None,
        ));
        for piece in pieces(&arguments_of(call), size) {
            out.push(chunk(
                json!({ "tool_calls": [{ "index": index, "function": { "arguments": piece } }] }),
                None,
            ));
        }
    }
    if let Some(hold) = &reply.stream.hold {
        out.push(Frame::Hold(hold.clone()));
    }
    out.push(chunk(json!({}), Some(finish_reason(reply))));
    let mut last = json!({
        "id": "chatcmpl-mock", "object": "chat.completion.chunk", "created": 0, "model": model,
        "choices": [], "usage": usage_of(reply, prompt_chars),
    });
    if let Some(timings) = &reply.timings {
        last["timings"] = timings.clone();
    }
    out.push(Frame::Data(last));
    if !matches!(reply.fault, Some(Fault::NoDone)) {
        out.push(Frame::Done);
    }
    out
}

/// Counts a client that went away before the reply finished.
struct HangUpGuard {
    shared: Arc<Shared>,
    finished: bool,
}

impl Drop for HangUpGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.shared.disconnects.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn stream_reply(shared: Arc<Shared>, reply: Reply, model: String, prompt_chars: usize) -> Response {
    let frames = frames(&reply, &model, prompt_chars);
    let newline = if matches!(reply.fault, Some(Fault::CrlfSse)) {
        "\r\n"
    } else {
        "\n"
    };
    let split = matches!(reply.fault, Some(Fault::SplitUtf8));
    let disconnect_after = match reply.fault {
        Some(Fault::DisconnectAfterEvents(n)) => Some(n),
        _ => None,
    };
    let delay = Duration::from_millis(reply.stream.delay_ms);
    let body = async_stream::stream! {
        let mut guard = HangUpGuard { shared: shared.clone(), finished: false };
        let mut sent = 0usize;
        for frame in frames {
            if disconnect_after.is_some_and(|limit| sent >= limit) {
                guard.finished = true; // a scripted fault, not a client hang-up
                // Let what was sent so far reach the client, then break the connection.
                tokio::time::sleep(Duration::from_millis(30)).await;
                yield Err(io::Error::new(io::ErrorKind::ConnectionReset, "mock disconnect"));
                return;
            }
            let text = match frame {
                Frame::Hold(name) => {
                    loop {
                        if lock(&shared.released).contains(&name) {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    continue;
                }
                Frame::Data(value) => format!("data: {value}{newline}{newline}"),
                Frame::Raw(raw) => raw,
                Frame::Done => format!("data: [DONE]{newline}{newline}"),
            };
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            sent += 1;
            if split {
                let bytes = text.into_bytes();
                let cut = bytes.iter().position(|b| (0x80..0xC0).contains(b)).unwrap_or(bytes.len() / 2).max(1);
                let (first, second) = bytes.split_at(cut.min(bytes.len()));
                yield Ok::<Bytes, io::Error>(Bytes::copy_from_slice(first));
                tokio::time::sleep(Duration::from_millis(2)).await;
                yield Ok(Bytes::copy_from_slice(second));
            } else {
                yield Ok(Bytes::from(text));
            }
        }
        guard.finished = true;
    };
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(body))
        .unwrap_or_else(|_| {
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not build the stream",
            )
        })
}

/// The text of a message's `content` (a string or parts).
fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn check_expectations(expectations: &[Expect], body: &Value) -> Result<(), String> {
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let role = |m: &Value| {
        m.get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let last_index_of = |wanted: &str| messages.iter().rposition(|m| role(m) == wanted);
    let tools: Vec<String> = body
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| {
                    t.pointer("/function/name")
                        .and_then(Value::as_str)
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    for expectation in expectations {
        match expectation {
            Expect::SystemContains(needle) => {
                let system = messages
                    .iter()
                    .find(|m| role(m) == "system")
                    .map(|m| text_of(m.get("content")));
                if !system
                    .as_deref()
                    .unwrap_or_default()
                    .contains(needle.as_str())
                {
                    return Err(format!("the system prompt doesn't contain {needle:?}"));
                }
            }
            Expect::LastUserContains(needle) => {
                let text = last_index_of("user")
                    .map(|i| text_of(messages[i].get("content")))
                    .unwrap_or_default();
                if !text.contains(needle.as_str()) {
                    return Err(format!(
                        "the last user message {text:?} doesn't contain {needle:?}"
                    ));
                }
            }
            Expect::LastToolResultContains(needle) => {
                let text = last_index_of("tool")
                    .map(|i| text_of(messages[i].get("content")))
                    .unwrap_or_default();
                if !text.contains(needle.as_str()) {
                    return Err(format!(
                        "the last tool result {text:?} doesn't contain {needle:?}"
                    ));
                }
            }
            Expect::HasTool(name) => {
                if !tools.contains(name) {
                    return Err(format!("tool {name:?} wasn't offered (offered: {tools:?})"));
                }
            }
            Expect::NoTool(name) => {
                if tools.contains(name) {
                    return Err(format!("tool {name:?} was offered but shouldn't be"));
                }
            }
            Expect::Param(path, expected) => {
                let pointer = format!("/{}", path.replace('.', "/"));
                let actual = body.pointer(&pointer).cloned().unwrap_or(Value::Null);
                if &actual != expected {
                    return Err(format!("{path} is {actual}, expected {expected}"));
                }
            }
            Expect::MessageCount { min, max } => {
                let count = messages.len();
                if min.is_some_and(|min| count < min) || max.is_some_and(|max| count > max) {
                    return Err(format!(
                        "{count} messages, expected between {min:?} and {max:?}"
                    ));
                }
            }
            Expect::ImageCount(expected) => {
                let count = messages
                    .iter()
                    .filter_map(|m| m.get("content").and_then(Value::as_array))
                    .flatten()
                    .filter(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
                    .count();
                if count != *expected {
                    return Err(format!("{count} images, expected {expected}"));
                }
            }
            Expect::NoReasoningBeforeLastUser => {
                let last_user = last_index_of("user").unwrap_or(0);
                if messages[..last_user]
                    .iter()
                    .any(|m| m.get("reasoning_content").is_some_and(|r| !r.is_null()))
                {
                    return Err("an earlier assistant message still carries its reasoning".into());
                }
            }
        }
    }
    Ok(())
}

async fn models(State(shared): State<Arc<Shared>>) -> Json<Value> {
    let model = lock(&shared.script).model.clone();
    Json(
        json!({ "object": "list", "data": [{ "id": model, "object": "model", "owned_by": "hearth-mock" }] }),
    )
}

/// A Streamable HTTP MCP server with the test tools (JSON answers, a session id).
async fn mcp(Json(message): Json<Value>) -> Response {
    match crate::mcp::answer(&message) {
        Some(answer) => ([("mcp-session-id", "fake-session")], Json(answer)).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// A SearXNG-style search: the scripted results, whatever the query.
async fn search(State(shared): State<Arc<Shared>>) -> Json<Value> {
    Json(json!({ "results": lock(&shared.script).search }))
}

async fn page(State(shared): State<Arc<Shared>>, Path(name): Path<String>) -> Response {
    match lock(&shared.script).pages.get(&name) {
        Some(html) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html.clone(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "no such page").into_response(),
    }
}

async fn props(State(shared): State<Arc<Shared>>) -> Json<Value> {
    let script = lock(&shared.script);
    Json(json!({
        "default_generation_settings": { "n_ctx": script.n_ctx },
        "model_path": format!("{}.gguf", script.model),
        "modalities": { "vision": script.vision, "audio": false },
        "chat_template_caps": { "supports_tools": true },
    }))
}

async fn tokenize(Json(body): Json<Value>) -> Json<Value> {
    let chars = body
        .get("content")
        .and_then(Value::as_str)
        .map_or(0, str::len);
    let tokens: Vec<u64> = (0..chars.div_ceil(4) as u64).collect();
    Json(json!({ "tokens": tokens }))
}

async fn set_script(State(shared): State<Arc<Shared>>, Json(script): Json<Script>) -> StatusCode {
    *lock(&shared.script) = script;
    *lock(&shared.cursor) = 0;
    StatusCode::NO_CONTENT
}

async fn release(State(shared): State<Arc<Shared>>, Path(hold): Path<String>) -> StatusCode {
    lock(&shared.released).insert(hold);
    StatusCode::NO_CONTENT
}

async fn requests(State(shared): State<Arc<Shared>>) -> Json<Value> {
    Json(Value::Array(lock(&shared.requests).clone()))
}

async fn reset(State(shared): State<Arc<Shared>>) -> StatusCode {
    lock(&shared.requests).clear();
    lock(&shared.released).clear();
    *lock(&shared.cursor) = 0;
    StatusCode::NO_CONTENT
}
