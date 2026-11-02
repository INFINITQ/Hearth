//! Checkpoints: each file as it was before the agent first changed it in a task, so a task's
//! changes can be undone and the session's changes shown. And "Always allow" command rules.

use rusqlite::params;

use super::{Result, Store};
use crate::now_ms;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub turn: i64,
    /// Absolute path.
    pub path: String,
    /// Whether the file existed before (if not, undoing deletes it).
    pub existed: bool,
    pub content: Vec<u8>,
}

impl Store {
    /// Remembers a file before its first change in a task (later calls for the same task and
    /// file are ignored: the first state is the one to go back to).
    pub fn save_checkpoint(
        &self,
        session_id: &str,
        turn: i64,
        path: &str,
        existed: bool,
        content: &[u8],
    ) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO checkpoints (session_id, turn, path, existed, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, turn, path, existed, content, now_ms()],
        )?;
        Ok(())
    }

    /// A session's checkpoints from task `turn` on, oldest task first.
    pub fn checkpoints(&self, session_id: &str, from_turn: i64) -> Result<Vec<Checkpoint>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT turn, path, existed, content FROM checkpoints
             WHERE session_id = ?1 AND turn >= ?2 ORDER BY turn, id",
        )?;
        let rows = statement.query_map(params![session_id, from_turn], |row| {
            Ok(Checkpoint {
                turn: row.get(0)?,
                path: row.get(1)?,
                existed: row.get(2)?,
                content: row.get::<_, Option<Vec<u8>>>(3)?.unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Forgets checkpoints from task `turn` on (after they were undone).
    pub fn drop_checkpoints(&self, session_id: &str, from_turn: i64) -> Result<()> {
        self.conn().execute(
            "DELETE FROM checkpoints WHERE session_id = ?1 AND turn >= ?2",
            params![session_id, from_turn],
        )?;
        Ok(())
    }

    /// The project's "Always allow" patterns of one kind (e.g. command prefixes).
    pub fn rules(&self, project_id: &str, class: &str) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT pattern FROM permission_rules WHERE workspace_id = ?1 AND class = ?2 ORDER BY id",
        )?;
        let rows = statement.query_map(params![project_id, class], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_rule(&self, project_id: &str, class: &str, pattern: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO permission_rules (workspace_id, class, pattern, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![project_id, class, pattern, now_ms()],
        )?;
        Ok(())
    }

    pub fn remove_rule(&self, project_id: &str, class: &str, pattern: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM permission_rules WHERE workspace_id = ?1 AND class = ?2 AND pattern = ?3",
            params![project_id, class, pattern],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[test]
    fn the_first_state_in_a_task_is_kept() {
        let store = Store::open_in_memory().unwrap();
        let project = store.open_project(r"D:\p").unwrap();
        let session = store.create_code_session(&project.id, None).unwrap();
        store
            .save_checkpoint(&session.id, 1, r"D:\p\a.txt", true, b"one")
            .unwrap();
        store
            .save_checkpoint(&session.id, 1, r"d:\P\A.TXT", true, b"two")
            .unwrap();
        store
            .save_checkpoint(&session.id, 2, r"D:\p\b.txt", false, b"")
            .unwrap();
        let all = store.checkpoints(&session.id, 1).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].content, b"one");
        assert!(!all[1].existed);
        store.drop_checkpoints(&session.id, 2).unwrap();
        assert_eq!(store.checkpoints(&session.id, 0).unwrap().len(), 1);

        store.add_rule(&project.id, "execute", "npm test").unwrap();
        store.add_rule(&project.id, "execute", "npm test").unwrap();
        assert_eq!(store.rules(&project.id, "execute").unwrap(), ["npm test"]);
        store
            .remove_rule(&project.id, "execute", "npm test")
            .unwrap();
        assert!(store.rules(&project.id, "execute").unwrap().is_empty());
    }
}
