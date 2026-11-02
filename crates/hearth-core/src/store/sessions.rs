//! Sessions: one chat or one coding session each.

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Result, Store, StoreError, from_json, to_json};
use crate::{new_id, now_ms};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Chat,
    Code,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Chat => "chat",
            Mode::Code => "code",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "chat" => Some(Mode::Chat),
            "code" => Some(Mode::Code),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub mode: Mode,
    pub title: String,
    /// `none`, `auto` (generated) or `user` (typed): user titles are never replaced.
    pub title_source: String,
    pub workspace_id: Option<String>,
    pub model_ref: Option<Value>,
    pub params: Option<Value>,
    pub persona: Option<String>,
    pub permission_mode: Option<String>,
    pub search_mode: Option<String>,
    pub active_leaf_id: Option<String>,
    pub pinned: bool,
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What the sidebar needs about a session.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub mode: Mode,
    pub title: String,
    pub pinned: bool,
    pub updated_at: i64,
}

/// Changes to a session; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPatch {
    pub title: Option<String>,
    pub title_source: Option<String>,
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
    pub model_ref: Option<Value>,
    pub params: Option<Value>,
    pub persona: Option<String>,
    pub permission_mode: Option<String>,
    pub search_mode: Option<String>,
}

const COLUMNS: &str = "id, mode, title, title_source, workspace_id, model_ref, params, persona, \
    permission_mode, search_mode, active_leaf_id, pinned, archived, created_at, updated_at";

fn row_to_session(row: &Row) -> rusqlite::Result<Session> {
    let mode: String = row.get(1)?;
    Ok(Session {
        id: row.get(0)?,
        mode: Mode::parse(&mode).unwrap_or(Mode::Chat),
        title: row.get(2)?,
        title_source: row.get(3)?,
        workspace_id: row.get(4)?,
        model_ref: from_json(row.get(5)?),
        params: from_json(row.get(6)?),
        persona: row.get(7)?,
        permission_mode: row.get(8)?,
        search_mode: row.get(9)?,
        active_leaf_id: row.get(10)?,
        pinned: row.get(11)?,
        archived: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

pub(super) fn session_in(conn: &Connection, id: &str) -> Result<Session> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM sessions WHERE id = ?1"),
        [id],
        row_to_session,
    )
    .optional()?
    .ok_or(StoreError::NotFound("conversation"))
}

impl Store {
    pub fn create_session(&self, mode: Mode, model_ref: Option<Value>) -> Result<Session> {
        let conn = self.conn();
        let id = new_id();
        conn.execute(
            "INSERT INTO sessions (id, mode, model_ref, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, mode.as_str(), to_json(model_ref.as_ref()), now_ms()],
        )?;
        session_in(&conn, &id)
    }

    pub fn session(&self, id: &str) -> Result<Session> {
        session_in(&self.conn(), id)
    }

    /// Sessions of one mode for the sidebar: pinned first, then most recently used.
    pub fn sessions(&self, mode: Mode, limit: usize, offset: usize) -> Result<Vec<SessionSummary>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT id, mode, title, pinned, updated_at FROM sessions
             WHERE mode = ?1 AND archived = 0
             ORDER BY pinned DESC, updated_at DESC, id DESC LIMIT ?2 OFFSET ?3",
        )?;
        let rows =
            statement.query_map(params![mode.as_str(), limit as i64, offset as i64], |row| {
                Ok(SessionSummary {
                    id: row.get(0)?,
                    mode,
                    title: row.get(2)?,
                    pinned: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn update_session(&self, id: &str, patch: &SessionPatch) -> Result<Session> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        session_in(&tx, id)?;
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<SqlValue> = Vec::new();
        let mut set = |column: &'static str, value: SqlValue| {
            sets.push(column);
            values.push(value);
        };
        let text = |value: &Option<String>| value.clone().map_or(SqlValue::Null, SqlValue::Text);
        let json = |value: &Value| {
            if value.is_null() {
                SqlValue::Null
            } else {
                SqlValue::Text(value.to_string())
            }
        };
        if let Some(title) = &patch.title {
            set("title", SqlValue::Text(title.trim().to_owned()));
            set(
                "title_source",
                SqlValue::Text(patch.title_source.clone().unwrap_or_else(|| "user".into())),
            );
        }
        if let Some(pinned) = patch.pinned {
            set("pinned", SqlValue::Integer(pinned.into()));
        }
        if let Some(archived) = patch.archived {
            set("archived", SqlValue::Integer(archived.into()));
        }
        if let Some(model_ref) = &patch.model_ref {
            set("model_ref", json(model_ref));
        }
        if let Some(params) = &patch.params {
            set("params", json(params));
        }
        if patch.persona.is_some() {
            set("persona", text(&patch.persona));
        }
        if patch.permission_mode.is_some() {
            set("permission_mode", text(&patch.permission_mode));
        }
        if patch.search_mode.is_some() {
            set("search_mode", text(&patch.search_mode));
        }
        if !sets.is_empty() {
            let assignments: Vec<String> =
                sets.iter().map(|column| format!("{column} = ?")).collect();
            values.push(SqlValue::Text(id.to_owned()));
            tx.execute(
                &format!(
                    "UPDATE sessions SET {} WHERE id = ?",
                    assignments.join(", ")
                ),
                params_from_iter(values),
            )?;
        }
        if let Some(title) = &patch.title {
            tx.execute("DELETE FROM titles_fts WHERE session_id = ?1", [id])?;
            if !title.trim().is_empty() {
                tx.execute(
                    "INSERT INTO titles_fts (session_id, title) VALUES (?1, ?2)",
                    params![id, title.trim()],
                )?;
            }
        }
        let session = session_in(&tx, id)?;
        tx.commit()?;
        Ok(session)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM messages_fts WHERE session_id = ?1", [id])?;
        tx.execute("DELETE FROM titles_fts WHERE session_id = ?1", [id])?;
        tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }
}
