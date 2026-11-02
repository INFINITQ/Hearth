//! The agent loop: the model works through a task with tools until it answers without one.
//!
//! Each step streams one assistant message. Its tool calls then run one at a time: prepared
//! (arguments checked, a diff worked out), checked against the session's permission mode (asking
//! the user when needed), executed, and answered with a tool message. The model gets the results
//! and carries on. Every tool call gets an answer, even when the user stops the run, so the
//! conversation is always valid to send again.
//!
//! Files are saved as checkpoints before their first change in a task, so a task can be undone.
//! Messages the user sends while the agent works ("steering") join the conversation before the
//! next step.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::changes::{self, FileChange};
use super::exec::{self, Processes};
use super::paths::Workspace;
use super::permissions::{self, Grants, PermissionMode, Verdict};
use super::prompt;
use super::shell::{self, Shell, ShellKind};
use super::tools::{self, Action, Prepared, Stamps, Todo, TodoStatus, ToolCx};
use crate::chat::{self, ChatError, ChatParams, ModelResolver, ResolvedModel, StartedRun};
use crate::llm::{
    ChatMessage, ChatRequest, FinishReason, LlmError, Role, ToolCall, ToolDef, toolparse,
};
use crate::new_id;
use crate::runs::{Run, RunEnd, RunEvent, RunManager, RunSink};
use crate::settings::SettingsStore;
use crate::store::{
    Message, MessageFinish, MessageRole, MessageStatus, Mode, NewMessage, Session, Store,
};

/// Model replies per task before the run pauses, so a confused model can't loop for ever.
pub const MAX_STEPS: usize = 50;
/// The same call with the same result this many times gets a warning; more stops the run.
const REPEAT_WARN: usize = 3;
const REPEAT_STOP: usize = 5;
/// How often the to-do list is repeated to the model (in steps).
const TODO_REMINDER_EVERY: usize = 10;
/// Command output kept for the UI (the model gets the first and last 150 lines).
const UI_OUTPUT_CHARS: usize = 50_000;

const UNDO_NOTE: &str = "\n\n(Note: the user undid file changes from an earlier task, so files may not match what \
you saw before. Read them again before editing.)";

const PYTHON_STUB: &str =
    "Python was not found; run without arguments to install from the Microsoft Store";

/// The user's answer to a permission prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    AllowOnce,
    AllowSession,
    /// Commands: remember the command's prefix as allowed in this project.
    AllowAlways,
    Deny,
}

impl Decision {
    fn as_str(self) -> &'static str {
        match self {
            Decision::AllowOnce => "allow_once",
            Decision::AllowSession => "allow_session",
            Decision::AllowAlways => "allow_always",
            Decision::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionReply {
    pub decision: Decision,
    /// With a denial: what to do instead.
    #[serde(default)]
    pub note: Option<String>,
}

/// What the agent remembers about a session while Hearth runs: which files the model has read
/// (and how they looked), what the user allowed for the session, its background commands, and
/// whether the user just undid some changes.
#[derive(Default)]
struct Memory {
    stamps: Stamps,
    grants: Mutex<Grants>,
    processes: Processes,
    undone: AtomicBool,
}

enum Waiter {
    Permission(oneshot::Sender<PermissionReply>),
    Answer(oneshot::Sender<String>),
}

struct Inner {
    store: Arc<Store>,
    runs: Arc<RunManager>,
    resolver: Arc<dyn ModelResolver>,
    settings: Arc<SettingsStore>,
    memory: Mutex<HashMap<String, Arc<Memory>>>,
    waits: Mutex<HashMap<String, Waiter>>,
    /// Messages sent while a run works, by run id.
    steers: Mutex<HashMap<String, Vec<String>>>,
    shells: Mutex<HashMap<ShellKind, Option<Shell>>>,
}

#[derive(Clone)]
pub struct Agent(Arc<Inner>);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// How a tool call ended, ready to be saved as a tool message.
struct Answered {
    status: &'static str,
    text: String,
    summary: String,
    display: Value,
    todos: Option<Vec<Todo>>,
    /// End the run after this call.
    stop: Option<RunEnd>,
}

impl Answered {
    fn new(
        status: &'static str,
        text: impl Into<String>,
        summary: impl Into<String>,
        display: Value,
    ) -> Self {
        Self {
            status,
            text: text.into(),
            summary: summary.into(),
            display,
            todos: None,
            stop: None,
        }
    }

    /// A call that never ran because the run stopped first.
    fn skipped(call: &ToolCall) -> Self {
        let display = json!({ "title": tool_title(call), "args": arguments_of(call) });
        Self::new(
            "cancelled",
            "Not run: the user stopped the run.",
            "Not run",
            display,
        )
    }
}

/// A short title for a call before (or without) preparing it.
fn tool_title(call: &ToolCall) -> String {
    tools::canonical_name(&call.function.name)
        .unwrap_or(call.function.name.as_str())
        .to_owned()
}

fn arguments_of(call: &ToolCall) -> Value {
    toolparse::parse_arguments(&call.function.arguments).unwrap_or(Value::Null)
}

/// Counts identical calls with identical results within a run.
#[derive(Default)]
struct Repeats(HashMap<u64, usize>);

impl Repeats {
    fn record(&mut self, name: &str, args: &Value, result: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        (name, args.to_string(), result).hash(&mut hasher);
        let count = self.0.entry(hasher.finish()).or_default();
        *count += 1;
        *count
    }
}

/// A 9-character id for a tool call (some chat templates insist on that length).
fn call_id() -> String {
    new_id().replace('-', "").chars().rev().take(9).collect()
}

/// Tool calls written as text (the server didn't parse them): taken out of the content, or
/// found in the reasoning.
fn calls_in_text(content: &str, reasoning: &str) -> Option<(String, Vec<ToolCall>)> {
    let known = |name: &str| tools::canonical_name(name).is_some();
    let to_calls = |found: Vec<toolparse::TextToolCall>| -> Vec<ToolCall> {
        found
            .into_iter()
            .map(|call| ToolCall::new(call_id(), call.name, call.arguments.to_string()))
            .collect()
    };
    let (found, cleaned) = toolparse::extract(content, &known);
    if !found.is_empty() {
        return Some((cleaned, to_calls(found)));
    }
    let (found, _) = toolparse::extract(reasoning, &known);
    (!found.is_empty()).then(|| (content.to_owned(), to_calls(found)))
}

/// The newest to-do list in a conversation.
fn latest_todos(history: &[Message]) -> Vec<Todo> {
    history
        .iter()
        .rev()
        .find_map(|m| m.tool_display.as_ref()?.get("todos").cloned())
        .and_then(|todos| serde_json::from_value(todos).ok())
        .unwrap_or_default()
}

/// Every assistant tool call followed by its answer, and no answers without a call: chat
/// templates break otherwise. Calls with no saved answer (Hearth stopped mid-run) get one.
fn pair_tool_results(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    let mut open: Vec<String> = Vec::new();
    let close = |out: &mut Vec<ChatMessage>, open: &mut Vec<String>| {
        for id in open.drain(..) {
            out.push(ChatMessage::tool(
                id,
                "(No result: the run stopped before this call ran.)",
            ));
        }
    };
    for message in messages {
        match message.role {
            Role::Tool => {
                let id = message.tool_call_id.clone().unwrap_or_default();
                if let Some(position) = open.iter().position(|open_id| *open_id == id) {
                    open.remove(position);
                    out.push(message);
                }
            }
            _ => {
                close(&mut out, &mut open);
                open = message
                    .tool_calls
                    .iter()
                    .map(|call| call.id.clone())
                    .collect();
                out.push(message);
            }
        }
    }
    close(&mut out, &mut open);
    out
}

/// What gets added to a request without being saved.
#[derive(Default)]
struct Extras {
    /// On the newest user message.
    note: String,
    /// On the newest tool result.
    reminder: Option<String>,
}

fn build_request(
    system: &str,
    history: &[Message],
    model: &ResolvedModel,
    tools: &[ToolDef],
    params: &ChatParams,
    extras: Extras,
) -> ChatRequest {
    let mut messages = vec![ChatMessage::system(system)];
    let last_user = history.iter().rposition(|m| m.role == MessageRole::User);
    for (index, message) in history.iter().enumerate() {
        match message.role {
            MessageRole::User => {
                let mut text = message.content.clone();
                if Some(index) == last_user {
                    text.push_str(&extras.note);
                }
                messages.push(ChatMessage::user(text));
            }
            MessageRole::Assistant => {
                let calls: Vec<ToolCall> = message
                    .tool_calls
                    .clone()
                    .and_then(|calls| serde_json::from_value(calls).ok())
                    .unwrap_or_default();
                if message.content.trim().is_empty() && calls.is_empty() {
                    continue;
                }
                let mut reply = ChatMessage::assistant(message.content.as_str());
                reply.tool_calls = calls;
                // Thinking from earlier steps of this task helps the model keep its thread;
                // templates that don't want it (or older turns) drop it themselves.
                if last_user.is_some_and(|user| index > user) {
                    reply.reasoning_content =
                        message.reasoning.clone().filter(|r| !r.trim().is_empty());
                }
                messages.push(reply);
            }
            MessageRole::Tool => {
                let id = message.tool_call_id.clone().unwrap_or_default();
                messages.push(ChatMessage::tool(id, message.content.as_str()));
            }
            MessageRole::Summary => messages.push(ChatMessage::user(format!(
                "(A summary of the earlier conversation)\n{}",
                message.content
            ))),
        }
    }
    let mut messages = pair_tool_results(messages);
    if let (Some(reminder), Some(last)) = (extras.reminder, messages.last_mut())
        && last.role == Role::Tool
        && let Some(crate::llm::Content::Text(text)) = &mut last.content
    {
        text.push_str(&reminder);
    }
    let mut request = ChatRequest {
        model: model.model.clone(),
        messages,
        tools: tools.to_vec(),
        ..Default::default()
    };
    chat::apply_params(&mut request, model, params);
    request
}

fn todo_reminder(step: usize, todos: &[Todo]) -> Option<String> {
    let open = todos
        .iter()
        .any(|todo| todo.status != TodoStatus::Completed);
    if step <= 1 || !(step - 1).is_multiple_of(TODO_REMINDER_EVERY) || !open {
        return None;
    }
    let list: Vec<String> = todos
        .iter()
        .map(|todo| {
            let mark = match todo.status {
                TodoStatus::Completed => "[x]",
                TodoStatus::InProgress => "[>]",
                TodoStatus::Pending => "[ ]",
            };
            format!("{mark} {}", todo.content)
        })
        .collect();
    Some(format!(
        "\n\n(Reminder, your to-do list:\n{})",
        list.join("\n")
    ))
}

fn first_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}\n… (cut here)", &text[..cut]),
        None => text.to_owned(),
    }
}

impl Agent {
    pub fn new(
        store: Arc<Store>,
        runs: Arc<RunManager>,
        resolver: Arc<dyn ModelResolver>,
        settings: Arc<SettingsStore>,
    ) -> Self {
        Self(Arc::new(Inner {
            store,
            runs,
            resolver,
            settings,
            memory: Mutex::new(HashMap::new()),
            waits: Mutex::new(HashMap::new()),
            steers: Mutex::new(HashMap::new()),
            shells: Mutex::new(HashMap::new()),
        }))
    }

    /// Starts working on a task. Returns once the user's message is saved.
    pub fn start(
        &self,
        session_id: &str,
        text: &str,
        sink: Arc<dyn RunSink>,
    ) -> Result<StartedRun, ChatError> {
        let inner = &self.0;
        let session = inner.store.session(session_id)?;
        if session.mode != Mode::Code {
            return Err(ChatError::Invalid(
                "This conversation isn't a coding session.".into(),
            ));
        }
        let (project, workspace) = self.workspace(&session).map_err(ChatError::Invalid)?;
        let model_ref =
            chat::model_for(&inner.settings.get(), &session).ok_or(ChatError::NoModel)?;
        let model = inner
            .resolver
            .resolve(&model_ref)
            .map_err(ChatError::Model)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(ChatError::Invalid("Describe the task first.".into()));
        }
        let turn = inner
            .store
            .active_path(&session.id)?
            .iter()
            .map(|m| m.turn)
            .max()
            .unwrap_or(0)
            + 1;

        let run = inner
            .runs
            .begin(&session.id, sink)
            .map_err(ChatError::Busy)?;
        let mut new = NewMessage::user(&session.id, session.active_leaf_id.as_deref(), text);
        new.turn = turn;
        let user = match inner.store.insert_message(new) {
            Ok(user) => user,
            Err(err) => {
                inner.runs.finish(&run.id);
                return Err(err.into());
            }
        };
        chat::keep_model(&inner.store, &session, &model_ref);

        let started = StartedRun {
            run_id: run.id.clone(),
            user_message_id: Some(user.id.clone()),
        };
        let this = self.clone();
        tokio::spawn(async move {
            let job = Job {
                run: &run,
                session: &session,
                workspace: Arc::new(workspace),
                project_id: project.id.clone(),
                trusted: project.trusted,
                turn,
                model: &model,
            };
            let end = this.work(&job, user).await;
            let unsent = lock(&this.0.steers).remove(&run.id).unwrap_or_default();
            if !unsent.is_empty() {
                run.emit(RunEvent::Notice {
                    level: "warning".into(),
                    text: format!(
                        "The task ended before this was sent, so send it again: {}",
                        unsent.join(" / ")
                    ),
                });
            }
            run.emit(RunEvent::Finished { status: end });
            this.0.runs.finish(&run.id);
        });
        Ok(started)
    }

    /// Adds a message to a task in progress; it joins the conversation before the next step.
    pub fn steer(&self, run_id: &str, text: &str) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Type a message first.".into());
        }
        let run = self
            .0
            .runs
            .get(run_id)
            .ok_or("The task already finished. Send your message as a new task.")?;
        let session = self
            .0
            .store
            .session(&run.session_id)
            .map_err(|e| e.to_string())?;
        if session.mode != Mode::Code {
            return Err("Only coding sessions take messages while they work.".into());
        }
        lock(&self.0.steers)
            .entry(run_id.to_owned())
            .or_default()
            .push(text.to_owned());
        Ok(())
    }

    /// Answers a permission prompt. False if nothing is waiting for it (any more).
    pub fn resolve_permission(&self, request_id: &str, reply: PermissionReply) -> bool {
        let mut waits = lock(&self.0.waits);
        match waits.remove(request_id) {
            Some(Waiter::Permission(tx)) => tx.send(reply).is_ok(),
            Some(other) => {
                waits.insert(request_id.to_owned(), other);
                false
            }
            None => false,
        }
    }

    /// Answers a question the agent asked. False if nothing is waiting for it (any more).
    pub fn answer(&self, question_id: &str, answer: &str) -> bool {
        let mut waits = lock(&self.0.waits);
        match waits.remove(question_id) {
            Some(Waiter::Answer(tx)) => tx.send(answer.trim().to_owned()).is_ok(),
            Some(other) => {
                waits.insert(question_id.to_owned(), other);
                false
            }
            None => false,
        }
    }

    /// The files the agent changed in a session, compared with how they were before.
    pub fn changes(&self, session_id: &str) -> Result<Vec<FileChange>, String> {
        let session = self
            .0
            .store
            .session(session_id)
            .map_err(|e| e.to_string())?;
        let (_, workspace) = self.workspace(&session)?;
        changes::changes(&self.0.store, session_id, &workspace)
    }

    /// Puts one file back as it was before the session changed it.
    pub fn revert(&self, session_id: &str, abs: &str) -> Result<(), String> {
        changes::revert(&self.0.store, session_id, abs)?;
        self.memory(session_id).undone.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Restores every file changed in task `turn` or later. Returns the files put back.
    pub fn undo(&self, session_id: &str, turn: i64) -> Result<Vec<String>, String> {
        if self.0.runs.for_session(session_id).is_some() {
            return Err("Stop the task first.".into());
        }
        let session = self
            .0
            .store
            .session(session_id)
            .map_err(|e| e.to_string())?;
        let (_, workspace) = self.workspace(&session)?;
        let restored = changes::undo_from(&self.0.store, session_id, turn, &workspace)?;
        if !restored.is_empty() {
            self.memory(session_id).undone.store(true, Ordering::SeqCst);
        }
        Ok(restored)
    }

    /// Forgets a session (it was deleted) and stops its background commands.
    pub fn forget(&self, session_id: &str) {
        if let Some(memory) = lock(&self.0.memory).remove(session_id) {
            memory.processes.kill_all();
        }
    }

    /// The shell commands run in: the one chosen in Settings, else the other one.
    pub fn shell(&self) -> Option<Shell> {
        let preferred = self.0.settings.get().code.shell;
        lock(&self.0.shells)
            .entry(preferred)
            .or_insert_with(|| Shell::choose(preferred))
            .clone()
    }

    fn workspace(&self, session: &Session) -> Result<(crate::store::Project, Workspace), String> {
        let project_id = session
            .workspace_id
            .clone()
            .ok_or("This session has no project folder.")?;
        let project = self
            .0
            .store
            .project(&project_id)
            .map_err(|e| e.to_string())?;
        let workspace = Workspace::new(Path::new(&project.path)).map_err(|_| {
            format!(
                "The project folder {} can't be opened. Was it moved or deleted?",
                project.path
            )
        })?;
        Ok((project, workspace))
    }

    fn memory(&self, session_id: &str) -> Arc<Memory> {
        lock(&self.0.memory)
            .entry(session_id.to_owned())
            .or_default()
            .clone()
    }

    fn mode(&self, session_id: &str) -> PermissionMode {
        self.0
            .store
            .session(session_id)
            .map(|s| PermissionMode::parse(s.permission_mode.as_deref()))
            .unwrap_or_default()
    }

    /// The session's frozen environment, taken now if this is its first task.
    fn snapshot(&self, session: &Session, workspace: &Workspace, trusted: bool) -> String {
        if let Some(snapshot) = &session.env_snapshot {
            return snapshot.clone();
        }
        let snapshot = prompt::snapshot(workspace, &chat::today(), trusted, self.shell().as_ref());
        if let Err(err) = self.0.store.set_env_snapshot(&session.id, &snapshot) {
            tracing::warn!("could not save the session's environment: {err}");
        }
        snapshot
    }

    async fn wait<T>(&self, run: &Run, id: &str, rx: oneshot::Receiver<T>) -> Option<T> {
        let answer = tokio::select! {
            answer = rx => answer.ok(),
            () = run.cancel.cancelled() => None,
        };
        lock(&self.0.waits).remove(id);
        answer
    }

    /// Adds messages the user sent while the run worked to the conversation.
    fn take_steers(
        &self,
        job: &Job<'_>,
        history: &mut Vec<Message>,
        step: usize,
    ) -> Result<(), String> {
        let texts = lock(&self.0.steers).remove(&job.run.id).unwrap_or_default();
        for text in texts {
            let parent = history.last().map(|m| m.id.clone());
            let mut new = NewMessage::user(&job.session.id, parent.as_deref(), &text);
            new.turn = job.turn;
            new.step = step as i64;
            let saved = self
                .0
                .store
                .insert_message(new)
                .map_err(|e| e.to_string())?;
            job.run.emit(RunEvent::Steered {
                message: Box::new(saved.clone()),
            });
            history.push(saved);
        }
        Ok(())
    }

    fn has_steers(&self, run_id: &str) -> bool {
        lock(&self.0.steers)
            .get(run_id)
            .is_some_and(|texts| !texts.is_empty())
    }

    async fn work(&self, job: &Job<'_>, user: Message) -> RunEnd {
        let store = &self.0.store;
        let run = job.run;
        run.emit(RunEvent::Started {
            session_id: job.session.id.clone(),
            user_message_id: Some(user.id.clone()),
            model: job.model.label.clone(),
        });
        chat::auto_title(store, run, &job.session.id);
        let system =
            prompt::system_prompt(&self.snapshot(job.session, &job.workspace, job.trusted));
        let defs = tools::tool_defs(&tools::specs());
        let params = ChatParams::of(job.session);
        let memory = self.memory(&job.session.id);
        let undone = memory.undone.swap(false, Ordering::SeqCst);
        let mut history = match store.path_to(&user.id) {
            Ok(history) => history,
            Err(err) => return chat::fail(run, "store", &err.to_string()),
        };
        let mut todos = latest_todos(&history);
        let mut repeats = Repeats::default();

        for step in 1..=MAX_STEPS {
            if run.cancel.is_cancelled() {
                return RunEnd::Cancelled;
            }
            if let Err(err) = self.take_steers(job, &mut history, step) {
                return chat::fail(run, "store", &err);
            }
            let leaf = history.last().map_or(user.id.clone(), |m| m.id.clone());
            let mut note = String::new();
            if undone {
                note.push_str(UNDO_NOTE);
            }
            if self.mode(&job.session.id) == PermissionMode::Plan {
                note.push_str(prompt::PLAN_REMINDER);
            }
            let extras = Extras {
                note,
                reminder: todo_reminder(step, &todos),
            };
            let request = build_request(&system, &history, job.model, &defs, &params, extras);
            let mut new = NewMessage::assistant(&job.session.id, Some(&leaf), &job.model.label);
            new.turn = job.turn;
            new.step = step as i64;
            let message = match store.insert_message(new) {
                Ok(message) => message,
                Err(err) => return chat::fail(run, "store", &err.to_string()),
            };
            run.emit(RunEvent::MessageStarted {
                message_id: message.id.clone(),
                parent_id: Some(leaf),
            });

            let streamed = chat::stream_reply(store, run, &message.id, job.model, &request).await;
            let mut content = streamed.content.clone();
            let mut calls = streamed.tool_calls.clone();
            if streamed.error.is_none()
                && calls.is_empty()
                && let Some((cleaned, found)) =
                    calls_in_text(&streamed.content, &streamed.reasoning)
            {
                content = cleaned;
                calls = found;
            }
            // Servers reuse ids like `call_0` from step to step, and some chat templates insist on
            // 9 characters: every call gets its own.
            for call in &mut calls {
                call.id = call_id();
            }
            let status = match &streamed.error {
                None => MessageStatus::Complete,
                Some(LlmError::Cancelled) => MessageStatus::Interrupted,
                Some(_) => MessageStatus::Error,
            };
            let error = streamed
                .error
                .as_ref()
                .filter(|e| **e != LlmError::Cancelled);
            let finish_reason = streamed.finish.as_ref().map(|f| f.as_str().to_owned());
            let finish = MessageFinish {
                content: content.clone(),
                reasoning: (!streamed.reasoning.is_empty()).then(|| streamed.reasoning.clone()),
                reasoning_ms: streamed.reasoning_ms,
                tool_calls: (!calls.is_empty()).then(|| json!(calls)),
                finish_reason: finish_reason.clone(),
                status,
                error: error.map(ToString::to_string),
                usage: streamed
                    .usage
                    .as_ref()
                    .and_then(|u| serde_json::to_value(u).ok()),
            };
            let saved = match store.finish_message(&message.id, &finish) {
                Ok(saved) => saved,
                Err(err) => return chat::fail(run, "store", &err.to_string()),
            };
            run.emit(RunEvent::MessageFinished {
                message_id: message.id.clone(),
                status: chat::status_name(status).into(),
                finish_reason,
                usage: streamed.usage.clone(),
                reasoning_ms: streamed.reasoning_ms,
                content: (content != streamed.content).then(|| content.clone()),
                tool_calls: calls.clone(),
            });
            history.push(saved);

            match &streamed.error {
                Some(LlmError::Cancelled) => return RunEnd::Cancelled,
                Some(err) => {
                    run.emit(RunEvent::Error {
                        code: err.code().into(),
                        message: err.to_string(),
                        retryable: err.retryable(),
                    });
                    return RunEnd::Failed;
                }
                None => {}
            }
            if calls.is_empty() {
                // The user said something while the model was finishing: carry on with it.
                if self.has_steers(&run.id) {
                    continue;
                }
                if streamed.finish == Some(FinishReason::Length) {
                    run.emit(RunEvent::Notice {
                        level: "warning".into(),
                        text: "The reply reached the model's length limit, so it may be cut off."
                            .into(),
                    });
                }
                return RunEnd::Completed;
            }

            for (index, call) in calls.iter().enumerate() {
                let answered = if run.cancel.is_cancelled() {
                    Answered {
                        stop: Some(RunEnd::Cancelled),
                        ..Answered::skipped(call)
                    }
                } else {
                    self.call_tool(job, &memory, call, &mut repeats).await
                };
                let saved = match self.save_answer(job, &history, step, call, &answered) {
                    Ok(saved) => saved,
                    Err(err) => return chat::fail(run, "store", &err),
                };
                run.emit(RunEvent::ToolFinished {
                    call_id: call.id.clone(),
                    status: answered.status.into(),
                    summary: answered.summary.clone(),
                    message: Box::new(saved.clone()),
                });
                history.push(saved);
                if let Some(items) = answered.todos {
                    todos.clone_from(&items);
                    run.emit(RunEvent::Todos { items });
                }
                if let Some(end) = answered.stop {
                    for rest in &calls[index + 1..] {
                        let skipped = Answered::skipped(rest);
                        match self.save_answer(job, &history, step, rest, &skipped) {
                            Ok(saved) => {
                                run.emit(RunEvent::ToolFinished {
                                    call_id: rest.id.clone(),
                                    status: skipped.status.into(),
                                    summary: skipped.summary.clone(),
                                    message: Box::new(saved.clone()),
                                });
                                history.push(saved);
                            }
                            Err(err) => return chat::fail(run, "store", &err),
                        }
                    }
                    return end;
                }
            }
        }
        run.emit(RunEvent::Notice {
            level: "info".into(),
            text: format!("Paused after {MAX_STEPS} steps. Send a message to let it continue."),
        });
        RunEnd::Paused
    }

    fn save_answer(
        &self,
        job: &Job<'_>,
        history: &[Message],
        step: usize,
        call: &ToolCall,
        answered: &Answered,
    ) -> Result<Message, String> {
        let parent = history.last().map(|m| m.id.as_str()).unwrap_or_default();
        let name =
            tools::canonical_name(&call.function.name).unwrap_or(call.function.name.as_str());
        let mut new = NewMessage::tool(&job.session.id, parent, &call.id, name, &answered.text);
        new.turn = job.turn;
        new.step = step as i64;
        new.tool_status = Some(answered.status.into());
        let mut display = answered.display.clone();
        display["summary"] = json!(answered.summary);
        new.tool_display = Some(display);
        self.0.store.insert_message(new).map_err(|e| e.to_string())
    }

    async fn call_tool(
        &self,
        job: &Job<'_>,
        memory: &Arc<Memory>,
        call: &ToolCall,
        repeats: &mut Repeats,
    ) -> Answered {
        let run = job.run;
        let name = call.function.name.clone();
        let started = |title: &str, tool: &str| {
            run.emit(RunEvent::ToolStarted {
                call_id: call.id.clone(),
                tool: tool.to_owned(),
                title: title.to_owned(),
            });
        };
        let args = match toolparse::parse_arguments(&call.function.arguments) {
            Ok(args) => args,
            Err(err) => {
                let title = tool_title(call);
                started(&title, &title);
                let text = format!(
                    "The arguments for {name} couldn't be read ({err}). Send them as one JSON object, \
                     for example {{\"path\": \"src/main.rs\"}}."
                );
                return Answered::new(
                    "error",
                    text,
                    "Unreadable arguments",
                    json!({ "title": title }),
                );
            }
        };

        let prepared = {
            let (workspace, memory, args, name) = (
                job.workspace.clone(),
                memory.clone(),
                args.clone(),
                name.clone(),
            );
            tokio::task::spawn_blocking(move || {
                tools::prepare(
                    &name,
                    args,
                    &ToolCx {
                        workspace: &workspace,
                        stamps: &memory.stamps,
                    },
                )
            })
            .await
            .unwrap_or_else(|err| Err(format!("The tool failed unexpectedly: {err}")))
        };
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(text) => {
                let title = tool_title(call);
                started(&title, &title);
                let summary = text.lines().next().unwrap_or_default().to_owned();
                return Answered::new(
                    "error",
                    text,
                    summary,
                    json!({ "title": title, "args": args }),
                );
            }
        };
        started(&prepared.title, prepared.tool);
        let mut display = json!({ "title": prepared.title, "args": args });
        if let Some(path) = &prepared.path {
            display["path"] = json!(path);
        }
        if let Some(diff) = &prepared.diff {
            display["diff"] = json!(diff);
        }

        if let Action::Ask { question, options } = &prepared.action {
            display["question"] = json!(question);
            display["options"] = json!(options);
            let question_id = new_id();
            let (tx, rx) = oneshot::channel();
            lock(&self.0.waits).insert(question_id.clone(), Waiter::Answer(tx));
            run.emit(RunEvent::Question {
                question_id: question_id.clone(),
                call_id: call.id.clone(),
                question: question.clone(),
                options: options.clone(),
            });
            let Some(answer) = self.wait(run, &question_id, rx).await else {
                return Answered {
                    stop: Some(RunEnd::Cancelled),
                    ..Answered::new(
                        "cancelled",
                        "The user stopped the run instead of answering.",
                        "Not answered",
                        display,
                    )
                };
            };
            run.emit(RunEvent::QuestionAnswered {
                question_id,
                answer: answer.clone(),
            });
            display["answer"] = json!(answer);
            let text = if answer.is_empty() {
                "The user didn't answer.".to_owned()
            } else {
                format!("The user answered: {answer}")
            };
            return Answered::new("ok", text, format!("Answered: {answer}"), display);
        }

        if let Some(denied) = self
            .permit(job, memory, call, &prepared, &mut display)
            .await
        {
            return denied;
        }

        let todos = match &prepared.action {
            Action::Todo(items) => Some(items.clone()),
            _ => None,
        };
        let outcome = match prepared.action {
            Action::Run {
                command,
                cwd,
                timeout_s,
                background,
            } => {
                return self
                    .run_command(
                        job, memory, call, &command, &cwd, timeout_s, background, display,
                    )
                    .await;
            }
            Action::Output {
                process_id,
                wait_s,
                tail_lines,
            } => match memory.processes.get(process_id) {
                Some(process) => {
                    let report = process
                        .report(Duration::from_secs(wait_s), tail_lines)
                        .await;
                    let summary = if process.running() {
                        "running"
                    } else {
                        "finished"
                    };
                    tools::Outcome {
                        ok: true,
                        text: report,
                        summary: summary.into(),
                    }
                }
                None => tools::Outcome {
                    ok: false,
                    text: format!("There's no background process {process_id} in this session."),
                    summary: "No such process".into(),
                },
            },
            Action::Kill { process_id } => match memory.processes.get(process_id) {
                Some(process) => {
                    process.kill();
                    tools::Outcome {
                        ok: true,
                        text: format!("Stopped process {process_id}."),
                        summary: "stopped".into(),
                    }
                }
                None => tools::Outcome {
                    ok: false,
                    text: format!("There's no background process {process_id} in this session."),
                    summary: "No such process".into(),
                },
            },
            action => {
                let prepared = Prepared { action, ..prepared };
                if let Action::Write { abs, .. } | Action::Edit { abs, .. } = &prepared.action {
                    // How the file was before its first change in this task, for undo.
                    let before = std::fs::read(abs).ok();
                    let key = abs.display().to_string();
                    let saved = self.0.store.save_checkpoint(
                        &job.session.id,
                        job.turn,
                        &key,
                        before.is_some(),
                        before.as_deref().unwrap_or_default(),
                    );
                    if let Err(err) = saved {
                        tracing::warn!("could not save a checkpoint of {key}: {err}");
                    }
                }
                let (workspace, memory) = (job.workspace.clone(), memory.clone());
                tokio::task::spawn_blocking(move || {
                    tools::execute(
                        prepared,
                        &ToolCx {
                            workspace: &workspace,
                            stamps: &memory.stamps,
                        },
                    )
                })
                .await
                .unwrap_or_else(|err| tools::Outcome {
                    ok: false,
                    text: format!("The tool failed unexpectedly: {err}"),
                    summary: "Failed".into(),
                })
            }
        };
        self.finish_outcome(run, &name, &args, outcome, display, todos, repeats)
    }

    /// Checks the call against the mode, the session's grants and the project's rules, asking
    /// the user when needed. `None` means go ahead; otherwise the answer to give the model.
    async fn permit(
        &self,
        job: &Job<'_>,
        memory: &Arc<Memory>,
        call: &ToolCall,
        prepared: &Prepared,
        display: &mut Value,
    ) -> Option<Answered> {
        let run = job.run;
        let rules = self
            .0
            .store
            .rules(&job.project_id, "execute")
            .unwrap_or_default();
        let verdict = permissions::decide(
            self.mode(&job.session.id),
            prepared,
            &lock(&memory.grants),
            &rules,
        );
        match verdict {
            Verdict::Allow => None,
            Verdict::Deny(reason) => Some(Answered::new(
                "denied",
                reason,
                "Not allowed in Plan mode",
                display.clone(),
            )),
            Verdict::Ask => {
                let request_id = new_id();
                let (tx, rx) = oneshot::channel();
                lock(&self.0.waits).insert(request_id.clone(), Waiter::Permission(tx));
                run.emit(RunEvent::PermissionRequest {
                    request_id: request_id.clone(),
                    call_id: call.id.clone(),
                    tool: prepared.tool.into(),
                    class: prepared.class,
                    title: prepared.title.clone(),
                    path: prepared.path.clone(),
                    diff: prepared.diff.clone(),
                    command: match &prepared.action {
                        Action::Run { command, .. } => Some(command.clone()),
                        _ => None,
                    },
                    danger: prepared
                        .command
                        .as_ref()
                        .and_then(|c| c.danger.map(str::to_owned)),
                    choices: permissions::choices(prepared),
                });
                let Some(reply) = self.wait(run, &request_id, rx).await else {
                    return Some(Answered {
                        stop: Some(RunEnd::Cancelled),
                        ..Answered::new(
                            "cancelled",
                            "The user stopped the run instead of answering.",
                            "Not run",
                            display.clone(),
                        )
                    });
                };
                run.emit(RunEvent::PermissionResolved {
                    request_id,
                    decision: reply.decision.as_str().into(),
                });
                match reply.decision {
                    Decision::Deny => {
                        let note = reply
                            .note
                            .map(|n| n.trim().to_owned())
                            .filter(|n| !n.is_empty());
                        let text = match &note {
                            Some(note) => format!(
                                "The user declined this {} call and said: {note}",
                                prepared.tool
                            ),
                            None => format!(
                                "The user declined this {} call. Don't try it again; take another approach, \
                                 or ask what they'd like.",
                                prepared.tool
                            ),
                        };
                        if let Some(note) = &note {
                            display["note"] = json!(note);
                        }
                        Some(Answered::new("denied", text, "Declined", display.clone()))
                    }
                    Decision::AllowAlways => {
                        if let Some(check) =
                            prepared.command.as_ref().filter(|c| !c.prefix.is_empty())
                        {
                            if let Err(err) =
                                self.0
                                    .store
                                    .add_rule(&job.project_id, "execute", &check.prefix)
                            {
                                tracing::warn!("could not save the rule: {err}");
                            }
                        } else {
                            lock(&memory.grants).grant(prepared);
                        }
                        None
                    }
                    Decision::AllowSession => {
                        lock(&memory.grants).grant(prepared);
                        None
                    }
                    Decision::AllowOnce => None,
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_command(
        &self,
        job: &Job<'_>,
        memory: &Arc<Memory>,
        call: &ToolCall,
        command: &str,
        cwd: &Path,
        timeout_s: u64,
        background: bool,
        mut display: Value,
    ) -> Answered {
        let run = job.run;
        display["command"] = json!(command);
        let Some(shell) = self.shell() else {
            return Answered::new(
                "error",
                "No shell was found on this PC, so commands can't run. The user can install Git for Windows \
                 (for Git Bash) or choose PowerShell in Settings.",
                "No shell",
                display,
            );
        };
        if background {
            return match memory.processes.start(&shell, command, cwd) {
                Ok(process) => {
                    let id = process.id;
                    display["processId"] = json!(id);
                    Answered::new(
                        "ok",
                        format!(
                            "Started process {id} in the background. Check its output with command_output \
                             (process_id {id}) and stop it with kill_process."
                        ),
                        format!("process {id}, in the background"),
                        display,
                    )
                }
                Err(err) => Answered::new(
                    "error",
                    format!("Couldn't start the command: {err}"),
                    "Couldn't start",
                    display,
                ),
            };
        }

        let call_id = call.id.clone();
        let ran = exec::run(
            &shell,
            command,
            cwd,
            Duration::from_secs(timeout_s),
            &run.cancel,
            |text| {
                if !text.is_empty() {
                    run.emit(RunEvent::ToolOutput {
                        call_id: call_id.clone(),
                        text,
                    });
                }
            },
        )
        .await;
        let ran = match ran {
            Ok(ran) => ran,
            Err(err) => {
                return Answered::new(
                    "error",
                    format!("Couldn't start the command: {err}"),
                    "Couldn't start",
                    display,
                );
            }
        };
        display["output"] = json!(first_chars(&ran.output, UI_OUTPUT_CHARS));
        display["exitCode"] = json!(ran.exit);
        display["durationMs"] = json!(ran.elapsed.as_millis() as u64);
        let shown = if ran.output.is_empty() {
            "(no output)".to_owned()
        } else {
            shell::head_and_tail(&ran.output, 150, 150)
        };
        let seconds = format!("{:.1}s", ran.elapsed.as_secs_f64());
        if ran.cancelled {
            return Answered {
                stop: Some(RunEnd::Cancelled),
                ..Answered::new(
                    "cancelled",
                    format!("The user stopped the command.\n\nOutput so far:\n{shown}"),
                    "stopped",
                    display,
                )
            };
        }
        if ran.timed_out {
            let text = format!(
                "The command was stopped after {timeout_s}s because it hadn't finished. If it's a server or a \
                 watcher, run it with background: true and check it with command_output.\n\nOutput so far:\n{shown}"
            );
            return Answered::new(
                "error",
                text,
                format!("timed out after {timeout_s}s"),
                display,
            );
        }
        let code = ran
            .exit
            .map_or_else(|| "unknown".to_owned(), |code| code.to_string());
        let mut text = format!("{shown}\n\n(exit code {code}, {seconds})");
        if ran.stopped_leftovers {
            text.push_str(
                "\n(It left processes running that held its output open, so they were stopped. Run servers \
                 with background: true.)",
            );
        }
        if ran.output.contains(PYTHON_STUB) {
            text.push_str(
                "\n(That message comes from Windows' placeholder for python, not a real Python. Try `py` \
                 instead, or tell the user Python isn't installed.)",
            );
        }
        let status = if ran.exit == Some(0) { "ok" } else { "error" };
        Answered::new(status, text, format!("exit {code} · {seconds}"), display)
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_outcome(
        &self,
        run: &Run,
        name: &str,
        args: &Value,
        outcome: tools::Outcome,
        mut display: Value,
        todos: Option<Vec<Todo>>,
        repeats: &mut Repeats,
    ) -> Answered {
        let mut text = outcome.text;
        let mut stop = None;
        let count = repeats.record(name, args, &text);
        if count >= REPEAT_STOP {
            stop = Some(RunEnd::Paused);
            run.emit(RunEvent::Notice {
                level: "warning".into(),
                text: format!(
                    "The agent made the same {name} call {count} times with the same result, so Hearth paused it. \
                     Send a message to guide it."
                ),
            });
        } else if count >= REPEAT_WARN {
            text.push_str(&format!(
                "\n\n(You've now made this exact call {count} times with the same result. Try something different.)"
            ));
        }
        if let Some(items) = &todos {
            display["todos"] = json!(items);
        }
        Answered {
            todos,
            stop,
            ..Answered::new(
                if outcome.ok { "ok" } else { "error" },
                text,
                outcome.summary,
                display,
            )
        }
    }
}

/// One run's fixed context.
struct Job<'a> {
    run: &'a Run,
    session: &'a Session,
    workspace: Arc<Workspace>,
    project_id: String,
    trusted: bool,
    /// The task's number in the session (checkpoints and messages are filed under it).
    turn: i64,
    model: &'a ResolvedModel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_call_gets_an_answer_and_strays_are_dropped() {
        let mut assistant = ChatMessage::assistant("");
        assistant.tool_calls = vec![
            ToolCall::new("a", "read_file", "{}"),
            ToolCall::new("b", "glob", "{}"),
        ];
        let messages = vec![
            ChatMessage::user("go"),
            assistant,
            ChatMessage::tool("b", "found"),
            ChatMessage::tool("zzz", "stray"),
            ChatMessage::user("again"),
        ];
        let paired = pair_tool_results(messages);
        let shape: Vec<(Role, Option<String>)> = paired
            .iter()
            .map(|m| (m.role, m.tool_call_id.clone()))
            .collect();
        assert_eq!(
            shape,
            [
                (Role::User, None),
                (Role::Assistant, None),
                (Role::Tool, Some("b".into())),
                (Role::Tool, Some("a".into())),
                (Role::User, None),
            ]
        );
    }

    #[test]
    fn calls_written_as_text_are_found_and_taken_out() {
        let (content, calls) =
            calls_in_text("Let me look.\n<tool_call>{\"name\": \"Read\", \"arguments\": {\"path\": \"a.py\"}}</tool_call>", "")
                .unwrap();
        assert_eq!(content.trim(), "Let me look.");
        assert_eq!(
            (calls[0].function.name.as_str(), calls[0].id.len()),
            ("Read", 9)
        );
        assert!(calls_in_text("No tools here.", "").is_none());
        let (content, calls) = calls_in_text(
            "",
            "<tool_call>{\"name\":\"glob\",\"arguments\":{\"pattern\":\"*\"}}</tool_call>",
        )
        .unwrap();
        assert_eq!((content.as_str(), calls.len()), ("", 1));
    }

    #[test]
    fn the_todo_reminder_comes_every_ten_steps_while_items_are_open() {
        let todos = vec![
            Todo {
                content: "Fix".into(),
                status: TodoStatus::Completed,
            },
            Todo {
                content: "Test".into(),
                status: TodoStatus::Pending,
            },
        ];
        assert!(todo_reminder(1, &todos).is_none());
        assert!(todo_reminder(10, &todos).is_none());
        assert_eq!(
            todo_reminder(11, &todos).unwrap(),
            "\n\n(Reminder, your to-do list:\n[x] Fix\n[ ] Test)"
        );
        let done = vec![Todo {
            content: "Fix".into(),
            status: TodoStatus::Completed,
        }];
        assert!(todo_reminder(11, &done).is_none());
    }

    #[test]
    fn long_command_output_is_cut_for_the_ui() {
        assert_eq!(first_chars("abc", 5), "abc");
        assert_eq!(first_chars("abcdef", 3), "abc\n… (cut here)");
    }
}
