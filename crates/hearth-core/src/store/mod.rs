//! Hearth's database: conversations stored as message trees, plus full-text search. One SQLite
//! file in the data folder, in WAL mode.
//!
//! Messages form a tree through `parent_id`: editing a message or regenerating a reply adds a
//! sibling, and the session remembers which leaf is active. The conversation shown (and sent to
//! the model) is the path from the root to that leaf.

mod checkpoints;
mod connections;
mod messages;
mod migrations;
mod projects;
mod search;
mod sessions;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection as Db;
use serde_json::Value;

pub use checkpoints::Checkpoint;
pub use connections::{Connection, ConnectionInput};
pub use messages::{Message, MessageFinish, MessageRole, MessageStatus, NewMessage};
pub use projects::Project;
pub use search::SearchHit;
pub use sessions::{Mode, Session, SessionPatch, SessionSummary};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("The {0} doesn't exist (any more).")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

pub struct Store {
    conn: Mutex<Db>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Db::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Db::open_in_memory()?)
    }

    fn init(mut conn: Db) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        migrations::apply(&mut conn)?;
        // Replies that were streaming when Hearth last stopped will never finish.
        conn.execute(
            "UPDATE messages SET status = 'interrupted' WHERE status = 'streaming'",
            [],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Db> {
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn to_json(value: Option<&Value>) -> Option<String> {
    value.map(Value::to_string)
}

fn from_json(text: Option<String>) -> Option<Value> {
    text.and_then(|text| serde_json::from_str(&text).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn user(store: &Store, session: &str, parent: Option<&str>, text: &str) -> Message {
        store
            .insert_message(NewMessage::user(session, parent, text))
            .unwrap()
    }

    fn reply(store: &Store, session: &str, parent: &str, text: &str) -> Message {
        let message = store
            .insert_message(NewMessage::assistant(session, Some(parent), "mock"))
            .unwrap();
        store
            .finish_message(&message.id, &MessageFinish::complete(text))
            .unwrap()
    }

    fn path_texts(store: &Store, session: &str) -> Vec<String> {
        store
            .active_path(session)
            .unwrap()
            .into_iter()
            .map(|m| m.content)
            .collect()
    }

    #[test]
    fn a_conversation_is_the_path_to_the_active_leaf() {
        let store = store();
        let session = store
            .create_session(Mode::Chat, Some(json!({"kind": "mock"})))
            .unwrap();
        let q1 = user(&store, &session.id, None, "Hi");
        let a1 = reply(&store, &session.id, &q1.id, "Hello!");
        user(&store, &session.id, Some(&a1.id), "How are you?");
        assert_eq!(
            path_texts(&store, &session.id),
            ["Hi", "Hello!", "How are you?"]
        );
        assert_eq!(
            store.session(&session.id).unwrap().model_ref,
            Some(json!({"kind": "mock"}))
        );
    }

    #[test]
    fn regenerating_adds_a_sibling_and_switching_back_restores_the_branch() {
        let store = store();
        let session = store.create_session(Mode::Chat, None).unwrap();
        let q = user(&store, &session.id, None, "Name a color");
        let first = reply(&store, &session.id, &q.id, "Red");
        let follow = user(&store, &session.id, Some(&first.id), "Why?");
        let second = reply(&store, &session.id, &q.id, "Blue");

        assert_eq!(path_texts(&store, &session.id), ["Name a color", "Blue"]);
        assert_eq!(
            store.siblings(&second.id).unwrap(),
            [first.id.clone(), second.id.clone()]
        );

        // Switching to the first reply restores its newest continuation.
        let leaf = store.select_branch(&session.id, &first.id).unwrap();
        assert_eq!(leaf, follow.id);
        assert_eq!(
            path_texts(&store, &session.id),
            ["Name a color", "Red", "Why?"]
        );
    }

    #[test]
    fn editing_the_first_message_makes_a_second_root() {
        let store = store();
        let session = store.create_session(Mode::Chat, None).unwrap();
        let original = user(&store, &session.id, None, "Draft one");
        let edited = user(&store, &session.id, None, "Draft two");
        assert_eq!(
            store.siblings(&edited.id).unwrap(),
            [original.id, edited.id]
        );
        assert_eq!(path_texts(&store, &session.id), ["Draft two"]);
    }

    #[test]
    fn a_parent_from_another_session_is_rejected() {
        let store = store();
        let a = store.create_session(Mode::Chat, None).unwrap();
        let b = store.create_session(Mode::Chat, None).unwrap();
        let q = user(&store, &a.id, None, "Hi");
        let err = store
            .insert_message(NewMessage::user(&b.id, Some(&q.id), "x"))
            .unwrap_err();
        assert!(matches!(err, StoreError::Invalid(_)), "{err}");
    }

    #[test]
    fn sessions_list_pinned_first_then_most_recent() {
        let store = store();
        let old = store.create_session(Mode::Chat, None).unwrap();
        let pinned = store.create_session(Mode::Chat, None).unwrap();
        let newest = store.create_session(Mode::Chat, None).unwrap();
        store.create_session(Mode::Code, None).unwrap();
        store
            .update_session(
                &pinned.id,
                &SessionPatch {
                    pinned: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        user(&store, &newest.id, None, "bump");
        let ids: Vec<String> = store
            .sessions(Mode::Chat, 50, 0)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, [pinned.id, newest.id, old.id]);
    }

    #[test]
    fn search_finds_finished_messages_and_titles() {
        let store = store();
        let session = store.create_session(Mode::Chat, None).unwrap();
        let q = user(&store, &session.id, None, "Tell me about tokenizers");
        let streaming = store
            .insert_message(NewMessage::assistant(&session.id, Some(&q.id), "mock"))
            .unwrap();
        store
            .update_streaming(&streaming.id, "Byte-pair encoding merges", None)
            .unwrap();
        assert!(
            store.search("merges", None, 10).unwrap().is_empty(),
            "unfinished replies aren't indexed"
        );

        store
            .finish_message(
                &streaming.id,
                &MessageFinish::complete("Byte-pair encoding merges frequent pairs"),
            )
            .unwrap();
        let hits = store.search("merg", Some(Mode::Chat), 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id.as_deref(), Some(streaming.id.as_str()));
        assert!(
            hits[0].snippet.contains("\u{2}merges\u{3}"),
            "{}",
            hits[0].snippet
        );

        store
            .update_session(
                &session.id,
                &SessionPatch {
                    title: Some("Tokenizer basics".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let hits = store.search("basics", None, 10).unwrap();
        assert_eq!(hits[0].session_id, session.id);
        assert!(
            store
                .search("basics", Some(Mode::Code), 10)
                .unwrap()
                .is_empty()
        );
        assert!(store.search("  \"  ", None, 10).unwrap().is_empty());
    }

    #[test]
    fn deleting_a_session_removes_its_messages_and_search_entries() {
        let store = store();
        let session = store.create_session(Mode::Chat, None).unwrap();
        let q = user(&store, &session.id, None, "remember the walrus");
        store.delete_session(&session.id).unwrap();
        assert!(matches!(
            store.session(&session.id),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(store.message(&q.id), Err(StoreError::NotFound(_))));
        assert!(store.search("walrus", None, 10).unwrap().is_empty());
    }

    #[test]
    fn replies_left_streaming_are_marked_interrupted_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hearth.db");
        let id = {
            let store = Store::open(&path).unwrap();
            let session = store.create_session(Mode::Chat, None).unwrap();
            let q = user(&store, &session.id, None, "Hi");
            store
                .insert_message(NewMessage::assistant(&session.id, Some(&q.id), "mock"))
                .unwrap()
                .id
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.message(&id).unwrap().status,
            MessageStatus::Interrupted
        );
    }
}
