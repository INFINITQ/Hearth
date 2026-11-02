//! Commands for Settings → MCP: the servers, their state and tools. Changes restart the server
//! concerned; every state change is also sent as `hearth://mcp`.

use hearth_core::mcp::{Mcp, ServerConfig, ServerView};
use tauri::State;

use crate::{Error, Result};

pub const MCP_EVENT: &str = "hearth://mcp";

fn plain(result: std::result::Result<Vec<ServerView>, String>) -> Result<Vec<ServerView>> {
    result.map_err(Error::Invalid)
}

#[tauri::command]
pub async fn mcp_servers(mcp: State<'_, Mcp>) -> Result<Vec<ServerView>> {
    Ok(mcp.servers())
}

/// Adds a server (no id) or changes one, then (re)starts it.
#[tauri::command]
pub async fn mcp_save(mcp: State<'_, Mcp>, config: ServerConfig) -> Result<Vec<ServerView>> {
    plain(mcp.save(config))
}

#[tauri::command]
pub async fn mcp_remove(mcp: State<'_, Mcp>, id: String) -> Result<Vec<ServerView>> {
    plain(mcp.remove(&id))
}

#[tauri::command]
pub async fn mcp_set_enabled(
    mcp: State<'_, Mcp>,
    id: String,
    enabled: bool,
) -> Result<Vec<ServerView>> {
    plain(mcp.set_enabled(&id, enabled))
}

/// Adds the servers in a Claude Desktop style config.
#[tauri::command]
pub async fn mcp_import(mcp: State<'_, Mcp>, json: String) -> Result<Vec<ServerView>> {
    plain(mcp.import(&json))
}

#[tauri::command]
pub async fn mcp_restart(mcp: State<'_, Mcp>, id: String) -> Result<Vec<ServerView>> {
    mcp.restart(&id);
    Ok(mcp.servers())
}
