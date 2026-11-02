//! Chat runs: a user message in, a streamed reply out, saved as it arrives.
//!
//! Sending adds the user message under the conversation's active leaf. Editing adds a new
//! version of a user message (a sibling). Regenerating adds a new version of a reply. The reply
//! streams to the UI in ~33 ms batches and is saved every second, so a crash loses at most a
//! moment of text. The streaming loop is shared with the coding agent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, json};
use tokio::time::Instant;

use crate::llm::think::ThinkSplitter;
use crate::llm::{
    Assembler, ChatMessage, ChatRequest, FinishReason, LlmClient, LlmError, Sampling, StreamEvent,
    ToolCall, Usage,
};
use crate::runs::{Run, RunEnd, RunEvent, RunManager, RunSink};
use crate::runtime::Runtime;
use crate::settings::{ModelRef, Settings, SettingsStore};
use crate::store::{
    Message, MessageFinish, MessageRole, MessageStatus, Mode, NewMessage, Session, SessionPatch,
    Store, StoreError,
};

/// How often streamed text is pushed to the UI, and saved.
const BATCH: Duration = Duration::from_millis(33);
const SAVE_EVERY: Duration = Duration::from_secs(1);

pub const DEFAULT_SYSTEM_PROMPT: &str = "You are Hearth, a helpful assistant running locally on the user's own computer. \
Answer clearly and directly. Use Markdown when it helps: short headings and lists for structure, fenced code blocks \
with a language tag for code, and LaTeX between $…$ or $$…$$ for math. If you don't know something, say so.";

/// Where a model's server is.
#[derive(Clone)]
pub enum Endpoint {
    /// A server Hearth talks to (a connection).
    Remote(LlmClient),
    /// A model file Hearth runs itself: its llama-server starts when first needed.
    Local { runtime: Runtime, model_id: String },
}

/// A model ready to use: where its server is and the model name to ask for.
#[derive(Clone)]
pub struct ResolvedModel {
    pub endpoint: Endpoint,
    pub model: String,
    /// Shown in the UI and saved with each reply.
    pub label: String,
    /// The server is llama.cpp's, so its extensions can be used.
    pub llama_extensions: bool,
}

impl ResolvedModel {
    /// A client for the model's server, starting it first (and saying so) when it's a local
    /// model that isn't loaded.
    pub async fn connect(&self, run: &Run) -> Result<LlmClient, LlmError> {
        match &self.endpoint {
            Endpoint::Remote(client) => Ok(client.clone()),
            Endpoint::Local { runtime, model_id } => {
                if !runtime.is_ready(model_id) {
                    run.emit(RunEvent::ModelLoading {
                        model: self.label.clone(),
                    });
                }
                runtime
                    .ensure(model_id, &run.cancel)
                    .await
                    .map_err(|message| {
                        if run.cancel.is_cancelled() {
                            LlmError::Cancelled
                        } else {
                            LlmError::Runtime(message)
                        }
                    })
            }
        }
    }
}

pub trait ModelResolver: Send + Sync {
    fn resolve(&self, model: &ModelRef) -> Result<ResolvedModel, String>;
}

/// Resolves local models through the runtime, and models on saved connections (one HTTP
/// client per connection).
pub struct StoreResolver {
    store: Arc<Store>,
    runtime: Option<Runtime>,
    clients: Mutex<HashMap<String, (String, LlmClient)>>,
}

impl StoreResolver {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            runtime: None,
            clients: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_runtime(store: Arc<Store>, runtime: Runtime) -> Self {
        Self {
            runtime: Some(runtime),
            ..Self::new(store)
        }
    }
}

impl ModelResolver for StoreResolver {
    fn resolve(&self, model_ref: &ModelRef) -> Result<ResolvedModel, String> {
        match model_ref {
            ModelRef::Local { model_id } => {
                let runtime = self
                    .runtime
                    .clone()
                    .ok_or("Local models aren't available here.")?;
                let model = runtime.model(model_id).ok_or(
                    "That model file isn't in the model folders any more. Choose another model.",
                )?;
                Ok(ResolvedModel {
                    endpoint: Endpoint::Local {
                        runtime,
                        model_id: model_id.clone(),
                    },
                    model: model_id.clone(),
                    label: model.name.clone(),
                    llama_extensions: true,
                })
            }
            ModelRef::Connection {
                connection_id,
                model,
            } => {
                let connection = self.store.connection(connection_id).map_err(|_| {
                    "The connection for this model was removed. Choose another model.".to_owned()
                })?;
                let fingerprint = format!(
                    "{}\u{0}{}",
                    connection.base_url,
                    connection.api_key.clone().unwrap_or_default()
                );
                let mut clients = self.clients.lock().unwrap_or_else(|p| p.into_inner());
                let client = match clients.get(connection_id) {
                    Some((known, client)) if *known == fingerprint => client.clone(),
                    _ => {
                        let client =
                            LlmClient::new(&connection.base_url, connection.api_key.clone())
                                .map_err(|e| e.to_string())?;
                        clients.insert(connection_id.clone(), (fingerprint, client.clone()));
                        client
                    }
                };
                Ok(ResolvedModel {
                    endpoint: Endpoint::Remote(client),
                    model: model.clone(),
                    label: model.clone(),
                    llama_extensions: connection.llama_extensions,
                })
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChatAction {
    Send { text: String },
    Regenerate { message_id: String },
    Edit { message_id: String, text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartedRun {
    pub run_id: String,
    pub user_message_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ChatError {
    #[error("Choose a model first: Settings → Models, or the model picker at the top.")]
    NoModel,
    #[error("{0}")]
    Model(String),
    #[error("{0}")]
    Busy(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Sampling and switches a chat can set (saved as the session's `params`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatParams {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Let models that can think do so (`None`: the model's default).
    pub thinking: Option<bool>,
}

impl ChatParams {
    pub fn of(session: &Session) -> Self {
        session
            .params
            .clone()
            .and_then(|p| serde_json::from_value(p).ok())
            .unwrap_or_default()
    }
}

/// The model a session uses: its own choice, else the default for its mode.
pub fn model_for(settings: &Settings, session: &Session) -> Option<ModelRef> {
    let own = session
        .model_ref
        .clone()
        .and_then(|value| serde_json::from_value::<ModelRef>(value).ok());
    own.or_else(|| match session.mode {
        Mode::Chat => settings.models.chat.clone(),
        Mode::Code => settings.models.code.clone(),
    })
}

#[derive(Clone)]
pub struct Chat {
    store: Arc<Store>,
    runs: Arc<RunManager>,
    resolver: Arc<dyn ModelResolver>,
    settings: Arc<SettingsStore>,
}

enum Plan {
    NewUser {
        parent: Option<String>,
        text: String,
    },
    Reply {
        parent: String,
    },
}

/// What one streamed reply came to.
#[derive(Default)]
pub(crate) struct Streamed {
    pub content: String,
    pub reasoning: String,
    pub reasoning_ms: Option<i64>,
    pub usage: Option<Usage>,
    pub finish: Option<FinishReason>,
    /// Complete tool calls (none when the stream broke off).
    pub tool_calls: Vec<ToolCall>,
    pub error: Option<LlmError>,
}

/// Text waiting to be pushed to the UI.
#[derive(Default)]
struct Pending {
    reasoning: String,
    content: String,
    /// The tool call whose arguments grew since the last push: (index, name, characters).
    call: Option<(usize, Option<String>, usize)>,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.reasoning.is_empty() && self.content.is_empty() && self.call.is_none()
    }

    fn flush(&mut self, run: &Run, message_id: &str) {
        if !self.reasoning.is_empty() {
            run.emit(RunEvent::ReasoningDelta {
                message_id: message_id.to_owned(),
                text: std::mem::take(&mut self.reasoning),
            });
        }
        if !self.content.is_empty() {
            run.emit(RunEvent::ContentDelta {
                message_id: message_id.to_owned(),
                text: std::mem::take(&mut self.content),
            });
        }
        if let Some((index, name, chars)) = self.call.take() {
            run.emit(RunEvent::ToolCallProgress {
                message_id: message_id.to_owned(),
                index,
                name,
                chars,
            });
        }
    }
}

impl Chat {
    pub fn new(
        store: Arc<Store>,
        runs: Arc<RunManager>,
        resolver: Arc<dyn ModelResolver>,
        settings: Arc<SettingsStore>,
    ) -> Self {
        Self {
            store,
            runs,
            resolver,
            settings,
        }
    }

    /// The model a session uses: its own choice, else the default for its mode.
    pub fn model_for(&self, session: &Session) -> Option<ModelRef> {
        model_for(&self.settings.get(), session)
    }

    /// Starts a reply. Returns once the user message is saved; the reply streams to `sink`.
    pub fn start(
        &self,
        session_id: &str,
        action: ChatAction,
        sink: Arc<dyn RunSink>,
    ) -> Result<StartedRun, ChatError> {
        let session = self.store.session(session_id)?;
        let model_ref = self.model_for(&session).ok_or(ChatError::NoModel)?;
        let model = self
            .resolver
            .resolve(&model_ref)
            .map_err(ChatError::Model)?;

        let plan = match action {
            ChatAction::Send { text } => {
                let text = text.trim();
                if text.is_empty() {
                    return Err(ChatError::Invalid("Type a message first.".into()));
                }
                Plan::NewUser {
                    parent: session.active_leaf_id.clone(),
                    text: text.to_owned(),
                }
            }
            ChatAction::Edit { message_id, text } => {
                let original = self.store.message(&message_id)?;
                if original.session_id != session.id || original.role != MessageRole::User {
                    return Err(ChatError::Invalid(
                        "Only your own messages in this conversation can be edited.".into(),
                    ));
                }
                let text = text.trim();
                if text.is_empty() {
                    return Err(ChatError::Invalid("The edited message is empty.".into()));
                }
                Plan::NewUser {
                    parent: original.parent_id,
                    text: text.to_owned(),
                }
            }
            ChatAction::Regenerate { message_id } => {
                let message = self.store.message(&message_id)?;
                if message.session_id != session.id {
                    return Err(ChatError::Invalid(
                        "That message isn't in this conversation.".into(),
                    ));
                }
                let parent = match message.role {
                    MessageRole::Assistant => message
                        .parent_id
                        .ok_or_else(|| ChatError::Invalid("Nothing to answer.".into()))?,
                    MessageRole::User => message.id,
                    _ => {
                        return Err(ChatError::Invalid(
                            "That message can't be regenerated.".into(),
                        ));
                    }
                };
                Plan::Reply { parent }
            }
        };

        let run = self
            .runs
            .begin(&session.id, sink)
            .map_err(ChatError::Busy)?;
        let (user_message_id, parent) = match plan {
            Plan::NewUser { parent, text } => {
                match self.store.insert_message(NewMessage::user(
                    &session.id,
                    parent.as_deref(),
                    &text,
                )) {
                    Ok(message) => (Some(message.id.clone()), message.id),
                    Err(err) => {
                        self.runs.finish(&run.id);
                        return Err(err.into());
                    }
                }
            }
            Plan::Reply { parent } => (None, parent),
        };
        keep_model(&self.store, &session, &model_ref);

        let started = StartedRun {
            run_id: run.id.clone(),
            user_message_id: user_message_id.clone(),
        };
        let this = self.clone();
        tokio::spawn(async move {
            let end = this
                .reply(&run, &session, &model, &parent, user_message_id)
                .await;
            run.emit(RunEvent::Finished { status: end });
            this.runs.finish(&run.id);
        });
        Ok(started)
    }

    async fn reply(
        &self,
        run: &Run,
        session: &Session,
        model: &ResolvedModel,
        parent: &str,
        user_message_id: Option<String>,
    ) -> RunEnd {
        run.emit(RunEvent::Started {
            session_id: session.id.clone(),
            user_message_id,
            model: model.label.clone(),
        });
        let history = match self.store.path_to(parent) {
            Ok(history) => history,
            Err(err) => return fail(run, "store", &err.to_string()),
        };
        let request = build_request(
            session,
            &self.settings.get(),
            &history,
            model,
            &ChatParams::of(session),
        );
        let message = match self.store.insert_message(NewMessage::assistant(
            &session.id,
            Some(parent),
            &model.label,
        )) {
            Ok(message) => message,
            Err(err) => return fail(run, "store", &err.to_string()),
        };
        run.emit(RunEvent::MessageStarted {
            message_id: message.id.clone(),
            parent_id: Some(parent.to_owned()),
        });
        // Named as soon as it starts, so the sidebar shows it right away.
        auto_title(&self.store, run, &session.id);

        let outcome = stream_reply(&self.store, run, &message.id, model, &request).await;

        let (status, end) = match &outcome.error {
            None => (MessageStatus::Complete, RunEnd::Completed),
            Some(LlmError::Cancelled) => (MessageStatus::Interrupted, RunEnd::Cancelled),
            Some(_) => (MessageStatus::Error, RunEnd::Failed),
        };
        let error = outcome
            .error
            .as_ref()
            .filter(|e| **e != LlmError::Cancelled);
        if let Some(err) = error {
            run.emit(RunEvent::Error {
                code: err.code().into(),
                message: err.to_string(),
                retryable: err.retryable(),
            });
        }
        let finish_reason = outcome.finish.as_ref().map(|f| f.as_str().to_owned());
        let usage_json = outcome
            .usage
            .as_ref()
            .and_then(|u| serde_json::to_value(u).ok());
        let finish = MessageFinish {
            content: outcome.content,
            reasoning: (!outcome.reasoning.is_empty()).then_some(outcome.reasoning),
            reasoning_ms: outcome.reasoning_ms,
            tool_calls: None,
            finish_reason: finish_reason.clone(),
            status,
            error: error.map(ToString::to_string),
            usage: usage_json,
        };
        if let Err(err) = self.store.finish_message(&message.id, &finish) {
            tracing::warn!("could not save the finished reply: {err}");
        }
        run.emit(RunEvent::MessageFinished {
            message_id: message.id,
            status: status_name(status).into(),
            finish_reason,
            usage: outcome.usage,
            reasoning_ms: outcome.reasoning_ms,
            content: None,
            tool_calls: Vec::new(),
        });
        end
    }
}

/// A conversation keeps the model it started with.
pub(crate) fn keep_model(store: &Store, session: &Session, model_ref: &ModelRef) {
    if session.model_ref.is_none() {
        let patch = SessionPatch {
            model_ref: serde_json::to_value(model_ref).ok(),
            ..Default::default()
        };
        let _ = store.update_session(&session.id, &patch);
    }
}

/// Streams one reply into an assistant message: batched to the UI, saved every second.
pub(crate) async fn stream_reply(
    store: &Store,
    run: &Run,
    message_id: &str,
    model: &ResolvedModel,
    request: &ChatRequest,
) -> Streamed {
    let mut out = Streamed::default();
    let client = match model.connect(run).await {
        Ok(client) => client,
        Err(err) => {
            out.error = Some(err);
            return out;
        }
    };
    let mut stream = match client.stream_chat(request, run.cancel.clone()).await {
        Ok(stream) => stream,
        Err(err) => {
            out.error = Some(err);
            return out;
        }
    };
    let mut splitter = ThinkSplitter::new(false);
    let mut calls = Assembler::new();
    let mut call_sizes: Vec<(Option<String>, usize)> = Vec::new();
    let mut pending = Pending::default();
    let mut thinking_since: Option<Instant> = None;
    let mut thinking_until: Option<Instant> = None;
    let mut last_emit = Instant::now();
    let mut last_save = Instant::now();

    loop {
        let item = if pending.is_empty() {
            Some(stream.next().await)
        } else {
            tokio::select! {
                item = stream.next() => Some(item),
                _ = tokio::time::sleep_until(last_emit + BATCH) => None,
            }
        };
        match item {
            None => {}
            Some(None) => break,
            Some(Some(Err(err))) => {
                out.error = Some(err);
                break;
            }
            Some(Some(Ok(event))) => match event {
                StreamEvent::Reasoning(text) => {
                    thinking_since.get_or_insert_with(Instant::now);
                    out.reasoning.push_str(&text);
                    pending.reasoning.push_str(&text);
                }
                StreamEvent::Content(text) => {
                    let split = splitter.push(&text);
                    if !split.reasoning.is_empty() {
                        thinking_since.get_or_insert_with(Instant::now);
                        out.reasoning.push_str(&split.reasoning);
                        pending.reasoning.push_str(&split.reasoning);
                    }
                    if !split.content.is_empty() {
                        if thinking_since.is_some() && thinking_until.is_none() {
                            thinking_until = Some(Instant::now());
                        }
                        out.content.push_str(&split.content);
                        pending.content.push_str(&split.content);
                    }
                }
                StreamEvent::PromptProgress {
                    processed,
                    total,
                    cached,
                } => {
                    run.emit(RunEvent::PromptProgress {
                        processed,
                        total,
                        cached,
                    });
                }
                StreamEvent::Usage(usage) => out.usage = Some(usage),
                StreamEvent::Finish(reason) => out.finish = Some(reason),
                StreamEvent::ToolCall {
                    index,
                    ref name,
                    ref arguments,
                    ..
                } => {
                    if thinking_since.is_some() && thinking_until.is_none() {
                        thinking_until = Some(Instant::now());
                    }
                    if call_sizes.len() <= index {
                        call_sizes.resize(index + 1, (None, 0));
                    }
                    let size = &mut call_sizes[index];
                    if let Some(name) = name.as_ref().filter(|n| !n.is_empty()) {
                        size.0 = Some(name.clone());
                    }
                    size.1 += arguments.chars().count();
                    pending.call = Some((index, size.0.clone(), size.1));
                    calls.push(&event);
                }
            },
        }
        if last_emit.elapsed() >= BATCH {
            pending.flush(run, message_id);
            last_emit = Instant::now();
        }
        if last_save.elapsed() >= SAVE_EVERY {
            let reasoning = (!out.reasoning.is_empty()).then_some(out.reasoning.as_str());
            let _ = store.update_streaming(message_id, &out.content, reasoning);
            last_save = Instant::now();
        }
    }
    let rest = splitter.finish();
    out.reasoning.push_str(&rest.reasoning);
    pending.reasoning.push_str(&rest.reasoning);
    out.content.push_str(&rest.content);
    pending.content.push_str(&rest.content);
    pending.flush(run, message_id);
    out.reasoning_ms = thinking_since.map(|since| {
        thinking_until
            .unwrap_or_else(Instant::now)
            .duration_since(since)
            .as_millis() as i64
    });
    if out.error.is_none() {
        out.tool_calls = calls.finish().tool_calls;
    }
    out
}

/// Names a new conversation after its first message (unless the user already named it).
pub(crate) fn auto_title(store: &Store, run: &Run, session_id: &str) {
    let Ok(current) = store.session(session_id) else {
        return;
    };
    if current.title_source != "none" {
        return;
    }
    let Ok(path) = store.active_path(session_id) else {
        return;
    };
    let Some(first) = path.iter().find(|m| m.role == MessageRole::User) else {
        return;
    };
    let title = title_from(&first.content);
    if title.is_empty() {
        return;
    }
    let patch = SessionPatch {
        title: Some(title.clone()),
        title_source: Some("auto".into()),
        ..Default::default()
    };
    if store.update_session(session_id, &patch).is_ok() {
        run.emit(RunEvent::TitleChanged {
            session_id: session_id.to_owned(),
            title,
        });
    }
}

pub(crate) fn fail(run: &Run, code: &str, message: &str) -> RunEnd {
    run.emit(RunEvent::Error {
        code: code.into(),
        message: message.into(),
        retryable: false,
    });
    RunEnd::Failed
}

pub(crate) fn status_name(status: MessageStatus) -> &'static str {
    match status {
        MessageStatus::Streaming => "streaming",
        MessageStatus::Complete => "complete",
        MessageStatus::Interrupted => "interrupted",
        MessageStatus::Error => "error",
    }
}

/// Sampling from the session, plus llama.cpp's extras when the server understands them.
pub(crate) fn apply_params(request: &mut ChatRequest, model: &ResolvedModel, params: &ChatParams) {
    request.sampling = Sampling {
        temperature: params.temperature,
        top_p: params.top_p,
        top_k: params.top_k,
        min_p: params.min_p,
        ..Default::default()
    };
    request.max_tokens = params.max_tokens;
    if model.llama_extensions {
        request.cache_prompt = Some(true);
        request.return_progress = Some(true);
        if let Some(thinking) = params.thinking {
            let mut kwargs = Map::new();
            kwargs.insert("enable_thinking".into(), json!(thinking));
            request.chat_template_kwargs = Some(kwargs);
        }
    }
}

fn build_request(
    session: &Session,
    settings: &Settings,
    history: &[Message],
    model: &ResolvedModel,
    params: &ChatParams,
) -> ChatRequest {
    let base = session
        .persona
        .clone()
        .filter(|p| !p.trim().is_empty())
        .or_else(|| {
            settings
                .chat
                .system_prompt
                .clone()
                .filter(|p| !p.trim().is_empty())
        })
        .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_owned());
    let mut messages = vec![ChatMessage::system(format!(
        "{base}\n\nToday's date: {}.",
        today()
    ))];
    for message in history {
        match message.role {
            MessageRole::User => messages.push(ChatMessage::user(message.content.as_str())),
            MessageRole::Assistant
                if message.status != MessageStatus::Error && !message.content.trim().is_empty() =>
            {
                messages.push(ChatMessage::assistant(message.content.as_str()));
            }
            MessageRole::Summary => messages.push(ChatMessage::user(format!(
                "(A summary of the earlier conversation)\n{}",
                message.content
            ))),
            _ => {}
        }
    }
    let mut request = ChatRequest {
        model: model.model.clone(),
        messages,
        ..Default::default()
    };
    apply_params(&mut request, model, params);
    request
}

/// A short title from the first line of a message.
pub fn title_from(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let cleaned = line
        .trim_start_matches(['#', '>', '-', '*', ' '])
        .replace(['`', '*', '_'], "");
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= 60 {
        return cleaned;
    }
    let mut title = String::new();
    for word in cleaned.split(' ') {
        if title.chars().count() + word.chars().count() + 1 > 57 {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(word);
    }
    if title.is_empty() {
        title = cleaned.chars().take(57).collect();
    }
    title.push('…');
    title
}

/// Today's date (UTC) as YYYY-MM-DD.
pub(crate) fn today() -> String {
    let days = crate::now_ms().div_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_come_from_the_first_line() {
        assert_eq!(
            title_from("  \n# How do **lifetimes** work?\nmore"),
            "How do lifetimes work?"
        );
        let long = "Explain the difference between processes and threads in operating systems with examples";
        let title = title_from(long);
        assert!(
            title.ends_with('…') && title.chars().count() <= 58,
            "{title}"
        );
        assert_eq!(title_from("   "), "");
    }

    #[test]
    fn calendar_dates_are_right() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_759), (2026, 11, 2));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}
