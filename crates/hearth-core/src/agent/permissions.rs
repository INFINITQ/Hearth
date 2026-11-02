//! When the agent may use a tool without asking.
//!
//! Each call is checked against the session's mode and what the user already allowed for the
//! session ("Allow for the session"). Reading inside the project never asks. Changes, and
//! anything outside the project, depend on the mode:
//!
//! | Mode      | read outside | edit inside | edit outside | commands      | MCP   | web  |
//! |-----------|--------------|-------------|--------------|---------------|-------|------|
//! | Plan      | ask          | deny        | deny         | read-only     | deny  | ask  |
//! | Ask       | ask          | ask         | ask          | ask           | ask   | ask  |
//! | Auto-edit | ask          | allow       | ask          | read-only, rules | ask | ask  |
//! | Full auto | allow        | allow       | allow        | allow         | allow | allow|
//!
//! Commands: "read-only" means a plain command from a short list (`ls`, `git status`…), and
//! "rules" are the project's "Always allow" prefixes (`npm test`). Dangerous commands (wiping a
//! drive, force-pushing…) always ask, whatever the mode or rules.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::tools::{Class, Prepared};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Look around and propose a plan; nothing changes.
    Plan,
    /// Ask before every change.
    Ask,
    /// Edits inside the project apply at once; everything else asks.
    #[default]
    AutoEdit,
    /// No questions inside the project.
    FullAuto,
}

impl PermissionMode {
    /// A session's saved mode (`None` or anything unknown is the default, Auto-edit).
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("plan") => Self::Plan,
            Some("ask") => Self::Ask,
            Some("full_auto") => Self::FullAuto,
            _ => Self::AutoEdit,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Ask => "ask",
            Self::AutoEdit => "auto_edit",
            Self::FullAuto => "full_auto",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Ask,
    /// Not allowed in this mode; the text tells the model why.
    Deny(String),
}

pub const PLAN_DENIAL: &str = "Plan mode is read-only, so this wasn't done. Finish looking around, \
then reply with your plan; the user switches modes when they want the changes made.";

/// What the user allowed for the rest of a session.
#[derive(Debug, Default)]
pub struct Grants {
    /// Classes allowed everywhere inside the project.
    project: HashSet<Class>,
    /// Classes allowed in a folder outside it (lowercase, ending in `/`), and below.
    folders: HashSet<(Class, String)>,
}

/// The folder a call outside the project touches: a file's folder, or the folder itself.
fn scope_folder(prepared: &Prepared) -> Option<String> {
    let path = prepared.path.as_deref()?;
    let folder = match prepared.tool {
        "read_file" | "write_file" | "edit_file" => {
            path.rsplit_once('/').map_or(path, |(dir, _)| dir)
        }
        _ => path,
    };
    Some(format!("{}/", folder.trim_end_matches('/').to_lowercase()))
}

impl Grants {
    pub fn covers(&self, prepared: &Prepared) -> bool {
        if prepared.inside {
            return self.project.contains(&prepared.class);
        }
        let Some(folder) = scope_folder(prepared) else {
            return false;
        };
        self.folders.iter().any(|(class, granted)| {
            *class == prepared.class && folder.starts_with(granted.as_str())
        })
    }

    pub fn grant(&mut self, prepared: &Prepared) {
        if prepared.inside {
            self.project.insert(prepared.class);
        } else if let Some(folder) = scope_folder(prepared) {
            self.folders.insert((prepared.class, folder));
        }
    }
}

pub const PLAN_COMMAND_DENIAL: &str = "Plan mode only runs read-only commands (like ls, git status \
or rg), so this one wasn't run. Finish looking around, then reply with your plan.";

/// `rules`: the project's "Always allow" command prefixes.
pub fn decide(
    mode: PermissionMode,
    prepared: &Prepared,
    grants: &Grants,
    rules: &[String],
) -> Verdict {
    use PermissionMode::{AutoEdit, FullAuto, Plan};
    let granted = grants.covers(prepared);
    match prepared.class {
        Class::Internal => Verdict::Allow,
        Class::Read if prepared.inside || mode == FullAuto || granted => Verdict::Allow,
        Class::Read => Verdict::Ask,
        Class::Edit => match mode {
            Plan => Verdict::Deny(PLAN_DENIAL.into()),
            FullAuto => Verdict::Allow,
            AutoEdit if prepared.inside => Verdict::Allow,
            _ if granted => Verdict::Allow,
            _ => Verdict::Ask,
        },
        Class::Execute => {
            let Some(check) = &prepared.command else {
                return Verdict::Ask;
            };
            let command = match &prepared.action {
                super::tools::Action::Run { command, .. } => command.as_str(),
                _ => "",
            };
            let ruled = rules
                .iter()
                .any(|rule| super::command::rule_covers(rule, command));
            match mode {
                _ if check.danger.is_some() => Verdict::Ask,
                Plan if check.read_only => Verdict::Allow,
                Plan => Verdict::Deny(PLAN_COMMAND_DENIAL.into()),
                FullAuto => Verdict::Allow,
                AutoEdit if check.read_only || ruled => Verdict::Allow,
                _ if ruled || granted => Verdict::Allow,
                _ => Verdict::Ask,
            }
        }
        Class::External => {
            // "Always allow" for an MCP tool is saved as its name.
            let ruled = match &prepared.action {
                super::tools::Action::Mcp { name, .. } => rules.iter().any(|rule| rule == name),
                _ => false,
            };
            match mode {
                Plan => Verdict::Deny(PLAN_DENIAL.into()),
                FullAuto => Verdict::Allow,
                _ if granted || ruled => Verdict::Allow,
                _ => Verdict::Ask,
            }
        }
        Class::Network => match mode {
            FullAuto => Verdict::Allow,
            _ if granted => Verdict::Allow,
            _ => Verdict::Ask,
        },
    }
}

/// A button on a permission prompt. `id` is what the UI sends back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Choice {
    pub id: &'static str,
    pub label: String,
}

/// The buttons a prompt offers for a call. Commands get "Always allow" for their prefix in the
/// project, unless they're dangerous (those ask every time).
pub fn choices(prepared: &Prepared) -> Vec<Choice> {
    let middle = match (prepared.class, prepared.inside, &prepared.command) {
        (Class::Execute, _, Some(check)) if check.danger.is_none() && !check.prefix.is_empty() => {
            Some(Choice {
                id: "allow_always",
                label: format!("Always allow `{}` here", check.prefix),
            })
        }
        (Class::Execute, _, _) => None,
        (Class::External, _, _) => Some(Choice {
            id: "allow_always",
            label: "Always allow this tool here".into(),
        }),
        (Class::Edit, true, _) => Some(Choice {
            id: "allow_session",
            label: "Allow edits this session".into(),
        }),
        (_, false, _) => Some(Choice {
            id: "allow_session",
            label: "Allow this folder for the session".into(),
        }),
        _ => Some(Choice {
            id: "allow_session",
            label: "Allow for the session".into(),
        }),
    };
    let mut choices = vec![Choice {
        id: "allow_once",
        label: "Allow".into(),
    }];
    choices.extend(middle);
    choices.push(Choice {
        id: "deny",
        label: "Deny".into(),
    });
    choices
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::agent::tools::Action;

    fn call(tool: &'static str, class: Class, path: &str, inside: bool) -> Prepared {
        Prepared {
            tool,
            class,
            title: format!("{tool} {path}"),
            path: Some(path.into()),
            inside,
            diff: None,
            command: None,
            action: Action::ReadFile {
                abs: PathBuf::from(path),
                display: path.into(),
                offset: 1,
                limit: 10,
            },
        }
    }

    fn run(command: &str) -> Prepared {
        Prepared {
            tool: "run_command",
            class: Class::Execute,
            title: format!("Run {command}"),
            path: Some(".".into()),
            inside: true,
            diff: None,
            command: Some(crate::agent::command::check(command)),
            action: Action::Run {
                command: command.into(),
                cwd: PathBuf::from("."),
                timeout_s: 120,
                background: false,
            },
        }
    }

    /// Without any "Always allow" rules.
    fn decide(mode: PermissionMode, prepared: &Prepared, grants: &Grants) -> Verdict {
        super::decide(mode, prepared, grants, &[])
    }

    #[test]
    fn commands_by_mode_rule_and_danger() {
        use PermissionMode::*;
        let none = Grants::default();
        let rules = vec!["npm test".to_owned()];
        let (status, test, push) = (run("git status"), run("npm test"), run("git push --force"));
        let verdicts = |mode| {
            (
                super::decide(mode, &status, &none, &rules),
                super::decide(mode, &test, &none, &[]),
                super::decide(mode, &test, &none, &rules),
                super::decide(mode, &push, &none, &rules),
            )
        };
        let deny = Verdict::Deny(PLAN_COMMAND_DENIAL.into());
        assert_eq!(
            verdicts(Plan),
            (Verdict::Allow, deny.clone(), deny, Verdict::Ask)
        );
        assert_eq!(
            verdicts(Ask),
            (Verdict::Ask, Verdict::Ask, Verdict::Allow, Verdict::Ask)
        );
        assert_eq!(
            verdicts(AutoEdit),
            (Verdict::Allow, Verdict::Ask, Verdict::Allow, Verdict::Ask)
        );
        assert_eq!(
            verdicts(FullAuto),
            (Verdict::Allow, Verdict::Allow, Verdict::Allow, Verdict::Ask)
        );

        let ids = |p: &Prepared| choices(p).iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(&test), ["allow_once", "allow_always", "deny"]);
        assert_eq!(choices(&test)[1].label, "Always allow `npm test` here");
        assert_eq!(
            ids(&push),
            ["allow_once", "deny"],
            "dangerous commands can't be always-allowed"
        );
    }

    #[test]
    fn the_matrix() {
        use PermissionMode::*;
        let none = Grants::default();
        let read_in = call("read_file", Class::Read, "src/a.rs", true);
        let read_out = call("read_file", Class::Read, "C:/notes/a.txt", false);
        let edit_in = call("edit_file", Class::Edit, "src/a.rs", true);
        let edit_out = call("edit_file", Class::Edit, "C:/notes/a.txt", false);
        let todo = call("todo_write", Class::Internal, "", true);

        for mode in [Plan, Ask, AutoEdit, FullAuto] {
            assert_eq!(decide(mode, &read_in, &none), Verdict::Allow, "{mode:?}");
            assert_eq!(decide(mode, &todo, &none), Verdict::Allow, "{mode:?}");
        }
        let expect = [
            (
                Plan,
                Verdict::Ask,
                Verdict::Deny(PLAN_DENIAL.into()),
                Verdict::Deny(PLAN_DENIAL.into()),
            ),
            (Ask, Verdict::Ask, Verdict::Ask, Verdict::Ask),
            (AutoEdit, Verdict::Ask, Verdict::Allow, Verdict::Ask),
            (FullAuto, Verdict::Allow, Verdict::Allow, Verdict::Allow),
        ];
        for (mode, read_outside, edit_inside, edit_outside) in expect {
            assert_eq!(
                decide(mode, &read_out, &none),
                read_outside,
                "{mode:?} read outside"
            );
            assert_eq!(
                decide(mode, &edit_in, &none),
                edit_inside,
                "{mode:?} edit inside"
            );
            assert_eq!(
                decide(mode, &edit_out, &none),
                edit_outside,
                "{mode:?} edit outside"
            );
        }
    }

    #[test]
    fn session_grants_cover_the_project_or_a_folder_and_below() {
        let mut grants = Grants::default();
        let edit_in = call("edit_file", Class::Edit, "src/a.rs", true);
        grants.grant(&edit_in);
        assert_eq!(
            decide(
                PermissionMode::Ask,
                &call("write_file", Class::Edit, "b.rs", true),
                &grants
            ),
            Verdict::Allow
        );

        grants.grant(&call("read_file", Class::Read, "C:/Notes/a.txt", false));
        let below = call("list_dir", Class::Read, "C:/notes/2026", false);
        let elsewhere = call("read_file", Class::Read, "C:/notesbook/x.txt", false);
        assert_eq!(decide(PermissionMode::Ask, &below, &grants), Verdict::Allow);
        assert_eq!(
            decide(PermissionMode::Ask, &elsewhere, &grants),
            Verdict::Ask
        );
        // A read grant doesn't allow edits there.
        assert_eq!(
            decide(
                PermissionMode::Ask,
                &call("edit_file", Class::Edit, "C:/notes/a.txt", false),
                &grants
            ),
            Verdict::Ask
        );
        // Plan mode stays read-only whatever was granted.
        assert!(matches!(
            decide(PermissionMode::Plan, &edit_in, &grants),
            Verdict::Deny(_)
        ));
    }

    #[test]
    fn modes_round_trip_and_default_to_auto_edit() {
        for mode in [
            PermissionMode::Plan,
            PermissionMode::Ask,
            PermissionMode::AutoEdit,
            PermissionMode::FullAuto,
        ] {
            assert_eq!(PermissionMode::parse(Some(mode.as_str())), mode);
        }
        assert_eq!(PermissionMode::parse(None), PermissionMode::AutoEdit);
        assert_eq!(
            PermissionMode::parse(Some("yolo")),
            PermissionMode::AutoEdit
        );
    }
}
