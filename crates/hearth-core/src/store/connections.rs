//! Connections: OpenAI-compatible servers Hearth can talk to (a llama-server started by hand,
//! LM Studio, Ollama's `/v1`, a remote machine). Hearth's own managed llama-server models come
//! later as presets.

use rusqlite::{Connection as Db, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::{Result, Store, StoreError};
use crate::{new_id, now_ms};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub base_url: String,
    /// Never sent to the UI; it only learns whether a key is set.
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
    pub has_api_key: bool,
    /// The server is llama.cpp's, so its extensions (thinking switches, prompt progress,
    /// prompt cache) can be used.
    pub llama_extensions: bool,
    pub created_at: i64,
}

/// A new or edited connection from the UI.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInput {
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    /// `None` keeps the current key; an empty string removes it.
    pub api_key: Option<String>,
    #[serde(default)]
    pub llama_extensions: bool,
}

fn row_to_connection(row: &Row) -> rusqlite::Result<Connection> {
    let api_key: Option<String> = row.get(3)?;
    Ok(Connection {
        id: row.get(0)?,
        name: row.get(1)?,
        base_url: row.get(2)?,
        has_api_key: api_key.as_deref().is_some_and(|k| !k.is_empty()),
        api_key,
        llama_extensions: row.get(4)?,
        created_at: row.get(5)?,
    })
}

const COLUMNS: &str = "id, name, base_url, api_key, llama_extensions, created_at";

fn connection_in(db: &Db, id: &str) -> Result<Connection> {
    db.query_row(
        &format!("SELECT {COLUMNS} FROM connections WHERE id = ?1"),
        [id],
        row_to_connection,
    )
    .optional()?
    .ok_or(StoreError::NotFound("connection"))
}

impl Store {
    pub fn connections(&self) -> Result<Vec<Connection>> {
        let db = self.conn();
        let mut statement = db.prepare(&format!(
            "SELECT {COLUMNS} FROM connections ORDER BY created_at"
        ))?;
        let rows = statement.query_map([], row_to_connection)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn connection(&self, id: &str) -> Result<Connection> {
        connection_in(&self.conn(), id)
    }

    pub fn save_connection(&self, input: &ConnectionInput) -> Result<Connection> {
        let name = input.name.trim();
        let base_url = input.base_url.trim();
        if name.is_empty() || base_url.is_empty() {
            return Err(StoreError::Invalid(
                "A connection needs a name and an address.".into(),
            ));
        }
        let db = self.conn();
        match &input.id {
            Some(id) => {
                connection_in(&db, id)?;
                db.execute(
                    "UPDATE connections SET name = ?1, base_url = ?2, llama_extensions = ?3 WHERE id = ?4",
                    params![name, base_url, input.llama_extensions, id],
                )?;
                if let Some(key) = &input.api_key {
                    let key = (!key.is_empty()).then_some(key.as_str());
                    db.execute(
                        "UPDATE connections SET api_key = ?1 WHERE id = ?2",
                        params![key, id],
                    )?;
                }
                connection_in(&db, id)
            }
            None => {
                let id = new_id();
                let key = input.api_key.as_deref().filter(|k| !k.is_empty());
                db.execute(
                    "INSERT INTO connections (id, name, base_url, api_key, llama_extensions, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, name, base_url, key, input.llama_extensions, now_ms()],
                )?;
                connection_in(&db, &id)
            }
        }
    }

    pub fn delete_connection(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM connections WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, url: &str, key: Option<&str>) -> ConnectionInput {
        ConnectionInput {
            id: None,
            name: name.into(),
            base_url: url.into(),
            api_key: key.map(String::from),
            llama_extensions: true,
        }
    }

    #[test]
    fn connections_are_saved_edited_and_keep_their_key_private() {
        let store = Store::open_in_memory().unwrap();
        let saved = store
            .save_connection(&input(
                "LM Studio",
                "http://localhost:1234/v1",
                Some("sk-1"),
            ))
            .unwrap();
        assert!(saved.has_api_key);
        let json = serde_json::to_value(&saved).unwrap();
        assert!(json.get("apiKey").is_none(), "the key never goes to the UI");

        let edited = store
            .save_connection(&ConnectionInput {
                id: Some(saved.id.clone()),
                api_key: None,
                ..input("Studio", "http://localhost:1234", None)
            })
            .unwrap();
        assert_eq!(
            (edited.name.as_str(), edited.api_key.as_deref()),
            ("Studio", Some("sk-1"))
        );

        let cleared = store
            .save_connection(&ConnectionInput {
                id: Some(saved.id.clone()),
                api_key: Some(String::new()),
                ..input("Studio", "x", None)
            })
            .unwrap();
        assert!(!cleared.has_api_key);

        store.delete_connection(&saved.id).unwrap();
        assert!(store.connections().unwrap().is_empty());
    }

    #[test]
    fn a_connection_needs_a_name_and_address() {
        let store = Store::open_in_memory().unwrap();
        assert!(
            store
                .save_connection(&input(" ", "http://x", None))
                .is_err()
        );
        assert!(store.save_connection(&input("x", "", None)).is_err());
    }
}
