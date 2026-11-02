//! A chat client for llama-server and any server with the same OpenAI-compatible API (LM Studio,
//! Ollama's `/v1`, vLLM, a remote machine).

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::assemble::AssistantReply;
use super::sse::{SseEvent, SseParser};
use super::types::{ChatRequest, FinishReason, LlmError, StreamEvent, ToolCall, Usage};

/// Health of a llama-server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ready,
    Loading,
}

#[derive(Debug, Clone)]
pub struct LlmClient {
    http: Client,
    /// Server root, e.g. `http://127.0.0.1:8080` (llama.cpp's `/health`, `/props` live here).
    root: String,
    /// API base, e.g. `http://127.0.0.1:8080/v1`.
    api: String,
    api_key: Option<String>,
}

impl LlmClient {
    /// `base_url` may be the server root (`http://127.0.0.1:8080`) or its API base (`…/v1`).
    /// Loopback servers are reached directly, never through a system proxy.
    pub fn new(base_url: &str, api_key: Option<String>) -> Result<Self, LlmError> {
        let (root, api) = normalize_base(base_url)?;
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(30));
        if is_loopback(&root) {
            builder = builder.no_proxy();
        }
        let http = builder
            .build()
            .map_err(|e| LlmError::Protocol(e.to_string()))?;
        Ok(Self {
            http,
            root,
            api,
            api_key: api_key.filter(|k| !k.is_empty()),
        })
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn api_base(&self) -> &str {
        &self.api
    }

    fn request(&self, method: Method, url: String) -> RequestBuilder {
        let builder = self.http.request(method, url);
        match &self.api_key {
            Some(key) => builder.bearer_auth(key),
            None => builder,
        }
    }

    fn send_error(&self, err: reqwest::Error) -> LlmError {
        if err.is_connect() || err.is_timeout() {
            LlmError::Unreachable {
                url: self.root.clone(),
                detail: short_cause(&err),
            }
        } else if err.is_request() || err.is_body() {
            // Connected, then the connection broke (server crashed or was stopped mid-reply).
            LlmError::ConnectionLost
        } else {
            LlmError::Protocol(short_cause(&err))
        }
    }

    /// Starts a streamed reply. The stream ends after the reply finishes; cancelling the token
    /// drops the connection, which tells the server to stop generating.
    pub async fn stream_chat(
        &self,
        request: &ChatRequest,
        cancel: CancellationToken,
    ) -> Result<ChatStream, LlmError> {
        let mut body = request.clone();
        body.stream = true;
        if body.stream_options.is_none() {
            body.stream_options = Some(json!({ "include_usage": true }));
        }
        let send = self
            .request(Method::POST, format!("{}/chat/completions", self.api))
            .json(&body)
            .send();
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(LlmError::Cancelled),
            response = send => response.map_err(|e| self.send_error(e))?,
        };
        let response = check(response).await?;
        Ok(ChatStream::new(response, cancel))
    }

    /// A whole reply at once (titles, summaries).
    pub async fn complete(&self, request: &ChatRequest) -> Result<AssistantReply, LlmError> {
        let mut body = request.clone();
        body.stream = false;
        body.stream_options = None;
        let response = self
            .request(Method::POST, format!("{}/chat/completions", self.api))
            .json(&body)
            .send()
            .await
            .map_err(|e| self.send_error(e))?;
        let value: Value = check(response)
            .await?
            .json()
            .await
            .map_err(|e| LlmError::Protocol(e.to_string()))?;
        parse_completion(&value)
    }

    /// Model ids the server offers.
    pub async fn models(&self) -> Result<Vec<String>, LlmError> {
        let response = self
            .request(Method::GET, format!("{}/models", self.api))
            .send()
            .await
            .map_err(|e| self.send_error(e))?;
        let value: Value = check(response)
            .await?
            .json()
            .await
            .map_err(|e| LlmError::Protocol(e.to_string()))?;
        Ok(value
            .get("data")
            .and_then(Value::as_array)
            .map(|models| {
                models
                    .iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str))
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// llama.cpp's server properties (context size, modalities, chat template capabilities).
    pub async fn props(&self) -> Result<Value, LlmError> {
        let response = self
            .request(Method::GET, format!("{}/props", self.root))
            .send()
            .await
            .map_err(|e| self.send_error(e))?;
        check(response)
            .await?
            .json()
            .await
            .map_err(|e| LlmError::Protocol(e.to_string()))
    }

    /// llama.cpp's `/health`: ready, or still loading the model.
    pub async fn health(&self) -> Result<Health, LlmError> {
        let response = self
            .request(Method::GET, format!("{}/health", self.root))
            .send()
            .await
            .map_err(|e| self.send_error(e))?;
        match check(response).await {
            Ok(_) => Ok(Health::Ready),
            Err(LlmError::Loading) => Ok(Health::Loading),
            Err(err) => Err(err),
        }
    }
}

/// A streamed reply: [`StreamEvent`]s until the reply finishes, or an error.
pub struct ChatStream {
    inner: Pin<Box<dyn Stream<Item = Result<StreamEvent, LlmError>> + Send>>,
}

impl ChatStream {
    fn new(response: Response, cancel: CancellationToken) -> Self {
        let inner = async_stream::stream! {
            let mut bytes = response.bytes_stream();
            let mut parser = SseParser::new();
            let mut decoder = Decoder::default();
            loop {
                let next = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        yield Err(LlmError::Cancelled);
                        return;
                    }
                    next = bytes.next() => next,
                };
                let (events, eof) = match next {
                    Some(Ok(chunk)) => (parser.push(&chunk), false),
                    Some(Err(err)) => {
                        tracing::debug!("stream read failed: {err}");
                        yield Err(LlmError::ConnectionLost);
                        return;
                    }
                    None => (parser.finish().into_iter().collect::<Vec<_>>(), true),
                };
                for event in events {
                    match decoder.decode(&event) {
                        Ok(decoded) => {
                            for item in decoded {
                                yield Ok(item);
                            }
                        }
                        Err(err) => {
                            yield Err(err);
                            return;
                        }
                    }
                    if decoder.done {
                        return;
                    }
                }
                if eof {
                    // The server closed the stream. Fine if the reply had finished (some
                    // servers skip `[DONE]`); otherwise the connection was lost.
                    if let Some(usage) = decoder.leftover_usage() {
                        yield Ok(StreamEvent::Usage(usage));
                    }
                    if !decoder.finished {
                        yield Err(LlmError::ConnectionLost);
                    }
                    return;
                }
            }
        };
        Self {
            inner: Box::pin(inner),
        }
    }
}

impl Stream for ChatStream {
    type Item = Result<StreamEvent, LlmError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

/// Turns SSE data payloads into stream events, remembering what the stream has said so far.
#[derive(Debug, Default)]
struct Decoder {
    /// A `finish_reason` arrived.
    finished: bool,
    /// `[DONE]` arrived.
    done: bool,
    usage_sent: bool,
    timings: Option<Value>,
}

impl Decoder {
    fn decode(&mut self, event: &SseEvent) -> Result<Vec<StreamEvent>, LlmError> {
        let data = event.data.trim();
        if data.is_empty() {
            return Ok(Vec::new());
        }
        if data == "[DONE]" {
            self.done = true;
            let mut out = Vec::new();
            if let Some(usage) = self.leftover_usage() {
                out.push(StreamEvent::Usage(usage));
            }
            if !self.finished {
                self.finished = true;
                out.push(StreamEvent::Finish(FinishReason::Stop));
            }
            return Ok(out);
        }
        let chunk: Value = serde_json::from_str(data)
            .map_err(|e| LlmError::Protocol(format!("bad JSON in stream: {e}")))?;
        if let Some(error) = chunk.get("error") {
            return Err(error_from_body(500, error));
        }
        let mut out = Vec::new();
        if let Some(progress) = chunk.get("prompt_progress") {
            let get = |key: &str| progress.get(key).and_then(Value::as_u64).unwrap_or(0);
            out.push(StreamEvent::PromptProgress {
                processed: get("processed"),
                total: get("total"),
                cached: get("cache"),
            });
        }
        if let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
        {
            let delta = choice
                .get("delta")
                .or_else(|| choice.get("message"))
                .unwrap_or(&Value::Null);
            let reasoning = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            if let Some(text) = reasoning {
                out.push(StreamEvent::Reasoning(text.to_owned()));
            }
            if let Some(text) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                out.push(StreamEvent::Content(text.to_owned()));
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for (position, call) in calls.iter().enumerate() {
                    let function = call.get("function").unwrap_or(&Value::Null);
                    out.push(StreamEvent::ToolCall {
                        index: call
                            .get("index")
                            .and_then(Value::as_u64)
                            .map_or(position, |i| i as usize),
                        id: call.get("id").and_then(Value::as_str).map(String::from),
                        name: function
                            .get("name")
                            .and_then(Value::as_str)
                            .map(String::from),
                        arguments: function
                            .get("arguments")
                            .map(arguments_text)
                            .unwrap_or_default(),
                    });
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finished = true;
                out.push(StreamEvent::Finish(FinishReason::parse(reason)));
            }
        }
        if let Some(timings) = chunk.get("timings").filter(|t| t.is_object()) {
            self.timings = Some(timings.clone());
        }
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            self.usage_sent = true;
            out.push(StreamEvent::Usage(usage_from(
                Some(usage),
                self.timings.as_ref(),
            )));
        }
        Ok(out)
    }

    /// Usage built from llama.cpp's timings when the server never sent a `usage` object.
    fn leftover_usage(&mut self) -> Option<Usage> {
        if self.usage_sent {
            return None;
        }
        let timings = self.timings.as_ref()?;
        self.usage_sent = true;
        Some(usage_from(None, Some(timings)))
    }
}

fn arguments_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(), // some servers send the arguments as an object
    }
}

fn usage_from(usage: Option<&Value>, timings: Option<&Value>) -> Usage {
    let number =
        |value: Option<&Value>, key: &str| value.and_then(|v| v.get(key)).and_then(Value::as_u64);
    let float =
        |value: Option<&Value>, key: &str| value.and_then(|v| v.get(key)).and_then(Value::as_f64);
    Usage {
        prompt_tokens: number(usage, "prompt_tokens")
            .or_else(|| number(timings, "prompt_n"))
            .unwrap_or(0),
        completion_tokens: number(usage, "completion_tokens")
            .or_else(|| number(timings, "predicted_n"))
            .unwrap_or(0),
        cached_tokens: number(timings, "cache_n").or_else(|| {
            usage
                .and_then(|u| u.get("prompt_tokens_details"))
                .and_then(|d| d.get("cached_tokens"))
                .and_then(Value::as_u64)
        }),
        prompt_per_second: float(timings, "prompt_per_second"),
        predicted_per_second: float(timings, "predicted_per_second"),
        context_window: None,
    }
}

fn parse_completion(value: &Value) -> Result<AssistantReply, LlmError> {
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .ok_or_else(|| LlmError::Protocol("reply has no choices".into()))?;
    let message = choice.get("message").unwrap_or(&Value::Null);
    let text = |key: &str| {
        message
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .enumerate()
                .filter_map(|(index, call)| {
                    let function = call.get("function")?;
                    let name = function.get("name").and_then(Value::as_str)?;
                    let id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .map_or_else(|| format!("call_{index}"), String::from);
                    Some(ToolCall::new(
                        id,
                        name,
                        function
                            .get("arguments")
                            .map(arguments_text)
                            .unwrap_or_default(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let reasoning = if message.get("reasoning_content").is_some() {
        text("reasoning_content")
    } else {
        text("reasoning")
    };
    Ok(AssistantReply {
        content: text("content"),
        reasoning,
        tool_calls,
        finish: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(FinishReason::parse),
        usage: value
            .get("usage")
            .map(|u| usage_from(Some(u), value.get("timings"))),
    })
}

/// Passes successful responses through and maps errors (llama-server sends
/// `{"error": {"code", "message", "type", …}}`).
async fn check(response: Response) -> Result<Response, LlmError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let text = response.text().await.unwrap_or_default();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let error = body
        .get("error")
        .cloned()
        .unwrap_or_else(|| json!({ "message": text.trim() }));
    Err(error_from_body(status.as_u16(), &error))
}

fn error_from_body(status: u16, error: &Value) -> LlmError {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or_default()
        .to_owned();
    let kind = error
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind == "exceed_context_size_error" || message.contains("exceeds the available context size")
    {
        let get = |key: &str| error.get(key).and_then(Value::as_u64).unwrap_or(0);
        return LlmError::ContextExceeded {
            n_prompt: get("n_prompt_tokens"),
            n_ctx: get("n_ctx"),
        };
    }
    match StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR) {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => LlmError::Unauthorized,
        StatusCode::SERVICE_UNAVAILABLE if message.to_ascii_lowercase().contains("loading") => {
            LlmError::Loading
        }
        _ => LlmError::Http {
            status,
            message: if message.is_empty() {
                "no details".into()
            } else {
                message
            },
        },
    }
}

/// Splits a user-entered URL into the server root and the API base.
fn normalize_base(input: &str) -> Result<(String, String), LlmError> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(LlmError::Protocol("the server address is empty".into()));
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    };
    let url = reqwest::Url::parse(&with_scheme)
        .map_err(|e| LlmError::Protocol(format!("bad server address: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(LlmError::Protocol(
            "the server address must start with http:// or https://".into(),
        ));
    }
    let base = with_scheme.trim_end_matches('/').to_owned();
    match base.strip_suffix("/v1") {
        Some(root) => Ok((root.to_owned(), base.clone())),
        None => Ok((base.clone(), format!("{base}/v1"))),
    }
}

fn is_loopback(root: &str) -> bool {
    reqwest::Url::parse(root)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| {
            host == "localhost" || host.starts_with("127.") || host == "[::1]" || host == "::1"
        })
}

fn short_cause(err: &reqwest::Error) -> String {
    let mut cause: &dyn std::error::Error = err;
    while let Some(next) = cause.source() {
        cause = next;
    }
    cause.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_are_normalized() {
        assert_eq!(
            normalize_base("127.0.0.1:8080").unwrap(),
            (
                "http://127.0.0.1:8080".into(),
                "http://127.0.0.1:8080/v1".into()
            )
        );
        assert_eq!(
            normalize_base("http://localhost:1234/v1/").unwrap(),
            (
                "http://localhost:1234".into(),
                "http://localhost:1234/v1".into()
            )
        );
        assert!(normalize_base("ftp://x").is_err());
        assert!(normalize_base("  ").is_err());
    }

    #[test]
    fn loopback_hosts_are_recognized() {
        assert!(is_loopback("http://127.0.0.1:8080"));
        assert!(is_loopback("http://localhost:8080"));
        assert!(!is_loopback("http://192.168.1.20:8080"));
    }

    #[test]
    fn llama_server_errors_map_to_clear_kinds() {
        let exceeded = json!({"code": 400, "type": "exceed_context_size_error", "message": "x", "n_prompt_tokens": 5000, "n_ctx": 4096});
        assert_eq!(
            error_from_body(400, &exceeded),
            LlmError::ContextExceeded {
                n_prompt: 5000,
                n_ctx: 4096
            }
        );
        assert_eq!(
            error_from_body(401, &json!({"message": "Invalid API Key"})),
            LlmError::Unauthorized
        );
        assert_eq!(
            error_from_body(503, &json!({"message": "Loading model"})),
            LlmError::Loading
        );
    }

    #[test]
    fn decodes_llama_cpp_chunks() {
        let mut decoder = Decoder::default();
        let event = |data: &str| SseEvent {
            event: None,
            data: data.into(),
        };
        let out = decoder
            .decode(&event(
                r#"{"choices":[{"index":0,"delta":{"reasoning_content":"hm","content":"Hi"}}]}"#,
            ))
            .unwrap();
        assert_eq!(
            out,
            [
                StreamEvent::Reasoning("hm".into()),
                StreamEvent::Content("Hi".into())
            ]
        );
        let out = decoder
            .decode(&event(r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"timings":{"predicted_per_second":42.5,"cache_n":10}}"#))
            .unwrap();
        assert_eq!(out, [StreamEvent::Finish(FinishReason::Stop)]);
        let out = decoder.decode(&event("[DONE]")).unwrap();
        assert!(
            matches!(&out[..], [StreamEvent::Usage(usage)] if usage.predicted_per_second == Some(42.5) && usage.cached_tokens == Some(10))
        );
    }
}
