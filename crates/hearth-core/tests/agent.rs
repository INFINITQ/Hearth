//! The coding agent end to end against the mock model server, in a temporary project: what it
//! sends, what it changes on disk, when it asks, and how it stops.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hearth_core::agent::{Agent, Decision, PermissionReply};
use hearth_core::chat::StoreResolver;
use hearth_core::runs::{CollectSink, RunEnd, RunEvent, RunManager};
use hearth_core::settings::SettingsStore;
use hearth_core::store::{ConnectionInput, MessageRole, Store};
use hearth_testkit::{Expect, MockServer, Reply, Script};
use serde_json::json;

const CALC: &str = "def add(a, b):\n    return a - b\n";

struct Harness {
    _settings: tempfile::TempDir,
    project: tempfile::TempDir,
    server: MockServer,
    store: Arc<Store>,
    runs: Arc<RunManager>,
    agent: Agent,
    session_id: String,
}

impl Harness {
    fn file(&self, path: &str) -> String {
        std::fs::read_to_string(self.project.path().join(path))
            .unwrap()
            .replace("\r\n", "\n")
    }

    fn start(&self, text: &str) -> (Arc<CollectSink>, String) {
        let sink = Arc::new(CollectSink::default());
        let started = self
            .agent
            .start(&self.session_id, text, sink.clone())
            .unwrap();
        (sink, started.run_id)
    }

    fn roles(&self) -> Vec<MessageRole> {
        self.store
            .active_path(&self.session_id)
            .unwrap()
            .iter()
            .map(|m| m.role)
            .collect()
    }
}

async fn harness(script: Script, mode: Option<&str>) -> Harness {
    let server = MockServer::start(script).await;
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/calc.py"), CALC).unwrap();
    std::fs::write(
        project.path().join("AGENTS.md"),
        "Run the tests with pytest.\n",
    )
    .unwrap();

    let settings_dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let connection = store
        .save_connection(&ConnectionInput {
            id: None,
            name: "Mock".into(),
            base_url: server.url.clone(),
            api_key: None,
            llama_extensions: true,
        })
        .unwrap();
    let settings = Arc::new(SettingsStore::open(
        settings_dir.path().join("settings.json"),
    ));
    settings
        .update(&json!({ "models": { "code": {
            "kind": "connection", "connectionId": connection.id, "model": "mock-model"
        } } }))
        .unwrap();
    let root: PathBuf = dunce::canonicalize(project.path()).unwrap();
    let saved = store.open_project(&root.display().to_string()).unwrap();
    store.trust_project(&saved.id).unwrap();
    let session = store.create_code_session(&saved.id, mode).unwrap();
    let runs = Arc::new(RunManager::new());
    let agent = Agent::new(
        store.clone(),
        runs.clone(),
        Arc::new(StoreResolver::new(store.clone())),
        settings,
    );
    Harness {
        _settings: settings_dir,
        project,
        server,
        store,
        runs,
        agent,
        session_id: session.id,
    }
}

/// Waits until an event matches, and returns it.
async fn wait_for(sink: &CollectSink, what: &str, matches: impl Fn(&RunEvent) -> bool) -> RunEvent {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(event) = sink.events().into_iter().find(|e| matches(e)) {
            return event;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no {what} came: {:#?}",
            sink.events()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn finished(sink: &CollectSink) -> RunEnd {
    match wait_for(sink, "finish", |e| matches!(e, RunEvent::Finished { .. })).await {
        RunEvent::Finished { status } => status,
        _ => unreachable!(),
    }
}

fn read_calc() -> Reply {
    Reply::tool_call("read_file", json!({ "path": "src/calc.py" }))
}

fn fix_calc() -> Reply {
    Reply::tool_call(
        "edit_file",
        json!({ "path": "src/calc.py", "old_string": "return a - b", "new_string": "return a + b" }),
    )
}

fn titles(events: &[RunEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            RunEvent::ToolStarted { title, .. } => Some(title.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn reads_edits_and_finishes() {
    let script = Script::new()
        .expect_then(
            vec![
                Expect::SystemContains("Run the tests with pytest.".into()),
                Expect::HasTool("edit_file".into()),
            ],
            read_calc(),
        )
        .expect_then(
            vec![Expect::LastToolResultContains(
                "     2→    return a - b".into(),
            )],
            fix_calc(),
        )
        .expect_then(
            vec![Expect::LastToolResultContains("Edited src/calc.py".into())],
            Reply::text("Fixed: add now adds."),
        );
    let h = harness(script, None).await;
    let (sink, _) = h.start("add() subtracts. Fix it.");
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );

    assert_eq!(h.file("src/calc.py"), "def add(a, b):\n    return a + b\n");
    let events = sink.events();
    assert_eq!(titles(&events), ["Read src/calc.py", "Edit src/calc.py"]);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, RunEvent::PermissionRequest { .. })),
        "Auto-edit doesn't ask"
    );
    use MessageRole::{Assistant, Tool, User};
    assert_eq!(
        h.roles(),
        [User, Assistant, Tool, Assistant, Tool, Assistant]
    );
    let path = h.store.active_path(&h.session_id).unwrap();
    let edit = &path[4];
    assert_eq!(edit.tool_status.as_deref(), Some("ok"));
    let diff = edit.tool_display.as_ref().unwrap()["diff"]
        .as_str()
        .unwrap();
    assert!(diff.contains("+    return a + b"), "{diff}");
    assert_eq!(path[5].content, "Fixed: add now adds.");
    // The mock calls every first tool call `call_0`; Hearth gives each its own id.
    let first_id = path[1].tool_calls.as_ref().unwrap()[0]["id"].clone();
    assert_ne!(first_id, path[3].tool_calls.as_ref().unwrap()[0]["id"]);
    assert_eq!(path[2].tool_call_id.as_deref(), first_id.as_str());

    // The same environment goes out with every request, so the server can reuse its cache.
    let requests = h.server.requests();
    let systems: Vec<&serde_json::Value> = requests
        .iter()
        .map(|r| &r["messages"][0]["content"])
        .collect();
    assert!(systems.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(
        requests[1]["messages"][2]["tool_calls"][0]["function"]["name"],
        "read_file"
    );
}

#[tokio::test]
async fn ask_mode_waits_for_permission_and_a_denial_reaches_the_model() {
    let script = Script::new()
        .reply(read_calc())
        .reply(fix_calc())
        .expect_then(
            vec![Expect::LastToolResultContains(
                "said: Use a constant instead".into(),
            )],
            Reply::text("Understood."),
        );
    let h = harness(script, Some("ask")).await;
    let (sink, _) = h.start("Fix add().");
    let request = wait_for(&sink, "permission request", |e| {
        matches!(e, RunEvent::PermissionRequest { .. })
    })
    .await;
    let RunEvent::PermissionRequest {
        request_id,
        title,
        diff,
        choices,
        ..
    } = request
    else {
        unreachable!()
    };
    assert_eq!(title, "Edit src/calc.py");
    assert!(
        diff.unwrap()
            .contains("-    return a - b\n+    return a + b")
    );
    assert_eq!(
        choices.iter().map(|c| c.id).collect::<Vec<_>>(),
        ["allow_once", "allow_session", "deny"]
    );
    assert_eq!(
        h.file("src/calc.py"),
        CALC,
        "nothing changes before the answer"
    );

    let reply = PermissionReply {
        decision: Decision::Deny,
        note: Some("Use a constant instead".into()),
    };
    assert!(h.agent.resolve_permission(&request_id, reply.clone()));
    assert!(
        !h.agent.resolve_permission(&request_id, reply),
        "answered once"
    );
    assert_eq!(finished(&sink).await, RunEnd::Completed);
    assert_eq!(h.file("src/calc.py"), CALC);
    let path = h.store.active_path(&h.session_id).unwrap();
    assert_eq!(path[4].tool_status.as_deref(), Some("denied"));
}

#[tokio::test]
async fn allowing_for_the_session_stops_further_prompts() {
    let rename = Reply::tool_call(
        "edit_file",
        json!({ "path": "src/calc.py", "old_string": "def add", "new_string": "def plus" }),
    );
    let script = Script::new()
        .reply(read_calc())
        .reply(fix_calc())
        .reply(rename)
        .reply(Reply::text("Done."));
    let h = harness(script, Some("ask")).await;
    let (sink, _) = h.start("Fix and rename add().");
    let RunEvent::PermissionRequest { request_id, .. } =
        wait_for(&sink, "permission request", |e| {
            matches!(e, RunEvent::PermissionRequest { .. })
        })
        .await
    else {
        unreachable!()
    };
    h.agent.resolve_permission(
        &request_id,
        PermissionReply {
            decision: Decision::AllowSession,
            note: None,
        },
    );
    assert_eq!(finished(&sink).await, RunEnd::Completed);
    let prompts = sink
        .events()
        .iter()
        .filter(|e| matches!(e, RunEvent::PermissionRequest { .. }))
        .count();
    assert_eq!(prompts, 1);
    assert_eq!(h.file("src/calc.py"), "def plus(a, b):\n    return a + b\n");
}

#[tokio::test]
async fn plan_mode_refuses_changes_without_asking() {
    let script = Script::new()
        .expect_then(
            vec![Expect::LastUserContains("Plan mode is on".into())],
            read_calc(),
        )
        .reply(fix_calc())
        .expect_then(
            vec![Expect::LastToolResultContains(
                "Plan mode is read-only".into(),
            )],
            Reply::text("Plan: change - to +."),
        );
    let h = harness(script, Some("plan")).await;
    let (sink, _) = h.start("How would you fix add()?");
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
    assert!(
        !sink
            .events()
            .iter()
            .any(|e| matches!(e, RunEvent::PermissionRequest { .. }))
    );
    assert_eq!(h.file("src/calc.py"), CALC);
}

#[tokio::test]
async fn stopping_while_waiting_still_answers_every_call() {
    let both = fix_calc().and_tool_call("glob", json!({ "pattern": "*.py" }));
    let script = Script::new().reply(read_calc()).reply(both).expect_then(
        vec![Expect::LastUserContains("Try again".into())],
        Reply::text("Retrying."),
    );
    let h = harness(script, Some("ask")).await;
    let (sink, run_id) = h.start("Fix add().");
    wait_for(&sink, "permission request", |e| {
        matches!(e, RunEvent::PermissionRequest { .. })
    })
    .await;
    assert!(h.runs.cancel(&run_id));
    assert_eq!(finished(&sink).await, RunEnd::Cancelled);
    let path = h.store.active_path(&h.session_id).unwrap();
    let answers: Vec<Option<&str>> = path[4..].iter().map(|m| m.tool_status.as_deref()).collect();
    assert_eq!(answers, [Some("cancelled"), Some("cancelled")]);

    // The conversation is still valid: the next task goes through.
    let (sink, _) = h.start("Try again");
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
}

#[tokio::test]
async fn questions_wait_for_the_answer() {
    let ask = Reply::tool_call(
        "ask_user",
        json!({ "question": "Tabs or spaces?", "options": ["Tabs", "Spaces"] }),
    );
    let script = Script::new().reply(ask).expect_then(
        vec![Expect::LastToolResultContains(
            "The user answered: Spaces".into(),
        )],
        Reply::text("Spaces it is."),
    );
    let h = harness(script, None).await;
    let (sink, _) = h.start("Set up formatting.");
    let RunEvent::Question {
        question_id,
        question,
        options,
        ..
    } = wait_for(&sink, "question", |e| {
        matches!(e, RunEvent::Question { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!((question.as_str(), options.len()), ("Tabs or spaces?", 2));
    assert!(h.agent.answer(&question_id, "Spaces"));
    assert_eq!(finished(&sink).await, RunEnd::Completed);
}

#[tokio::test]
async fn tool_calls_written_as_text_still_run() {
    let text = "I'll read it first.\n<tool_call>\n{\"name\": \"read_file\", \"arguments\": {\"path\": \"src/calc.py\"}}\n</tool_call>";
    let script = Script::new().reply(Reply::text(text)).expect_then(
        vec![Expect::LastToolResultContains("return a - b".into())],
        Reply::text("It subtracts."),
    );
    let h = harness(script, None).await;
    let (sink, _) = h.start("What does add() do?");
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
    let path = h.store.active_path(&h.session_id).unwrap();
    assert_eq!(path[1].content.trim(), "I'll read it first.");
    assert!(sink.events().iter().any(|e| matches!(e, RunEvent::MessageFinished { content: Some(c), .. } if c.trim() == "I'll read it first.")));
}

#[tokio::test]
async fn the_same_call_over_and_over_pauses_the_run() {
    let mut script = Script::new();
    for _ in 0..6 {
        script = script.reply(Reply::tool_call("glob", json!({ "pattern": "*.py" })));
    }
    let h = harness(script, None).await;
    let (sink, _) = h.start("Find the Python files.");
    assert_eq!(finished(&sink).await, RunEnd::Paused);
    let path = h.store.active_path(&h.session_id).unwrap();
    let results: Vec<&str> = path
        .iter()
        .filter(|m| m.role == MessageRole::Tool)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(results.len(), 5);
    assert!(
        results[2].contains("made this exact call 3 times"),
        "{}",
        results[2]
    );
    assert!(
        sink.events()
            .iter()
            .any(|e| matches!(e, RunEvent::Notice { text, .. } if text.contains("paused")))
    );
}

#[tokio::test]
async fn unreadable_arguments_get_an_explanation() {
    let mut broken = Reply::tool_call("read_file", json!({}));
    broken.tool_calls[0].raw_arguments = Some("path is src/calc.py".into());
    let script = Script::new().reply(broken).expect_then(
        vec![Expect::LastToolResultContains("couldn't be read".into())],
        Reply::text("Sorry."),
    );
    let h = harness(script, None).await;
    let (sink, _) = h.start("Read calc.");
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
}

// ── M4: commands, undo, steering ──────────────────────────────────────────────────────────────

fn has_bash() -> bool {
    use hearth_core::agent::shell::{Shell, ShellKind};
    let found = Shell::find(ShellKind::GitBash).is_some();
    if !found {
        eprintln!("Git Bash isn't installed here; skipped");
    }
    found
}

fn run_command(command: &str) -> Reply {
    Reply::tool_call("run_command", json!({ "command": command }))
}

#[tokio::test]
async fn commands_run_and_their_output_streams() {
    if !has_bash() {
        return;
    }
    let script = Script::new()
        .reply(run_command("echo hello-from-bash; exit 2"))
        .expect_then(
            vec![
                Expect::LastToolResultContains("hello-from-bash".into()),
                Expect::LastToolResultContains("(exit code 2,".into()),
                Expect::SystemContains("Shell for run_command: Git Bash".into()),
            ],
            Reply::text("It printed and failed."),
        );
    let h = harness(script, Some("full_auto")).await;
    let (sink, _) = h.start("Run it.");
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
    let events = sink.events();
    assert!(events.iter().any(
        |e| matches!(e, RunEvent::ToolOutput { text, .. } if text.contains("hello-from-bash"))
    ));
    let path = h.store.active_path(&h.session_id).unwrap();
    assert_eq!(
        path[2].tool_status.as_deref(),
        Some("error"),
        "a non-zero exit shows as failed"
    );
    assert_eq!(path[2].tool_display.as_ref().unwrap()["exitCode"], 2);
}

#[tokio::test]
async fn ask_mode_asks_before_commands_and_always_allow_is_remembered() {
    if !has_bash() {
        return;
    }
    let script = Script::new()
        .reply(run_command("echo one"))
        .reply(run_command("echo two"))
        .reply(Reply::text("Both ran."));
    let h = harness(script, Some("ask")).await;
    let (sink, _) = h.start("Echo twice.");
    let RunEvent::PermissionRequest {
        request_id,
        command,
        choices,
        ..
    } = wait_for(&sink, "permission request", |e| {
        matches!(e, RunEvent::PermissionRequest { .. })
    })
    .await
    else {
        unreachable!()
    };
    assert_eq!(command.as_deref(), Some("echo one"));
    assert_eq!(choices[1].label, "Always allow `echo` here");
    h.agent.resolve_permission(
        &request_id,
        PermissionReply {
            decision: Decision::AllowAlways,
            note: None,
        },
    );
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
    let prompts = sink
        .events()
        .iter()
        .filter(|e| matches!(e, RunEvent::PermissionRequest { .. }))
        .count();
    assert_eq!(prompts, 1, "the second echo is allowed by the rule");
    let project = h
        .store
        .session(&h.session_id)
        .unwrap()
        .workspace_id
        .unwrap();
    assert_eq!(h.store.rules(&project, "execute").unwrap(), ["echo"]);
}

#[tokio::test]
async fn plan_mode_runs_only_read_only_commands() {
    if !has_bash() {
        return;
    }
    let script = Script::new()
        .reply(run_command("ls src"))
        .expect_then(
            vec![Expect::LastToolResultContains("calc.py".into())],
            run_command("touch made.txt"),
        )
        .expect_then(
            vec![Expect::LastToolResultContains(
                "only runs read-only commands".into(),
            )],
            Reply::text("Plan: …"),
        );
    let h = harness(script, Some("plan")).await;
    let (sink, _) = h.start("Look around.");
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
    assert!(!h.project.path().join("made.txt").exists());
}

#[tokio::test]
async fn stopping_a_command_ends_it_and_the_run() {
    if !has_bash() {
        return;
    }
    let h = harness(
        Script::new().reply(run_command("sleep 30")),
        Some("full_auto"),
    )
    .await;
    let (sink, run_id) = h.start("Wait a while.");
    wait_for(&sink, "the command to start", |e| {
        matches!(e, RunEvent::ToolStarted { .. })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let stopped_at = tokio::time::Instant::now();
    h.runs.cancel(&run_id);
    assert_eq!(finished(&sink).await, RunEnd::Cancelled);
    assert!(
        stopped_at.elapsed() < Duration::from_secs(10),
        "{:?}",
        stopped_at.elapsed()
    );
    let path = h.store.active_path(&h.session_id).unwrap();
    assert_eq!(path[2].tool_status.as_deref(), Some("cancelled"));
}

#[tokio::test]
async fn undoing_a_task_restores_its_files_and_tells_the_model() {
    let script = Script::new()
        .reply(read_calc())
        .reply(fix_calc())
        .reply(Reply::tool_call(
            "write_file",
            json!({ "path": "notes.md", "content": "# Notes\n" }),
        ))
        .reply(Reply::text("Fixed and noted."))
        .expect_then(
            vec![Expect::LastUserContains(
                "the user undid file changes".into(),
            )],
            Reply::text("I'll re-read."),
        );
    let h = harness(script, None).await;
    let (sink, _) = h.start("Fix add() and take notes.");
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
    assert_eq!(h.file("src/calc.py"), "def add(a, b):\n    return a + b\n");

    let changes = h.agent.changes(&h.session_id).unwrap();
    let listed: Vec<(&str, usize, usize)> = changes
        .iter()
        .map(|c| (c.path.as_str(), c.added, c.removed))
        .collect();
    assert_eq!(listed, [("src/calc.py", 1, 1), ("notes.md", 1, 0)]);

    let restored = h.agent.undo(&h.session_id, 1).unwrap();
    assert_eq!(restored, ["src/calc.py", "notes.md"]);
    assert_eq!(h.file("src/calc.py"), CALC);
    assert!(!h.project.path().join("notes.md").exists());
    assert!(h.agent.changes(&h.session_id).unwrap().is_empty());

    let (sink, _) = h.start("Now what?");
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
}

#[tokio::test]
async fn messages_sent_while_working_join_the_next_step() {
    let script = Script::new()
        .reply(Reply::tool_call("glob", json!({ "pattern": "*.py" })).held("first"))
        .expect_then(
            vec![Expect::LastUserContains("Also look at the README".into())],
            Reply::text("Looked at both."),
        );
    let h = harness(script, None).await;
    let (sink, run_id) = h.start("Find the Python files.");
    wait_for(&sink, "the first step", |e| {
        matches!(e, RunEvent::MessageStarted { .. })
    })
    .await;
    h.agent.steer(&run_id, "Also look at the README").unwrap();
    h.server.release("first");
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
    use MessageRole::{Assistant, Tool, User};
    assert_eq!(h.roles(), [User, Assistant, Tool, User, Assistant]);
    assert!(sink.events().iter().any(|e| matches!(e, RunEvent::Steered { message } if message.content == "Also look at the README")));
    assert!(h.agent.steer(&run_id, "too late").is_err());
}
