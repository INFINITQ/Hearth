//! Projects: the folders coding sessions work in (the `workspaces` table).

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use super::{Result, Store, StoreError};
use crate::{new_id, now_ms};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    /// The folder, as an absolute Windows path.
    pub path: String,
    pub name: String,
    /// The user said the folder's contents can be trusted (its AGENTS.md is followed).
    pub trusted: bool,
    pub created_at: i64,
    pub last_opened_at: i64,
}

const COLUMNS: &str = "id, path, name, trusted, created_at, last_opened_at";

fn row_to_project(row: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        trusted: row.get(3)?,
        created_at: row.get(4)?,
        last_opened_at: row.get(5)?,
    })
}

fn project_in(conn: &Connection, id: &str) -> Result<Project> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM workspaces WHERE id = ?1"),
        [id],
        row_to_project,
    )
    .optional()?
    .ok_or(StoreError::NotFound("project"))
}

/// `D:\code\app` → `app`; a drive root keeps its whole path.
fn folder_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(
        || path.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

impl Store {
    /// Adds a folder, or finds it again (paths compare case-insensitively), and marks it as
    /// opened now.
    pub fn open_project(&self, path: &str) -> Result<Project> {
        let conn = self.conn();
        // Strictly increasing, so the list follows the order folders were opened in even
        // within one millisecond.
        let latest: Option<i64> =
            conn.query_row("SELECT MAX(last_opened_at) FROM workspaces", [], |row| {
                row.get(0)
            })?;
        let now = now_ms().max(latest.map_or(0, |t| t + 1));
        let existing: Option<String> = conn
            .query_row("SELECT id FROM workspaces WHERE path = ?1", [path], |row| {
                row.get(0)
            })
            .optional()?;
        let id = match existing {
            Some(id) => {
                conn.execute(
                    "UPDATE workspaces SET last_opened_at = ?1 WHERE id = ?2",
                    params![now, id],
                )?;
                id
            }
            None => {
                let id = new_id();
                conn.execute(
                    "INSERT INTO workspaces (id, path, name, created_at, last_opened_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![id, path, folder_name(path), now],
                )?;
                id
            }
        };
        project_in(&conn, &id)
    }

    pub fn project(&self, id: &str) -> Result<Project> {
        project_in(&self.conn(), id)
    }

    /// Projects, most recently opened first.
    pub fn projects(&self, limit: usize) -> Result<Vec<Project>> {
        let conn = self.conn();
        let mut statement = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM workspaces ORDER BY last_opened_at DESC, id DESC LIMIT ?1"
        ))?;
        let rows = statement.query_map([limit as i64], row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn trust_project(&self, id: &str) -> Result<Project> {
        let conn = self.conn();
        if conn.execute("UPDATE workspaces SET trusted = 1 WHERE id = ?1", [id])? == 0 {
            return Err(StoreError::NotFound("project"));
        }
        project_in(&conn, id)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::{Mode, Store};

    #[test]
    fn projects_are_found_again_case_insensitively_and_listed_by_use() {
        let store = Store::open_in_memory().unwrap();
        let app = store.open_project(r"D:\code\app").unwrap();
        assert_eq!((app.name.as_str(), app.trusted), ("app", false));
        let other = store.open_project(r"D:\code\other").unwrap();
        let again = store.open_project(r"d:\CODE\app").unwrap();
        assert_eq!(again.id, app.id);
        assert_eq!(again.path, r"D:\code\app", "the first spelling is kept");
        let order: Vec<String> = store
            .projects(10)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(order, [app.id.clone(), other.id]);
        assert!(store.trust_project(&app.id).unwrap().trusted);
        assert_eq!(store.open_project(r"E:\").unwrap().name, r"E:\");
    }

    #[test]
    fn coding_sessions_belong_to_a_project() {
        let store = Store::open_in_memory().unwrap();
        let a = store.open_project(r"D:\a").unwrap();
        let b = store.open_project(r"D:\b").unwrap();
        let first = store.create_code_session(&a.id, Some("ask")).unwrap();
        store.create_code_session(&b.id, None).unwrap();
        store.create_session(Mode::Chat, None).unwrap();
        assert_eq!(first.permission_mode.as_deref(), Some("ask"));
        assert_eq!(first.workspace_id.as_deref(), Some(a.id.as_str()));
        let listed: Vec<String> = store
            .project_sessions(&a.id, 50, 0)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(listed, std::slice::from_ref(&first.id));

        store
            .set_env_snapshot(&first.id, "<environment>…</environment>")
            .unwrap();
        assert_eq!(
            store.session(&first.id).unwrap().env_snapshot.as_deref(),
            Some("<environment>…</environment>")
        );
    }
}
