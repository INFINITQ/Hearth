//! Schema migrations, tracked with SQLite's `user_version`. Append only: never edit a migration
//! that has shipped; add a new one.

use rusqlite::Connection;

use super::{Result, StoreError};

const MIGRATIONS: &[&str] = &[INITIAL, PROJECTS];

const INITIAL: &str = r#"
CREATE TABLE kv (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  mode TEXT NOT NULL CHECK (mode IN ('chat', 'code')),
  title TEXT NOT NULL DEFAULT '',
  title_source TEXT NOT NULL DEFAULT 'none',
  workspace_id TEXT,
  model_ref TEXT,
  params TEXT,
  persona TEXT,
  permission_mode TEXT,
  search_mode TEXT,
  active_leaf_id TEXT,
  pinned INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX sessions_by_mode ON sessions (mode, archived, pinned, updated_at DESC);

CREATE TABLE messages (
  id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
  parent_id TEXT REFERENCES messages (id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'tool', 'summary')),
  turn INTEGER NOT NULL DEFAULT 0,
  step INTEGER NOT NULL DEFAULT 0,
  content TEXT NOT NULL DEFAULT '',
  reasoning TEXT,
  reasoning_ms INTEGER,
  tool_calls TEXT,
  tool_call_id TEXT,
  tool_name TEXT,
  tool_status TEXT,
  tool_display TEXT,
  model_label TEXT,
  finish_reason TEXT,
  status TEXT NOT NULL DEFAULT 'complete'
    CHECK (status IN ('streaming', 'complete', 'interrupted', 'error')),
  error TEXT,
  usage TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX messages_by_session ON messages (session_id, created_at);
CREATE INDEX messages_by_parent ON messages (parent_id, created_at);

-- Search keeps its own copies of the text (rowid-based external content breaks after VACUUM
-- on tables with TEXT primary keys).
CREATE VIRTUAL TABLE messages_fts USING fts5 (
  message_id UNINDEXED,
  session_id UNINDEXED,
  body,
  tokenize = 'unicode61 remove_diacritics 2'
);
CREATE VIRTUAL TABLE titles_fts USING fts5 (
  session_id UNINDEXED,
  title,
  tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE connections (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  api_key TEXT,
  llama_extensions INTEGER NOT NULL DEFAULT 0,
  caps TEXT,
  created_at INTEGER NOT NULL
);
"#;

/// M3: project folders for coding sessions, and each session's frozen environment (the part of
/// the agent's prompt that must not change between requests).
const PROJECTS: &str = r#"
CREATE TABLE workspaces (
  id TEXT PRIMARY KEY,
  path TEXT NOT NULL UNIQUE COLLATE NOCASE,
  name TEXT NOT NULL,
  trusted INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  last_opened_at INTEGER NOT NULL
);
ALTER TABLE sessions ADD COLUMN env_snapshot TEXT;
CREATE INDEX sessions_by_workspace ON sessions (workspace_id, archived, pinned, updated_at DESC);
"#;

pub fn apply(conn: &mut Connection) -> Result<()> {
    let version =
        conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))? as usize;
    if version > MIGRATIONS.len() {
        return Err(StoreError::Invalid(format!(
            "This database was made by a newer version of Hearth (schema {version})."
        )));
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (index + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_once_and_refuse_newer_databases() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();
        apply(&mut conn).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version as usize, MIGRATIONS.len());

        conn.pragma_update(None, "user_version", 999).unwrap();
        assert!(matches!(apply(&mut conn), Err(StoreError::Invalid(_))));
    }
}
