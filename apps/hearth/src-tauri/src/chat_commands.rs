//! Commands for conversations, search, export, connections and runs.

use std::sync::Arc;

use hearth_core::agent::Agent;
use hearth_core::chat::{Chat, ChatAction, StartedRun};
use hearth_core::llm::LlmClient;
use hearth_core::runs::{RunEnvelope, RunInfo, RunManager, RunSink};
use hearth_core::store::{
    Connection, ConnectionInput, Message, Mode, SearchHit, Session, SessionPatch, SessionSummary,
    Store,
};
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State, Wry};

use crate::{Error, Result, window};

/// Sidebar lists changed (a new, renamed, pinned or deleted conversation).
pub const SESSIONS_EVENT: &str = "hearth://sessions";
/// A run started or ended somewhere (for spinners in the sidebar).
pub const RUN_STATE_EVENT: &str = "hearth://run-state";

impl From<hearth_core::store::StoreError> for Error {
    fn from(err: hearth_core::store::StoreError) -> Self {
        match err {
            hearth_core::store::StoreError::NotFound(what) => {
                Error::Invalid(format!("The {what} doesn't exist any more."))
            }
            other => Error::Invalid(other.to_string()),
        }
    }
}

impl From<hearth_core::chat::ChatError> for Error {
    fn from(err: hearth_core::chat::ChatError) -> Self {
        Error::Invalid(err.to_string())
    }
}

fn parse_mode(mode: &str) -> Result<Mode> {
    Mode::parse(mode).ok_or_else(|| Error::Invalid(format!("unknown mode {mode}")))
}

pub(crate) fn sessions_changed(app: &AppHandle<Wry>) {
    let _ = app.emit_to(window::MAIN, SESSIONS_EVENT, ());
}

/// Which version a message is among its siblings.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Versions {
    index: usize,
    ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    #[serde(flatten)]
    message: Message,
    versions: Versions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    session: Session,
    messages: Vec<MessageView>,
    /// A reply still being written, to attach to.
    run: Option<RunInfo>,
}

fn session_view(store: &Store, runs: &RunManager, id: &str) -> Result<SessionView> {
    let session = store.session(id)?;
    let messages = store
        .active_path(id)?
        .into_iter()
        .map(|message| {
            let ids = store
                .siblings(&message.id)
                .unwrap_or_else(|_| vec![message.id.clone()]);
            let index = ids
                .iter()
                .position(|sibling| *sibling == message.id)
                .unwrap_or(0);
            MessageView {
                message,
                versions: Versions { index, ids },
            }
        })
        .collect();
    Ok(SessionView {
        session,
        messages,
        run: runs.for_session(id),
    })
}

#[tauri::command]
pub async fn sessions_list(
    store: State<'_, Arc<Store>>,
    mode: String,
    limit: Option<usize>,
) -> Result<Vec<SessionSummary>> {
    Ok(store.sessions(parse_mode(&mode)?, limit.unwrap_or(200), 0)?)
}

#[tauri::command]
pub async fn session_create(
    app: AppHandle<Wry>,
    store: State<'_, Arc<Store>>,
    mode: String,
) -> Result<Session> {
    let session = store.create_session(parse_mode(&mode)?, None)?;
    sessions_changed(&app);
    Ok(session)
}

#[tauri::command]
pub async fn session_get(
    store: State<'_, Arc<Store>>,
    runs: State<'_, Arc<RunManager>>,
    id: String,
) -> Result<SessionView> {
    session_view(&store, &runs, &id)
}

#[tauri::command]
pub async fn session_update(
    app: AppHandle<Wry>,
    store: State<'_, Arc<Store>>,
    id: String,
    patch: SessionPatch,
) -> Result<Session> {
    let session = store.update_session(&id, &patch)?;
    sessions_changed(&app);
    Ok(session)
}

#[tauri::command]
pub async fn session_delete(
    app: AppHandle<Wry>,
    store: State<'_, Arc<Store>>,
    runs: State<'_, Arc<RunManager>>,
    agent: State<'_, Agent>,
    id: String,
) -> Result<()> {
    runs.cancel_session(&id);
    store.delete_session(&id)?;
    agent.forget(&id);
    sessions_changed(&app);
    Ok(())
}

/// Shows another version of a message (and the newest conversation after it).
#[tauri::command]
pub async fn branch_select(
    store: State<'_, Arc<Store>>,
    runs: State<'_, Arc<RunManager>>,
    session_id: String,
    message_id: String,
) -> Result<SessionView> {
    store.select_branch(&session_id, &message_id)?;
    session_view(&store, &runs, &session_id)
}

#[tauri::command]
pub async fn search(
    store: State<'_, Arc<Store>>,
    query: String,
    mode: Option<String>,
) -> Result<Vec<SearchHit>> {
    let mode = mode.as_deref().map(parse_mode).transpose()?;
    Ok(store.search(&query, mode, 30)?)
}

/// Saves a conversation as Markdown or JSON. Returns where it went, or `None` if cancelled.
#[tauri::command]
pub async fn session_export(
    store: State<'_, Arc<Store>>,
    id: String,
    format: String,
) -> Result<Option<String>> {
    let session = store.session(&id)?;
    let messages = store.active_path(&id)?;
    let (text, extension) = match format.as_str() {
        "json" => (
            serde_json::to_string_pretty(
                &serde_json::json!({ "session": session, "messages": messages }),
            )
            .map_err(|e| Error::Invalid(e.to_string()))?,
            "json",
        ),
        _ => (markdown(&session, &messages), "md"),
    };
    let stem = if session.title.trim().is_empty() {
        "Conversation".to_owned()
    } else {
        safe_file_name(&session.title)
    };
    let path = match crate::guard::test_save_path() {
        Some(path) => Some(path),
        None => rfd::AsyncFileDialog::new()
            .set_file_name(format!("{stem}.{extension}"))
            .add_filter(
                if extension == "md" {
                    "Markdown"
                } else {
                    "JSON"
                },
                &[extension],
            )
            .save_file()
            .await
            .map(|handle| handle.path().to_path_buf()),
    };
    let Some(path) = path else { return Ok(None) };
    hearth_core::fsutil::write_atomic(&path, text.as_bytes())?;
    Ok(Some(path.display().to_string()))
}

fn markdown(session: &Session, messages: &[Message]) -> String {
    let title = if session.title.trim().is_empty() {
        "Conversation"
    } else {
        session.title.as_str()
    };
    let mut out = format!("# {title}\n");
    for message in messages {
        let who = match message.role {
            hearth_core::store::MessageRole::User => "You".to_owned(),
            hearth_core::store::MessageRole::Assistant => message
                .model_label
                .clone()
                .unwrap_or_else(|| "Assistant".into()),
            _ => continue,
        };
        if message.content.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("\n**{who}**\n\n{}\n", message.content.trim()));
    }
    out
}

fn safe_file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if r#"<>:"/\|?*"#.contains(c) || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    cleaned
        .trim_end_matches(['.', ' '])
        .chars()
        .take(80)
        .collect()
}

#[tauri::command]
pub async fn connections_list(store: State<'_, Arc<Store>>) -> Result<Vec<Connection>> {
    Ok(store.connections()?)
}

#[tauri::command]
pub async fn connection_save(
    store: State<'_, Arc<Store>>,
    input: ConnectionInput,
) -> Result<Connection> {
    Ok(store.save_connection(&input)?)
}

#[tauri::command]
pub async fn connection_delete(store: State<'_, Arc<Store>>, id: String) -> Result<()> {
    Ok(store.delete_connection(&id)?)
}

/// The models a saved connection offers.
#[tauri::command]
pub async fn connection_models(store: State<'_, Arc<Store>>, id: String) -> Result<Vec<String>> {
    let connection = store.connection(&id)?;
    let client = LlmClient::new(&connection.base_url, connection.api_key)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    client
        .models()
        .await
        .map_err(|e| Error::Invalid(e.to_string()))
}

/// Checks an address (and key) before saving it: the models it offers, or why it failed.
#[tauri::command]
pub async fn connection_test(base_url: String, api_key: Option<String>) -> Result<Vec<String>> {
    let client = LlmClient::new(&base_url, api_key).map_err(|e| Error::Invalid(e.to_string()))?;
    client
        .models()
        .await
        .map_err(|e| Error::Invalid(e.to_string()))
}

struct ChannelSink(Channel<RunEnvelope>);

impl RunSink for ChannelSink {
    fn emit(&self, envelope: &RunEnvelope) {
        let _ = self.0.send(envelope.clone());
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStartArgs {
    session_id: String,
    action: ChatAction,
}

/// Starts a chat reply, or the agent on a task in a coding session.
#[tauri::command]
pub async fn run_start(
    app: AppHandle<Wry>,
    store: State<'_, Arc<Store>>,
    chat: State<'_, Chat>,
    agent: State<'_, Agent>,
    args: RunStartArgs,
    channel: Channel<RunEnvelope>,
) -> Result<StartedRun> {
    let sink = Arc::new(ChannelSink(channel));
    let started = match store.session(&args.session_id)?.mode {
        Mode::Chat => chat.start(&args.session_id, args.action, sink)?,
        Mode::Code => match args.action {
            ChatAction::Send { text, .. } => agent.start(&args.session_id, &text, sink)?,
            _ => {
                return Err(Error::Invalid(
                    "Editing or regenerating earlier messages isn't available in coding sessions yet."
                        .into(),
                ));
            }
        },
    };
    sessions_changed(&app);
    Ok(started)
}

/// Watches a run already in progress (after a reload), starting with a snapshot.
#[tauri::command]
pub async fn run_attach(
    runs: State<'_, Arc<RunManager>>,
    run_id: String,
    channel: Channel<RunEnvelope>,
) -> Result<bool> {
    Ok(runs.attach(&run_id, Arc::new(ChannelSink(channel))))
}

#[tauri::command]
pub async fn run_cancel(runs: State<'_, Arc<RunManager>>, run_id: String) -> Result<bool> {
    Ok(runs.cancel(&run_id))
}

#[tauri::command]
pub async fn runs_active(runs: State<'_, Arc<RunManager>>) -> Result<Vec<RunInfo>> {
    Ok(runs.active())
}

#[tauri::command]
pub async fn open_link(url: String) -> Result<()> {
    crate::links::open(&url)
}

#[cfg(test)]
mod tests {
    use super::safe_file_name;

    #[test]
    fn titles_become_safe_file_names() {
        assert_eq!(
            safe_file_name("What is C:\\Windows? <test>"),
            "What is C Windows test"
        );
        assert_eq!(safe_file_name("trailing dots..."), "trailing dots");
    }
}
