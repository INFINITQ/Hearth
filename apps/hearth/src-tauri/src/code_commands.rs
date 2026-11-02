//! Commands for coding sessions: project folders, and answering the agent's permission prompts
//! and questions. Runs start through `run_start` like chats.

use std::path::Path;
use std::sync::Arc;

use hearth_core::agent::changes::FileChange;
use hearth_core::agent::shell::{Shell, ShellKind};
use hearth_core::agent::{Agent, PermissionMode, PermissionReply};
use hearth_core::store::{Project, Session, SessionSummary, Store};
use serde::Serialize;
use tauri::{AppHandle, State, Wry};

use crate::chat_commands::sessions_changed;
use crate::{Error, Result};

/// A project in the recent list, and whether its folder is still there.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    #[serde(flatten)]
    project: Project,
    exists: bool,
}

fn view(project: Project) -> ProjectView {
    let exists = Path::new(&project.path).is_dir();
    ProjectView { project, exists }
}

/// Adds (or finds) a folder by its real path, so the same folder is one project however it was
/// typed or picked.
fn open(store: &Store, path: &Path) -> Result<ProjectView> {
    let real = dunce::canonicalize(path)
        .map_err(|_| Error::Invalid(format!("{} can't be opened.", path.display())))?;
    if !real.is_dir() {
        return Err(Error::Invalid(format!(
            "{} isn't a folder.",
            real.display()
        )));
    }
    Ok(view(store.open_project(&real.display().to_string())?))
}

/// Shows the folder picker. `None` if it was cancelled.
#[tauri::command]
pub async fn project_pick(store: State<'_, Arc<Store>>) -> Result<Option<ProjectView>> {
    let picked = match crate::guard::test_pick_folder() {
        Some(path) => Some(path),
        None => rfd::AsyncFileDialog::new()
            .set_title("Open a project folder")
            .pick_folder()
            .await
            .map(|handle| handle.path().to_path_buf()),
    };
    picked.map(|path| open(&store, &path)).transpose()
}

/// Opens a project from the recent list (it moves to the top).
#[tauri::command]
pub async fn project_open(store: State<'_, Arc<Store>>, path: String) -> Result<ProjectView> {
    open(&store, Path::new(&path))
}

#[tauri::command]
pub async fn projects_recent(store: State<'_, Arc<Store>>) -> Result<Vec<ProjectView>> {
    Ok(store.projects(20)?.into_iter().map(view).collect())
}

#[tauri::command]
pub async fn project_trust(store: State<'_, Arc<Store>>, id: String) -> Result<ProjectView> {
    Ok(view(store.trust_project(&id)?))
}

#[tauri::command]
pub async fn project_sessions(
    store: State<'_, Arc<Store>>,
    project_id: String,
) -> Result<Vec<SessionSummary>> {
    Ok(store.project_sessions(&project_id, 200, 0)?)
}

#[tauri::command]
pub async fn code_session_create(
    app: AppHandle<Wry>,
    store: State<'_, Arc<Store>>,
    project_id: String,
    permission_mode: Option<String>,
) -> Result<Session> {
    store.project(&project_id)?;
    let mode = PermissionMode::parse(permission_mode.as_deref());
    let session = store.create_code_session(&project_id, Some(mode.as_str()))?;
    sessions_changed(&app);
    Ok(session)
}

/// Answers a permission prompt. False if it was already answered or the run ended.
#[tauri::command]
pub async fn run_permission(
    agent: State<'_, Agent>,
    request_id: String,
    reply: PermissionReply,
) -> Result<bool> {
    Ok(agent.resolve_permission(&request_id, reply))
}

/// Answers a question from the agent. False if it was already answered or the run ended.
#[tauri::command]
pub async fn run_answer(
    agent: State<'_, Agent>,
    question_id: String,
    answer: String,
) -> Result<bool> {
    Ok(agent.answer(&question_id, &answer))
}

/// A message for a task in progress: it joins the conversation before the agent's next step.
#[tauri::command]
pub async fn run_steer(agent: State<'_, Agent>, run_id: String, text: String) -> Result<()> {
    agent.steer(&run_id, &text).map_err(Error::Invalid)
}

/// The files the agent changed in a session, with diffs against how they were before.
#[tauri::command]
pub async fn session_changes(
    agent: State<'_, Agent>,
    session_id: String,
) -> Result<Vec<FileChange>> {
    agent.changes(&session_id).map_err(Error::Invalid)
}

/// Puts one file back as it was before the session changed it.
#[tauri::command]
pub async fn session_revert(
    agent: State<'_, Agent>,
    session_id: String,
    path: String,
) -> Result<()> {
    agent.revert(&session_id, &path).map_err(Error::Invalid)
}

/// Undoes the file changes of a task and every task after it. Returns the files put back.
#[tauri::command]
pub async fn session_undo(
    agent: State<'_, Agent>,
    session_id: String,
    turn: i64,
) -> Result<Vec<String>> {
    agent.undo(&session_id, turn).map_err(Error::Invalid)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellsInfo {
    git_bash: Option<String>,
    powershell: Option<String>,
}

/// Which shells this PC has, for Settings.
#[tauri::command]
pub async fn shells_info() -> Result<ShellsInfo> {
    let path = |kind| Shell::find(kind).map(|shell| shell.path.display().to_string());
    Ok(ShellsInfo {
        git_bash: path(ShellKind::GitBash),
        powershell: path(ShellKind::PowerShell),
    })
}
