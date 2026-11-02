//! What the agent changed in a session, and putting files back.
//!
//! Before the agent first changes a file in a task, the file is saved as a checkpoint (or noted
//! as not existing yet). A session's changes compare each file with its oldest checkpoint; undoing
//! a task restores every file changed in that task or later to how it was before. Changes made by
//! commands (builds, formatters, `git`) aren't tracked.

use std::path::Path;

use serde::Serialize;

use super::paths::Workspace;
use super::text::TextFile;
use crate::fsutil::write_atomic;
use crate::store::{Checkpoint, Store};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Relative to the project with `/` (absolute when outside it).
    pub path: String,
    /// The absolute path, for reverting.
    pub abs: String,
    pub kind: ChangeKind,
    /// Unified diff from the original; empty for binary files.
    pub diff: String,
    pub added: usize,
    pub removed: usize,
    pub binary: bool,
}

/// Each file's oldest checkpoint (from `turn` on), in the order files were first changed.
fn originals(checkpoints: Vec<Checkpoint>) -> Vec<Checkpoint> {
    let mut seen = std::collections::HashSet::new();
    checkpoints
        .into_iter()
        .filter(|c| seen.insert(c.path.to_lowercase()))
        .collect()
}

fn text_of(bytes: &[u8]) -> Option<String> {
    TextFile::decode(bytes).ok().map(|file| file.text)
}

pub fn changes(
    store: &Store,
    session_id: &str,
    workspace: &Workspace,
) -> Result<Vec<FileChange>, String> {
    let checkpoints = store
        .checkpoints(session_id, 0)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for original in originals(checkpoints) {
        let path = Path::new(&original.path);
        let now = std::fs::read(path).ok();
        let kind = match (&now, original.existed) {
            (None, false) => continue,
            (Some(_), false) => ChangeKind::Added,
            (None, true) => ChangeKind::Deleted,
            (Some(now), true) if *now == original.content => continue,
            (Some(_), true) => ChangeKind::Modified,
        };
        let before = if original.existed {
            text_of(&original.content)
        } else {
            Some(String::new())
        };
        let after = match &now {
            Some(bytes) => text_of(bytes),
            None => Some(String::new()),
        };
        let display = workspace.display(path);
        let (diff, added, removed, binary) = match (before, after) {
            (Some(before), Some(after)) => {
                let text_diff = similar::TextDiff::from_lines(&before, &after);
                let (mut added, mut removed) = (0, 0);
                for change in text_diff.iter_all_changes() {
                    match change.tag() {
                        similar::ChangeTag::Insert => added += 1,
                        similar::ChangeTag::Delete => removed += 1,
                        similar::ChangeTag::Equal => {}
                    }
                }
                let diff = text_diff
                    .unified_diff()
                    .context_radius(3)
                    .header(&format!("a/{display}"), &format!("b/{display}"))
                    .to_string();
                (diff, added, removed, false)
            }
            _ => (String::new(), 0, 0, true),
        };
        out.push(FileChange {
            path: display,
            abs: original.path,
            kind,
            diff,
            added,
            removed,
            binary,
        });
    }
    Ok(out)
}

fn restore(checkpoint: &Checkpoint) -> Result<(), String> {
    let path = Path::new(&checkpoint.path);
    if checkpoint.existed {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Couldn't restore {}: {e}", path.display()))?;
        }
        write_atomic(path, &checkpoint.content)
            .map_err(|e| format!("Couldn't restore {}: {e}", path.display()))
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("Couldn't remove {}: {err}", path.display())),
        }
    }
}

/// Puts one file back as it was before the session changed it.
pub fn revert(store: &Store, session_id: &str, abs: &str) -> Result<(), String> {
    let checkpoints = store
        .checkpoints(session_id, 0)
        .map_err(|e| e.to_string())?;
    let original = originals(checkpoints)
        .into_iter()
        .find(|c| c.path.eq_ignore_ascii_case(abs))
        .ok_or("Hearth has no earlier version of that file.")?;
    restore(&original)
}

/// Restores every file the agent changed in task `turn` or later to how it was before that
/// task. Returns the files put back (relative paths).
pub fn undo_from(
    store: &Store,
    session_id: &str,
    turn: i64,
    workspace: &Workspace,
) -> Result<Vec<String>, String> {
    let checkpoints = store
        .checkpoints(session_id, turn)
        .map_err(|e| e.to_string())?;
    let mut restored = Vec::new();
    for original in originals(checkpoints) {
        restore(&original)?;
        restored.push(workspace.display(Path::new(&original.path)));
    }
    store
        .drop_checkpoints(session_id, turn)
        .map_err(|e| e.to_string())?;
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_compare_with_the_first_state_and_undo_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();
        let root = workspace.root().to_path_buf();
        let store = Store::open_in_memory().unwrap();
        let project = store.open_project(&root.display().to_string()).unwrap();
        let session = store.create_code_session(&project.id, None).unwrap();
        let (a, b) = (root.join("a.txt"), root.join("new.txt"));
        let key = |p: &Path| p.display().to_string();
        std::fs::write(&a, "one\ntwo\n").unwrap();

        // Task 1 edits a.txt; task 2 edits it again and creates new.txt.
        store
            .save_checkpoint(&session.id, 1, &key(&a), true, b"one\ntwo\n")
            .unwrap();
        std::fs::write(&a, "one\n2\n").unwrap();
        store
            .save_checkpoint(&session.id, 2, &key(&a), true, b"one\n2\n")
            .unwrap();
        std::fs::write(&a, "1\n2\n").unwrap();
        store
            .save_checkpoint(&session.id, 2, &key(&b), false, b"")
            .unwrap();
        std::fs::write(&b, "hello\n").unwrap();

        let list = changes(&store, &session.id, &workspace).unwrap();
        let summary: Vec<(&str, ChangeKind, usize, usize)> = list
            .iter()
            .map(|c| (c.path.as_str(), c.kind, c.added, c.removed))
            .collect();
        assert_eq!(
            summary,
            [
                ("a.txt", ChangeKind::Modified, 2, 2),
                ("new.txt", ChangeKind::Added, 1, 0)
            ]
        );

        // Undoing task 2 goes back to the end of task 1.
        let restored = undo_from(&store, &session.id, 2, &workspace).unwrap();
        assert_eq!(restored, ["a.txt", "new.txt"]);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "one\n2\n");
        assert!(!b.exists());

        // Reverting a file goes back to before the session.
        revert(&store, &session.id, &key(&a)).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "one\ntwo\n");
        assert!(changes(&store, &session.id, &workspace).unwrap().is_empty());
    }
}
