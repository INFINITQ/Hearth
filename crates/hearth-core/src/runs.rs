//! Runs: a chat reply being written, or the agent working through a task.
//!
//! Rust owns each run: it keeps going if the UI reloads or the window is hidden. The UI attaches
//! a sink (a Tauri channel) to watch it, and gets a snapshot of where the run is whenever it
//! (re)attaches, so nothing is lost and nothing needs replaying. A tool that is running and a
//! question waiting for the user are sent again after the snapshot.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::agent::permissions::Choice;
use crate::agent::tools::{Class, Todo};
use crate::llm::{ToolCall, Usage};
use crate::new_id;
use crate::store::Message;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Starting,
    Streaming,
    /// Waiting for the user: a permission prompt or a question.
    Waiting,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEnd {
    Completed,
    Cancelled,
    Failed,
    /// Stopped by a guard (the step limit, repeated calls). A new message carries on.
    Paused,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RunEvent {
    /// Where the run is now. Sent first to a sink that (re)attaches. `message_id` is the reply
    /// streaming at that moment, if any.
    Snapshot {
        session_id: String,
        state: RunState,
        user_message_id: Option<String>,
        message_id: Option<String>,
        content: String,
        reasoning: String,
    },
    Started {
        session_id: String,
        user_message_id: Option<String>,
        model: String,
    },
    /// llama.cpp reading the prompt before it starts answering.
    PromptProgress {
        processed: u64,
        total: u64,
        cached: u64,
    },
    MessageStarted {
        message_id: String,
        parent_id: Option<String>,
    },
    ReasoningDelta {
        message_id: String,
        text: String,
    },
    ContentDelta {
        message_id: String,
        text: String,
    },
    MessageFinished {
        message_id: String,
        status: String,
        finish_reason: Option<String>,
        usage: Option<Usage>,
        reasoning_ms: Option<i64>,
        /// The final text, when it differs from what streamed (tool calls written as text are
        /// taken out).
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    /// A tool call's arguments arriving, e.g. a long file being written.
    ToolCallProgress {
        message_id: String,
        index: usize,
        name: Option<String>,
        chars: usize,
    },
    ToolStarted {
        call_id: String,
        tool: String,
        title: String,
    },
    /// Output from a running command, as it arrives.
    ToolOutput {
        call_id: String,
        text: String,
    },
    /// A tool call answered; `message` is the saved tool message.
    ToolFinished {
        call_id: String,
        status: String,
        summary: String,
        message: Box<Message>,
    },
    PermissionRequest {
        request_id: String,
        call_id: String,
        tool: String,
        class: Class,
        title: String,
        path: Option<String>,
        diff: Option<String>,
        /// For commands: the command, and why it's dangerous if it is.
        command: Option<String>,
        danger: Option<String>,
        choices: Vec<Choice>,
    },
    PermissionResolved {
        request_id: String,
        decision: String,
    },
    Question {
        question_id: String,
        call_id: String,
        question: String,
        options: Vec<String>,
    },
    QuestionAnswered {
        question_id: String,
        answer: String,
    },
    Todos {
        items: Vec<Todo>,
    },
    /// A message the user sent while the agent worked, now part of the conversation.
    Steered {
        message: Box<Message>,
    },
    /// Something the user should know that isn't an error (a pause, a cut-off reply).
    Notice {
        level: String,
        text: String,
    },
    TitleChanged {
        session_id: String,
        title: String,
    },
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
    Finished {
        status: RunEnd,
    },
}

/// An event as delivered: which run, and its position in that run's sequence.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEnvelope {
    pub run_id: String,
    pub seq: u64,
    #[serde(flatten)]
    pub event: RunEvent,
}

/// Where a run's events go (a Tauri channel in the app; a list in tests).
pub trait RunSink: Send + Sync {
    fn emit(&self, envelope: &RunEnvelope);
}

/// Keeps every event, for tests.
#[derive(Default)]
pub struct CollectSink {
    events: Mutex<Vec<RunEnvelope>>,
}

impl CollectSink {
    pub fn events(&self) -> Vec<RunEvent> {
        lock(&self.events).iter().map(|e| e.event.clone()).collect()
    }
}

impl RunSink for CollectSink {
    fn emit(&self, envelope: &RunEnvelope) {
        lock(&self.events).push(envelope.clone());
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone)]
struct Progress {
    state: RunState,
    user_message_id: Option<String>,
    message_id: Option<String>,
    content: String,
    reasoning: String,
    /// The tool running now (its `ToolStarted`).
    tool: Option<RunEvent>,
    /// The permission request or question waiting for the user.
    pending: Option<RunEvent>,
}

type Listener = Arc<dyn Fn(&RunInfo) + Send + Sync>;

pub struct Run {
    pub id: String,
    pub session_id: String,
    pub cancel: CancellationToken,
    seq: AtomicU64,
    sink: Mutex<Option<Arc<dyn RunSink>>>,
    progress: Mutex<Progress>,
    listener: Option<Listener>,
}

impl Run {
    pub fn emit(&self, event: RunEvent) {
        let changed = {
            let mut progress = lock(&self.progress);
            let before = progress.state;
            match &event {
                RunEvent::Started {
                    user_message_id, ..
                } => progress.user_message_id.clone_from(user_message_id),
                RunEvent::MessageStarted { message_id, .. } => {
                    progress.message_id = Some(message_id.clone());
                    progress.content.clear();
                    progress.reasoning.clear();
                    progress.state = RunState::Streaming;
                }
                RunEvent::ReasoningDelta { text, .. } => progress.reasoning.push_str(text),
                RunEvent::ContentDelta { text, .. } => progress.content.push_str(text),
                RunEvent::MessageFinished { .. } => {
                    progress.message_id = None;
                    progress.content.clear();
                    progress.reasoning.clear();
                }
                RunEvent::ToolStarted { .. } => progress.tool = Some(event.clone()),
                RunEvent::ToolFinished { .. } => progress.tool = None,
                RunEvent::PermissionRequest { .. } | RunEvent::Question { .. } => {
                    progress.pending = Some(event.clone());
                    progress.state = RunState::Waiting;
                }
                RunEvent::PermissionResolved { .. } | RunEvent::QuestionAnswered { .. } => {
                    progress.pending = None;
                    progress.state = RunState::Streaming;
                }
                RunEvent::Finished { .. } => progress.state = RunState::Finished,
                _ => {}
            }
            (progress.state != before
                && matches!(progress.state, RunState::Waiting)
                    != matches!(before, RunState::Waiting))
            .then_some(progress.state)
        };
        self.send(event);
        if let (Some(state), Some(listener)) = (changed, &self.listener) {
            listener(&RunInfo {
                run_id: self.id.clone(),
                session_id: self.session_id.clone(),
                state,
            });
        }
    }

    fn send(&self, event: RunEvent) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let sink = lock(&self.sink).clone();
        if let Some(sink) = sink {
            sink.emit(&RunEnvelope {
                run_id: self.id.clone(),
                seq,
                event,
            });
        }
    }

    fn attach(&self, sink: Arc<dyn RunSink>) {
        *lock(&self.sink) = Some(sink);
        let progress = lock(&self.progress).clone();
        self.send(RunEvent::Snapshot {
            session_id: self.session_id.clone(),
            state: progress.state,
            user_message_id: progress.user_message_id,
            message_id: progress.message_id,
            content: progress.content,
            reasoning: progress.reasoning,
        });
        if let Some(tool) = progress.tool {
            self.send(tool);
        }
        if let Some(pending) = progress.pending {
            self.send(pending);
        }
    }

    pub fn state(&self) -> RunState {
        lock(&self.progress).state
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub run_id: String,
    pub session_id: String,
    pub state: RunState,
}

/// All runs in progress. At most one per session.
#[derive(Default)]
pub struct RunManager {
    runs: Mutex<HashMap<String, Arc<Run>>>,
    listener: Mutex<Option<Listener>>,
}

impl RunManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called whenever a run starts, ends, or starts or stops waiting for the user (the app
    /// turns this into sidebar spinners and "needs you" dots).
    pub fn on_change(&self, listener: impl Fn(&RunInfo) + Send + Sync + 'static) {
        *lock(&self.listener) = Some(Arc::new(listener));
    }

    fn notify(&self, info: &RunInfo) {
        let listener = lock(&self.listener).clone();
        if let Some(listener) = listener {
            listener(info);
        }
    }

    /// Starts a run for `session_id`, sending its events to `sink`.
    pub fn begin(&self, session_id: &str, sink: Arc<dyn RunSink>) -> Result<Arc<Run>, String> {
        let listener = lock(&self.listener).clone();
        let run = {
            let mut runs = lock(&self.runs);
            if runs.values().any(|run| run.session_id == session_id) {
                return Err("A reply is still being written in this conversation.".into());
            }
            let run = Arc::new(Run {
                id: new_id(),
                session_id: session_id.to_owned(),
                cancel: CancellationToken::new(),
                seq: AtomicU64::new(0),
                sink: Mutex::new(Some(sink)),
                progress: Mutex::new(Progress {
                    state: RunState::Starting,
                    user_message_id: None,
                    message_id: None,
                    content: String::new(),
                    reasoning: String::new(),
                    tool: None,
                    pending: None,
                }),
                listener,
            });
            runs.insert(run.id.clone(), run.clone());
            run
        };
        self.notify(&RunInfo {
            run_id: run.id.clone(),
            session_id: run.session_id.clone(),
            state: RunState::Starting,
        });
        Ok(run)
    }

    /// Points a run's events at a new sink, starting with a snapshot. False if it has ended.
    pub fn attach(&self, run_id: &str, sink: Arc<dyn RunSink>) -> bool {
        let run = lock(&self.runs).get(run_id).cloned();
        match run {
            Some(run) => {
                run.attach(sink);
                true
            }
            None => false,
        }
    }

    pub fn cancel(&self, run_id: &str) -> bool {
        let run = lock(&self.runs).get(run_id).cloned();
        run.map(|run| run.cancel.cancel()).is_some()
    }

    pub fn cancel_session(&self, session_id: &str) {
        for run in lock(&self.runs)
            .values()
            .filter(|run| run.session_id == session_id)
        {
            run.cancel.cancel();
        }
    }

    /// The run in progress for a session, if any.
    /// A run in progress.
    pub fn get(&self, run_id: &str) -> Option<Arc<Run>> {
        lock(&self.runs).get(run_id).cloned()
    }

    pub fn for_session(&self, session_id: &str) -> Option<RunInfo> {
        lock(&self.runs)
            .values()
            .find(|run| run.session_id == session_id)
            .map(|run| RunInfo {
                run_id: run.id.clone(),
                session_id: run.session_id.clone(),
                state: run.state(),
            })
    }

    pub fn active(&self) -> Vec<RunInfo> {
        lock(&self.runs)
            .values()
            .map(|run| RunInfo {
                run_id: run.id.clone(),
                session_id: run.session_id.clone(),
                state: run.state(),
            })
            .collect()
    }

    /// Removes a run that has ended.
    pub fn finish(&self, run_id: &str) {
        let removed = lock(&self.runs).remove(run_id);
        if let Some(run) = removed {
            self.notify(&RunInfo {
                run_id: run.id.clone(),
                session_id: run.session_id.clone(),
                state: RunState::Finished,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attaching_sends_a_snapshot_then_live_events() {
        let runs = RunManager::new();
        let first = Arc::new(CollectSink::default());
        let run = runs.begin("s1", first.clone()).unwrap();
        run.emit(RunEvent::MessageStarted {
            message_id: "m".into(),
            parent_id: None,
        });
        run.emit(RunEvent::ContentDelta {
            message_id: "m".into(),
            text: "Hel".into(),
        });

        let second = Arc::new(CollectSink::default());
        assert!(runs.attach(&run.id, second.clone()));
        run.emit(RunEvent::ContentDelta {
            message_id: "m".into(),
            text: "lo".into(),
        });

        let events = second.events();
        assert!(
            matches!(&events[0], RunEvent::Snapshot { content, state: RunState::Streaming, .. } if content == "Hel")
        );
        assert_eq!(
            events[1],
            RunEvent::ContentDelta {
                message_id: "m".into(),
                text: "lo".into()
            }
        );
        assert_eq!(first.events().len(), 2, "the old sink stops receiving");
    }

    #[test]
    fn one_run_per_session_and_cancel_reaches_the_token() {
        let runs = RunManager::new();
        let sink = Arc::new(CollectSink::default());
        let run = runs.begin("s1", sink.clone()).unwrap();
        assert!(runs.begin("s1", sink.clone()).is_err());
        assert!(runs.begin("s2", sink.clone()).is_ok());
        assert!(runs.cancel(&run.id));
        assert!(run.cancel.is_cancelled());
        runs.finish(&run.id);
        assert!(!runs.attach(&run.id, sink));
        assert!(runs.for_session("s1").is_none());
    }

    #[test]
    fn a_waiting_question_is_sent_again_on_attach_and_reported_as_waiting() {
        let runs = RunManager::new();
        let states = Arc::new(Mutex::new(Vec::new()));
        let seen = states.clone();
        runs.on_change(move |info| lock(&seen).push(info.state));
        let run = runs.begin("s1", Arc::new(CollectSink::default())).unwrap();
        let question = RunEvent::Question {
            question_id: "q".into(),
            call_id: "c".into(),
            question: "Which one?".into(),
            options: vec!["A".into(), "B".into()],
        };
        run.emit(question.clone());
        assert_eq!(run.state(), RunState::Waiting);

        let late = Arc::new(CollectSink::default());
        runs.attach(&run.id, late.clone());
        let events = late.events();
        assert!(matches!(
            events[0],
            RunEvent::Snapshot {
                state: RunState::Waiting,
                ..
            }
        ));
        assert_eq!(events[1], question);

        run.emit(RunEvent::QuestionAnswered {
            question_id: "q".into(),
            answer: "B".into(),
        });
        assert_eq!(
            *lock(&states),
            [RunState::Starting, RunState::Waiting, RunState::Streaming]
        );
    }
}
