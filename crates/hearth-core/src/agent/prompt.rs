//! The agent's system prompt: fixed rules for working with the tools, then a snapshot of the
//! environment and the project's own instructions (AGENTS.md, or CLAUDE.md), taken when the
//! session's first task starts. The prompt stays byte-identical for the whole session, so
//! llama-server can keep reusing its cached copy of it.

use std::path::Path;

use super::paths::Workspace;
use super::text::TextFile;
use super::tools::walker;

pub const RULES: &str = "You are Hearth's coding agent, working in the user's project on their Windows PC. \
You complete coding tasks with the tools provided: find and read the relevant code, change it, and explain the result.

Working rules:
1. Look before you change anything. Find files with glob, grep and list_dir, and read them with read_file. \
Don't guess at file contents, names or APIs.
2. Change files with edit_file: copy old_string exactly from read_file's output (without the line-number prefix) and \
include enough surrounding lines for it to be unique. Use write_file for new files, or to replace a whole file.
3. Keep changes focused on the task. Match the project's style, and don't reformat or rewrite code you weren't asked \
to change.
4. For a task with several steps, keep a to-do list with todo_write and mark items done as you go.
5. When a tool reports an error, read it: it says what went wrong and how to fix the call. Don't repeat a call that \
failed the same way.
6. Use ask_user only when you can't continue without the user's decision. Otherwise make a sensible choice and \
mention it.
7. Paths are relative to the project folder, with forward slashes. Stay inside the project unless the user asks \
otherwise.
8. Be brief. Don't narrate every step. When you finish, say in a few sentences what you changed and anything the \
user should check.";

/// Added to the newest user message while Plan mode is on (never saved).
pub const PLAN_REMINDER: &str = "\n\n(Plan mode is on: you can look around but not change anything. \
Investigate, then reply with a short step-by-step plan.)";

/// Files with instructions for agents, in order of preference.
const INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md"];
const MAX_INSTRUCTIONS: usize = 16 * 1024;
const MAX_TOP_LEVEL: usize = 60;

/// The environment part of the prompt, frozen when a session's first task starts. Project
/// instructions are read only from folders the user trusts.
pub fn snapshot(workspace: &Workspace, today: &str, trusted: bool) -> String {
    let root = workspace.root();
    let git = if root.join(".git").exists() {
        "yes"
    } else {
        "no"
    };
    let mut out = format!(
        "<environment>\nOperating system: Windows 11\nProject folder: {} (in Git Bash: {})\nToday's date: {today}\n\
         Git repository: {git}\nAt the top of the project:\n{}</environment>",
        root.display(),
        workspace.posix_root(),
        top_level(root)
    );
    if trusted && let Some((file, text)) = instructions(root) {
        out.push_str(&format!(
            "\n\n<project_instructions file=\"{file}\">\n{text}\n</project_instructions>\n\
             These instructions come from the project's {file}. Follow them for how to work in this project; \
             the user's requests come first."
        ));
    }
    out
}

pub fn system_prompt(snapshot: &str) -> String {
    format!("{RULES}\n\n{snapshot}")
}

/// Folders first, then files, as git sees them.
fn top_level(root: &Path) -> String {
    let mut folders = Vec::new();
    let mut files = Vec::new();
    for entry in walker(root, Some(1), false).flatten() {
        if entry.depth() == 0 {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            folders.push(format!("{name}/"));
        } else {
            files.push(name);
        }
    }
    let total = folders.len() + files.len();
    if total == 0 {
        return "(empty)\n".into();
    }
    let mut lines: Vec<String> = folders
        .into_iter()
        .chain(files)
        .take(MAX_TOP_LEVEL)
        .collect();
    if total > MAX_TOP_LEVEL {
        lines.push(format!("(+{} more)", total - MAX_TOP_LEVEL));
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn instructions(root: &Path) -> Option<(&'static str, String)> {
    INSTRUCTION_FILES.iter().find_map(|file| {
        let text = TextFile::read(&root.join(file)).ok()?.text;
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let text = if text.len() > MAX_INSTRUCTIONS {
            let mut cut = MAX_INSTRUCTIONS;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            format!(
                "{}\n(… the rest of {file} was left out: it's longer than 16 KB.)",
                &text[..cut]
            )
        } else {
            text.to_owned()
        };
        Some((*file, text))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_describes_the_project_and_its_instructions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("README.md"), "hi").unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "Use tabs.").unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            "Run `make test` after changes.\n",
        )
        .unwrap();
        let workspace = Workspace::new(dir.path()).unwrap();

        let snapshot = snapshot(&workspace, "2026-11-02", true);
        assert!(snapshot.contains("Today's date: 2026-11-02"));
        assert!(snapshot.contains("Git repository: no"));
        assert!(
            snapshot
                .contains("At the top of the project:\nsrc/\nAGENTS.md\nCLAUDE.md\nREADME.md\n"),
            "{snapshot}"
        );
        assert!(snapshot.contains("<project_instructions file=\"AGENTS.md\">\nRun `make test` after changes.\n</project_instructions>"));
        assert!(
            !snapshot.contains("Use tabs."),
            "AGENTS.md wins over CLAUDE.md"
        );

        let untrusted = super::snapshot(&workspace, "2026-11-02", false);
        assert!(!untrusted.contains("project_instructions"));
        assert!(system_prompt(&snapshot).starts_with(RULES));
    }
}
