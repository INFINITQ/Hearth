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
    /// Summaries: the last message they stand in for.
    pub covers_until: Option<String>,
    /// Files and images attached to a user message.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    /// A text or code file, or a PDF's text.
    Text,
    Image,
}

/// A file attached to a message. Its contents stay in Rust (the UI fetches an image when it
/// shows it), so transcripts stay light.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub kind: AttachmentKind,
    pub name: String,
    pub mime: String,
    /// The file's own size in bytes.
    pub size: i64,
    /// Characters of text (text files and PDFs).
    pub chars: i64,
    #[serde(skip_serializing)]
    pub text: Option<String>,
    /// Images: a `data:` URI.
    #[serde(skip_serializing)]
    pub data: Option<String>,
}

/// A file the user attaches (from the UI: text already extracted, images resized).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAttachment {
    pub kind: AttachmentKind,
    pub name: String,
    pub mime: String,
    pub size: i64,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
}

/// At most this many files on one message.
pub const MAX_ATTACHMENTS: usize = 10;
/// A file's text is cut here (about 100,000 tokens).
pub const MAX_ATTACHMENT_CHARS: usize = 400_000;
/// An image's data URI, at most.
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

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
    pub covers_until: Option<String>,
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
            covers_until: None,
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

    /// A summary that stands in for the messages up to `covers_until` in requests.
    pub fn summary(session_id: &str, parent_id: &str, text: &str, covers_until: &str) -> Self {
        Self {
            content: text.to_owned(),
            covers_until: Some(covers_until.to_owned()),
            ..Self::base(
                session_id,
                Some(parent_id),
                MessageRole::Summary,
                MessageStatus::Complete,
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
    usage, created_at, covers_until";

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
        covers_until: row.get(20)?,
        attachments: Vec::new(),
    })
}

fn row_to_attachment(row: &Row) -> rusqlite::Result<(String, Attachment)> {
    let kind: String = row.get(2)?;
    let text: Option<String> = row.get(6)?;
    let length: Option<i64> = row.get(8)?;
    Ok((
        row.get(1)?,
        Attachment {
            id: row.get(0)?,
            kind: if kind == "image" {
                AttachmentKind::Image
            } else {
                AttachmentKind::Text
            },
            name: row.get(3)?,
            mime: row.get(4)?,
            size: row.get(5)?,
            chars: length
                .unwrap_or_else(|| text.as_deref().map_or(0, |t| t.chars().count() as i64)),
            text,
            data: row.get(7)?,
        },
    ))
}

const ATTACHMENT_COLUMNS: &str = "id, message_id, kind, name, mime, size, text, data, NULL";
/// The same without the contents (`length(text)` stands in for the text).
const ATTACHMENT_SUMMARY: &str = "id, message_id, kind, name, mime, size, NULL, NULL, length(text)";

/// Root to `leaf`, with attachments (their contents only when asked).
fn path_in(conn: &Connection, leaf: &str, contents: bool) -> Result<Vec<Message>> {
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
    let mut messages: Vec<Message> = rows.collect::<rusqlite::Result<_>>()?;
    with_attachments(conn, &mut messages, contents)?;
    Ok(messages)
}

/// Saves checked attachments for a message, in order.
fn insert_attachments(conn: &Connection, message_id: &str, files: &[NewAttachment]) -> Result<()> {
    for (position, attachment) in files.iter().enumerate() {
        let kind = match attachment.kind {
            AttachmentKind::Text => "text",
            AttachmentKind::Image => "image",
        };
        conn.execute(
            "INSERT INTO attachments (id, message_id, position, kind, name, mime, size, text, data, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                new_id(),
                message_id,
                position as i64,
                kind,
                attachment.name.trim(),
                attachment.mime,
                attachment.size,
                attachment.text,
                attachment.data,
                now_ms()
            ],
        )?;
    }
    Ok(())
}

/// Fills in the user messages' attachments: with their contents (for requests), or just what
/// they are (for the transcript, which loads an image when it shows it).
fn with_attachments(conn: &Connection, messages: &mut [Message], contents: bool) -> Result<()> {
    let ids: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .map(|m| m.id.as_str())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let marks = vec!["?"; ids.len()].join(", ");
    let columns = if contents {
        ATTACHMENT_COLUMNS
    } else {
        ATTACHMENT_SUMMARY
    };
    let mut statement = conn.prepare(&format!(
        "SELECT {columns} FROM attachments WHERE message_id IN ({marks}) ORDER BY position"
    ))?;
    let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), row_to_attachment)?;
    let mut found: std::collections::HashMap<String, Vec<Attachment>> = Default::default();
    for row in rows {
        let (message_id, attachment) = row?;
        found.entry(message_id).or_default().push(attachment);
    }
    for message in messages {
        if let Some(attachments) = found.remove(&message.id) {
            message.attachments = attachments;
        }
    }
    Ok(())
}

impl Attachment {
    /// The same file, to attach again (an edited message keeps its files).
    pub fn to_new(&self) -> NewAttachment {
        NewAttachment {
            kind: self.kind,
            name: self.name.clone(),
            mime: self.mime.clone(),
            size: self.size,
            text: self.text.clone(),
            data: self.data.clone(),
        }
    }
}

/// Checks files before a message is saved with them.
pub fn check_attachments(attachments: &[NewAttachment]) -> Result<()> {
    if attachments.len() > MAX_ATTACHMENTS {
        return Err(StoreError::Invalid(format!(
            "At most {MAX_ATTACHMENTS} files can go with one message."
        )));
    }
    attachments.iter().try_for_each(|a| checked(a).map(|_| ()))
}

/// Checks what the UI sent, cutting text that's too long.
fn checked(attachment: &NewAttachment) -> Result<NewAttachment> {
    let invalid = |why: &str| StoreError::Invalid(format!("{}: {why}", attachment.name));
    let mut attachment = attachment.clone();
    match attachment.kind {
        AttachmentKind::Text => {
            let text = attachment.text.take().ok_or_else(|| invalid("no text"))?;
            let text = match text.char_indices().nth(MAX_ATTACHMENT_CHARS) {
                Some((cut, _)) => {
                    format!("{}\n… (the rest of this file was left out)", &text[..cut])
                }
                None => text,
            };
            attachment.text = Some(text);
            attachment.data = None;
        }
        AttachmentKind::Image => {
            let data = attachment.data.as_deref().unwrap_or_default();
            if !data.starts_with("data:image/") {
                return Err(invalid("not an image"));
            }
            if data.len() > MAX_IMAGE_BYTES {
                return Err(invalid("the image is too big"));
            }
            attachment.text = None;
        }
    }
    Ok(attachment)
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
        self.insert_message_with(new, &[])
    }

    /// Adds a message with files attached: all of it is saved, or none.
    pub fn insert_message_with(
        &self,
        new: NewMessage,
        attachments: &[NewAttachment],
    ) -> Result<Message> {
        check_attachments(attachments)?;
        let files: Vec<NewAttachment> = attachments.iter().map(checked).collect::<Result<_>>()?;
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
               tool_call_id, tool_name, tool_status, tool_display, model_label, status, created_at, covers_until)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
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
                now,
                new.covers_until
            ],
        )?;
        tx.execute(
            "UPDATE sessions SET active_leaf_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![id, now, new.session_id],
        )?;
        insert_attachments(&tx, &id, &files)?;
        let mut message = [message_in(&tx, &id)?];
        with_attachments(&tx, &mut message, false)?;
        let [message] = message;
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
        let conn = self.conn();
        let mut message = [message_in(&conn, id)?];
        with_attachments(&conn, &mut message, true)?;
        let [message] = message;
        Ok(message)
    }

    /// Saves files attached to a (user) message, in order.
    pub fn add_attachments(
        &self,
        message_id: &str,
        attachments: &[NewAttachment],
    ) -> Result<Vec<Attachment>> {
        if attachments.len() > MAX_ATTACHMENTS {
            return Err(StoreError::Invalid(format!(
                "At most {MAX_ATTACHMENTS} files can go with one message."
            )));
        }
        let checked: Vec<NewAttachment> = attachments.iter().map(checked).collect::<Result<_>>()?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        message_in(&tx, message_id)?;
        insert_attachments(&tx, message_id, &checked)?;
        tx.commit()?;
        // Read back on the same connection (its lock is still held).
        let mut saved = [message_in(&conn, message_id)?];
        with_attachments(&conn, &mut saved, false)?;
        let [saved] = saved;
        Ok(saved.attachments)
    }

    /// One attachment with its contents.
    pub fn attachment(&self, id: &str) -> Result<Attachment> {
        self.conn()
            .query_row(
                &format!("SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE id = ?1"),
                [id],
                row_to_attachment,
            )
            .optional()?
            .map(|(_, attachment)| attachment)
            .ok_or(StoreError::NotFound("attachment"))
    }

    /// The conversation as shown: root to active leaf.
    pub fn active_path(&self, session_id: &str) -> Result<Vec<Message>> {
        let conn = self.conn();
        let Some(leaf) = session_in(&conn, session_id)?.active_leaf_id else {
            return Ok(Vec::new());
        };
        // For showing: attachments without their contents.
        path_in(&conn, &leaf, false)
    }

    /// The messages from the root down to `message_id` (inclusive).
    pub fn path_to(&self, message_id: &str) -> Result<Vec<Message>> {
        let conn = self.conn();
        message_in(&conn, message_id)?;
        // For requests: attachments with their contents.
        path_in(&conn, message_id, true)
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
