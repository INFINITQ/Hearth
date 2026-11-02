//! Commands for the terminal panel. Terminals run in Rust (`hearth_core::terminal`) and stream
//! to the UI over a channel; a reloaded UI attaches again and gets what they showed so far.

use std::path::PathBuf;
use std::sync::Arc;

use hearth_core::agent::shell::{Shell, ShellKind};
use hearth_core::settings::SettingsStore;
use hearth_core::store::Store;
use hearth_core::terminal::{Launch, TerminalEvent, TerminalInfo, TerminalSink, Terminals};
use tauri::State;
use tauri::ipc::Channel;

use crate::{Error, Result};

struct ChannelSink(Channel<TerminalEvent>);

impl TerminalSink for ChannelSink {
    fn send(&self, event: TerminalEvent) {
        // A closed page just misses the event; it gets the scrollback when it attaches again.
        let _ = self.0.send(event);
    }
}

fn shell_name(kind: ShellKind) -> &'static str {
    match kind {
        ShellKind::GitBash => "Git Bash",
        ShellKind::PowerShell => "PowerShell",
    }
}

/// Starts a shell (the chosen one, or the one set for the agent) in the project's folder, or in
/// the home folder without a project.
#[tauri::command]
pub async fn terminal_open(
    terminals: State<'_, Arc<Terminals>>,
    store: State<'_, Arc<Store>>,
    settings: State<'_, Arc<SettingsStore>>,
    shell: Option<ShellKind>,
    project_id: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<TerminalInfo> {
    let shell = match shell {
        Some(kind) => Shell::find(kind)
            .ok_or_else(|| Error::Invalid(format!("{} isn't installed.", shell_name(kind))))?,
        None => Shell::choose(settings.get().code.shell)
            .ok_or_else(|| Error::Invalid("Neither Git Bash nor PowerShell was found.".into()))?,
    };
    let cwd = match &project_id {
        Some(id) => PathBuf::from(store.project(id)?.path),
        None => std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir),
    };
    if !cwd.is_dir() {
        return Err(Error::Invalid(format!(
            "{} isn't there anymore.",
            cwd.display()
        )));
    }
    let terminals = terminals.inner().clone();
    let launch = Launch::shell(&shell);
    tauri::async_runtime::spawn_blocking(move || {
        terminals.open(&launch, &cwd, project_id, cols, rows)
    })
    .await
    .map_err(|err| Error::Invalid(err.to_string()))?
    .map_err(Error::from)
}

/// Streams a terminal's output to the page, starting with what it showed so far.
#[tauri::command]
pub async fn terminal_attach(
    terminals: State<'_, Arc<Terminals>>,
    id: String,
    channel: Channel<TerminalEvent>,
) -> Result<()> {
    Ok(terminals.attach(&id, Arc::new(ChannelSink(channel)))?)
}

#[tauri::command]
pub async fn terminal_write(
    terminals: State<'_, Arc<Terminals>>,
    id: String,
    data: String,
) -> Result<()> {
    Ok(terminals.write(&id, &data)?)
}

#[tauri::command]
pub async fn terminal_resize(
    terminals: State<'_, Arc<Terminals>>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<()> {
    Ok(terminals.resize(&id, cols, rows)?)
}

/// Ends the shell and everything started in it.
#[tauri::command]
pub async fn terminal_close(terminals: State<'_, Arc<Terminals>>, id: String) -> Result<()> {
    terminals.close(&id);
    Ok(())
}

#[tauri::command]
pub async fn terminals_list(terminals: State<'_, Arc<Terminals>>) -> Result<Vec<TerminalInfo>> {
    Ok(terminals.list())
}
