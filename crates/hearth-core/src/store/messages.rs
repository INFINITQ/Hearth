//! Messages and the conversation tree.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::sessions::session_in;
use super::{Result, Store, StoreError, from_json, to_json};
use crate::{new_id, now_ms};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    /// A compaction summary standing in for older messages.
    Summary,
}

impl MessageRole {
    fn as_str(self) -> &'static str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
            MessageRole::Summary => "summary",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "assistant" => MessageRole::Assistant,
            "tool" => MessageRole::Tool,
            "summary" => MessageRole::Summary,
            _ => MessageRole::User,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageStatus {
    Streaming,
    Complete,
    /// Stopped by the user, or Hearth closed while it was streaming.
    Interrupted,
    Error,
}

impl MessageStatus {
    fn as_str(self) -> &'static str {
        match self {
            MessageStatus::Streaming => "streaming",
            MessageStatus::Complete => "complete",
            MessageStatus::Interrupted => "interrupted",
            MessageStatus::Error => "error",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "streaming" => MessageStatus::Streaming,
            "interrupted" => MessageStatus::Interrupted,
            "error" => MessageStatus::Error,
            _ => MessageStatus::Complete,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub role: MessageRole,
    pub turn: i64,
    pub step: i64,
    pub content: String,
    pub reasoning: Option<String>,
    pub reasoning_ms: Option<i64>,
    pub tool_calls: Option<Value>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_status: Option<String>,
    pub tool_display: Option<Value>,
    pub model_label: Option<String>,
    pub finish_reason: Option<String>,
    pub status: MessageStatus,
    pub error: Option<String>,
    pub usage: Option<Value>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewMessage {
    pub session_id: String,
    pub parent_id: Option<String>,
    pub role: MessageRole,
    pub turn: i64,
    pub step: i64,
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_calls: Option<Value>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_status: Option<String>,
    pub tool_display: Option<Value>,
    pub model_label: Option<String>,
    pub status: MessageStatus,
}

impl NewMessage {
    fn base(
        session_id: &str,
        parent_id: Option<&str>,
        role: MessageRole,
        status: MessageStatus,
    ) -> Self {
        Self {
            session_id: session_id.to_owned(),
            parent_id: parent_id.map(str::to_owned),
            role,
            turn: 0,
            step: 0,
            content: String::new(),
            reasoning: None,
            tool_calls: None,
            tool_call_id: None,
            tool_name: None,
            tool_status: None,
            tool_display: None,
            model_label: None,
            status,
        }
    }

    pub fn user(session_id: &str, parent_id: Option<&str>, text: &str) -> Self {
        Self {
            content: text.to_owned(),
            ..Self::base(
                session_id,
                parent_id,
                MessageRole::User,
                MessageStatus::Complete,
            )
        }
    }

    /// An assistant reply about to stream in.
    pub fn assistant(session_id: &str, parent_id: Option<&str>, model_label: &str) -> Self {
        Self {
            model_label: Some(model_label.to_owned()),
            ..Self::base(
                session_id,
                parent_id,
                MessageRole::Assistant,
                MessageStatus::Streaming,
            )
        }
    }

    pub fn tool(
        session_id: &str,
        parent_id: &str,
        call_id: &str,
        tool_name: &str,
        text: &str,
    ) -> Self {
        Self {
            content: text.to_owned(),
            tool_call_id: Some(call_id.to_owned()),
            tool_name: Some(tool_name.to_owned()),
            ..Self::base(
                session_id,
                Some(parent_id),
                MessageRole::Tool,
                MessageStatus::Complete,
            )
        }
    }
}

/// The final state of a streamed reply.
#[derive(Debug, Clone)]
pub struct MessageFinish {
    pub content: String,
    pub reasoning: Option<String>,
    pub reasoning_ms: Option<i64>,
    pub tool_calls: Option<Value>,
    pub finish_reason: Option<String>,
    pub status: MessageStatus,
    pub error: Option<String>,
    pub usage: Option<Value>,
}

impl MessageFinish {
    pub fn complete(content: &str) -> Self {
        Self {
            content: content.to_owned(),
            reasoning: None,
            reasoning_ms: None,
            tool_calls: None,
            finish_reason: Some("stop".into()),
            status: MessageStatus::Complete,
            error: None,
            usage: None,
        }
    }
}

const COLUMNS: &str = "id, session_id, parent_id, role, turn, step, content, reasoning, reasoning_ms, \
    tool_calls, tool_call_id, tool_name, tool_status, tool_display, model_label, finish_reason, status, error, \
    usage, created_at";

fn row_to_message(row: &Row) -> rusqlite::Result<Message> {
    let role: String = row.get(3)?;
    let status: String = row.get(16)?;
    Ok(Message {
        id: row.get(0)?,
        session_id: row.get(1)?,
        parent_id: row.get(2)?,
        role: MessageRole::parse(&role),
        turn: row.get(4)?,
        step: row.get(5)?,
        content: row.get(6)?,
        reasoning: row.get(7)?,
        reasoning_ms: row.get(8)?,
        tool_calls: from_json(row.get(9)?),
        tool_call_id: row.get(10)?,
        tool_name: row.get(11)?,
        tool_status: row.get(12)?,
        tool_display: from_json(row.get(13)?),
        model_label: row.get(14)?,
        finish_reason: row.get(15)?,
        status: MessageStatus::parse(&status),
        error: row.get(17)?,
        usage: from_json(row.get(18)?),
        created_at: row.get(19)?,
    })
}

fn message_in(conn: &Connection, id: &str) -> Result<Message> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM messages WHERE id = ?1"),
        [id],
        row_to_message,
    )
    .optional()?
    .ok_or(StoreError::NotFound("message"))
}

/// User and assistant text is searchable once it is final.
fn index(conn: &Connection, message: &Message) -> Result<()> {
    let searchable = matches!(message.role, MessageRole::User | MessageRole::Assistant)
        && message.status != MessageStatus::Streaming
        && !message.content.trim().is_empty();
    if searchable {
        conn.execute(
            "DELETE FROM messages_fts WHERE message_id = ?1",
            [&message.id],
        )?;
        conn.execute(
            "INSERT INTO messages_fts (message_id, session_id, body) VALUES (?1, ?2, ?3)",
            params![message.id, message.session_id, message.content],
        )?;
    }
    Ok(())
}

impl Store {
    /// Adds a message under `parent_id` (or as a new root) and makes it the active leaf.
    pub fn insert_message(&self, new: NewMessage) -> Result<Message> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        session_in(&tx, &new.session_id)?;
        if let Some(parent) = &new.parent_id {
            let owner: Option<String> = tx
                .query_row(
                    "SELECT session_id FROM messages WHERE id = ?1",
                    [parent],
                    |row| row.get(0),
                )
                .optional()?;
            if owner.as_deref() != Some(new.session_id.as_str()) {
                return Err(StoreError::Invalid(
                    "the parent message isn't in this conversation".into(),
                ));
            }
        }
        let id = new_id();
        let now = now_ms();
        tx.execute(
            "INSERT INTO messages (id, session_id, parent_id, role, turn, step, content, reasoning, tool_calls,
               tool_call_id, tool_name, tool_status, tool_display, model_label, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                id,
                new.session_id,
                new.parent_id,
                new.role.as_str(),
                new.turn,
                new.step,
                new.content,
                new.reasoning,
                to_json(new.tool_calls.as_ref()),
                new.tool_call_id,
                new.tool_name,
                new.tool_status,
                to_json(new.tool_display.as_ref()),
                new.model_label,
                new.status.as_str(),
                now
            ],
        )?;
        tx.execute(
            "UPDATE sessions SET active_leaf_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![id, now, new.session_id],
        )?;
        let message = message_in(&tx, &id)?;
        index(&tx, &message)?;
        tx.commit()?;
        Ok(message)
    }

    /// Saves a streaming reply's text so far (not indexed until it finishes).
    pub fn update_streaming(&self, id: &str, content: &str, reasoning: Option<&str>) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE messages SET content = ?1, reasoning = ?2 WHERE id = ?3",
            params![content, reasoning, id],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound("message"));
        }
        Ok(())
    }

    pub fn finish_message(&self, id: &str, finish: &MessageFinish) -> Result<Message> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE messages SET content = ?1, reasoning = ?2, reasoning_ms = ?3, tool_calls = ?4, finish_reason = ?5,
               status = ?6, error = ?7, usage = ?8 WHERE id = ?9",
            params![
                finish.content,
                finish.reasoning,
                finish.reasoning_ms,
                to_json(finish.tool_calls.as_ref()),
                finish.finish_reason,
                finish.status.as_str(),
                finish.error,
                to_json(finish.usage.as_ref()),
                id
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound("message"));
        }
        let message = message_in(&tx, id)?;
        index(&tx, &message)?;
        tx.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now_ms(), message.session_id],
        )?;
        tx.commit()?;
        Ok(message)
    }

    pub fn message(&self, id: &str) -> Result<Message> {
        message_in(&self.conn(), id)
    }

    /// The conversation as shown: root to active leaf.
    pub fn active_path(&self, session_id: &str) -> Result<Vec<Message>> {
        let conn = self.conn();
        let Some(leaf) = session_in(&conn, session_id)?.active_leaf_id else {
            return Ok(Vec::new());
        };
        let mut statement = conn.prepare(&format!(
            "WITH RECURSIVE path(id, depth) AS (
               SELECT id, 0 FROM messages WHERE id = ?1
               UNION ALL
               SELECT m.parent_id, p.depth + 1 FROM messages m JOIN path p ON m.id = p.id
               WHERE m.parent_id IS NOT NULL
             )
             SELECT {COLUMNS} FROM messages JOIN path USING (id) ORDER BY path.depth DESC"
        ))?;
        let rows = statement.query_map([leaf], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The messages from the root down to `message_id` (inclusive).
    pub fn path_to(&self, message_id: &str) -> Result<Vec<Message>> {
        let conn = self.conn();
        message_in(&conn, message_id)?;
        let mut statement = conn.prepare(&format!(
            "WITH RECURSIVE path(id, depth) AS (
               SELECT id, 0 FROM messages WHERE id = ?1
               UNION ALL
               SELECT m.parent_id, p.depth + 1 FROM messages m JOIN path p ON m.id = p.id
               WHERE m.parent_id IS NOT NULL
             )
             SELECT {COLUMNS} FROM messages JOIN path USING (id) ORDER BY path.depth DESC"
        ))?;
        let rows = statement.query_map([message_id], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The versions of a message: it and its siblings (same parent), oldest first.
    pub fn siblings(&self, message_id: &str) -> Result<Vec<String>> {
        let conn = self.conn();
        let message = message_in(&conn, message_id)?;
        let mut statement = conn.prepare(
            "SELECT id FROM messages
             WHERE session_id = ?1 AND parent_id IS ?2 AND role = ?3
             ORDER BY created_at, id",
        )?;
        let rows = statement.query_map(
            params![message.session_id, message.parent_id, message.role.as_str()],
            |row| row.get(0),
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Shows the branch through `message_id`, continuing to its newest leaf. Returns that leaf.
    pub fn select_branch(&self, session_id: &str, message_id: &str) -> Result<String> {
        let conn = self.conn();
        let message = message_in(&conn, message_id)?;
        if message.session_id != session_id {
            return Err(StoreError::Invalid(
                "that message isn't in this conversation".into(),
            ));
        }
        let leaf: String = conn.query_row(
            "WITH RECURSIVE sub(id, created_at) AS (
               SELECT id, created_at FROM messages WHERE id = ?1
               UNION ALL
               SELECT m.id, m.created_at FROM messages m JOIN sub s ON m.parent_id = s.id
             )
             SELECT id FROM sub s
             WHERE NOT EXISTS (SELECT 1 FROM messages c WHERE c.parent_id = s.id)
             ORDER BY created_at DESC, id DESC LIMIT 1",
            [message_id],
            |row| row.get(0),
        )?;
        conn.execute(
            "UPDATE sessions SET active_leaf_id = ?1 WHERE id = ?2",
            params![leaf, session_id],
        )?;
        Ok(leaf)
    }
}
