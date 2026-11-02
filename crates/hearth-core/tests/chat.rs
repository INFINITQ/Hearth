//! Chat runs end to end against the mock model server: what streams, what is saved, and how
//! regenerate, edit, stop and errors behave.

use std::sync::Arc;
use std::time::Duration;

use hearth_core::chat::{Chat, ChatAction, ChatError, StoreResolver};
use hearth_core::runs::{CollectSink, RunEnd, RunEvent, RunManager};
use hearth_core::settings::SettingsStore;
use hearth_core::store::{ConnectionInput, MessageRole, MessageStatus, Mode, SessionPatch, Store};
use hearth_testkit::{Expect, Fault, MockServer, Reply, Script};
use serde_json::json;

struct Harness {
    _dir: tempfile::TempDir,
    server: MockServer,
    store: Arc<Store>,
    runs: Arc<RunManager>,
    settings: Arc<SettingsStore>,
    chat: Chat,
}

async fn harness(script: Script) -> Harness {
    let server = MockServer::start(script).await;
    let dir = tempfile::tempdir().unwrap();
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
    let settings = Arc::new(SettingsStore::open(dir.path().join("settings.json")));
    settings
        .update(&json!({ "models": { "chat": {
            "kind": "connection", "connectionId": connection.id, "model": "mock-model"
        } } }))
        .unwrap();
    let runs = Arc::new(RunManager::new());
    let chat = Chat::new(
        store.clone(),
        runs.clone(),
        Arc::new(StoreResolver::new(store.clone())),
        settings.clone(),
    );
    Harness {
        _dir: dir,
        server,
        store,
        runs,
        settings,
        chat,
    }
}

async fn wait_finished(sink: &CollectSink) -> RunEnd {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let finished = sink.events().iter().find_map(|event| match event {
            RunEvent::Finished { status } => Some(*status),
            _ => None,
        });
        if let Some(status) = finished {
            return status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the run never finished: {:?}",
            sink.events()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn streamed(events: &[RunEvent]) -> (String, String) {
    let mut reasoning = String::new();
    let mut content = String::new();
    for event in events {
        match event {
            RunEvent::ReasoningDelta { text, .. } => reasoning.push_str(text),
            RunEvent::ContentDelta { text, .. } => content.push_str(text),
            _ => {}
        }
    }
    (reasoning, content)
}

fn send(text: &str) -> ChatAction {
    ChatAction::Send {
        text: text.into(),
        web: None,
    }
}

#[tokio::test]
async fn sending_streams_a_reply_saves_it_and_names_the_chat() {
    let reply = Reply::text("**Paris** is the capital.")
        .with_reasoning("Easy one.")
        .chunked(3)
        .usage(50, 8);
    let h = harness(Script::new().reply(reply)).await;
    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let sink = Arc::new(CollectSink::default());

    let started = h
        .chat
        .start(
            &session.id,
            send("What is the capital of France?"),
            sink.clone(),
        )
        .unwrap();
    assert!(started.user_message_id.is_some());
    assert_eq!(wait_finished(&sink).await, RunEnd::Completed);

    let events = sink.events();
    assert_eq!(
        streamed(&events),
        ("Easy one.".into(), "**Paris** is the capital.".into())
    );
    assert!(events.iter().any(
        |e| matches!(e, RunEvent::TitleChanged { title, .. } if title == "What is the capital of France?")
    ));

    let path = h.store.active_path(&session.id).unwrap();
    assert_eq!(path.len(), 2);
    assert_eq!(path[1].role, MessageRole::Assistant);
    assert_eq!(path[1].content, "**Paris** is the capital.");
    assert_eq!(path[1].reasoning.as_deref(), Some("Easy one."));
    assert_eq!(path[1].status, MessageStatus::Complete);
    assert_eq!(path[1].usage.as_ref().unwrap()["promptTokens"], json!(50));
    assert!(
        h.store.session(&session.id).unwrap().model_ref.is_some(),
        "the chat keeps its model"
    );

    let sent = h.server.last_request().unwrap();
    assert_eq!(sent["messages"][0]["role"], json!("system"));
    assert_eq!(
        sent["messages"][1]["content"],
        json!("What is the capital of France?")
    );
    assert_eq!(sent["cache_prompt"], json!(true));
}

#[tokio::test]
async fn regenerating_makes_a_new_version_without_the_old_reply_in_the_prompt() {
    let script = Script::new()
        .reply(Reply::text("First answer"))
        .expect_then(
            vec![
                Expect::MessageCount {
                    min: Some(2),
                    max: Some(2),
                },
                Expect::LastUserContains("Hi".into()),
            ],
            Reply::text("Second answer"),
        );
    let h = harness(script).await;
    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let sink = Arc::new(CollectSink::default());
    h.chat.start(&session.id, send("Hi"), sink.clone()).unwrap();
    wait_finished(&sink).await;
    let first = h.store.active_path(&session.id).unwrap()[1].id.clone();

    let sink = Arc::new(CollectSink::default());
    let started = h
        .chat
        .start(
            &session.id,
            ChatAction::Regenerate {
                message_id: first.clone(),
            },
            sink.clone(),
        )
        .unwrap();
    assert_eq!(started.user_message_id, None);
    assert_eq!(wait_finished(&sink).await, RunEnd::Completed);

    let path = h.store.active_path(&session.id).unwrap();
    assert_eq!(path[1].content, "Second answer");
    assert_eq!(
        h.store.siblings(&path[1].id).unwrap(),
        [first, path[1].id.clone()]
    );
}

#[tokio::test]
async fn editing_a_message_starts_a_new_branch() {
    let h = harness(
        Script::new()
            .reply(Reply::text("A"))
            .reply(Reply::text("B")),
    )
    .await;
    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let sink = Arc::new(CollectSink::default());
    h.chat
        .start(&session.id, send("hello"), sink.clone())
        .unwrap();
    wait_finished(&sink).await;
    let original = h.store.active_path(&session.id).unwrap()[0].id.clone();

    let sink = Arc::new(CollectSink::default());
    let edit = ChatAction::Edit {
        message_id: original.clone(),
        text: "hello again".into(),
    };
    h.chat.start(&session.id, edit, sink.clone()).unwrap();
    wait_finished(&sink).await;

    let path = h.store.active_path(&session.id).unwrap();
    let texts: Vec<&str> = path.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(texts, ["hello again", "B"]);
    assert_eq!(
        h.store.siblings(&path[0].id).unwrap(),
        [original, path[0].id.clone()]
    );
    assert_eq!(
        h.server.last_request().unwrap()["messages"][1]["content"],
        json!("hello again")
    );
}

#[tokio::test]
async fn stopping_keeps_what_was_written() {
    let reply = Reply::text("Partial answer that keeps going")
        .chunked(4)
        .held("end");
    let h = harness(Script::new().reply(reply)).await;
    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let sink = Arc::new(CollectSink::default());
    let started = h.chat.start(&session.id, send("Go"), sink.clone()).unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !sink
        .events()
        .iter()
        .any(|e| matches!(e, RunEvent::ContentDelta { .. }))
    {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(h.runs.cancel(&started.run_id));
    assert_eq!(wait_finished(&sink).await, RunEnd::Cancelled);

    let reply = &h.store.active_path(&session.id).unwrap()[1];
    assert_eq!(reply.status, MessageStatus::Interrupted);
    assert!(!reply.content.is_empty());
}

#[tokio::test]
async fn server_errors_are_saved_and_reported() {
    let fault = Fault::Http {
        status: 500,
        body: json!({ "error": { "message": "model crashed" } }),
    };
    let h = harness(Script::new().reply(Reply::fault(fault))).await;
    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let sink = Arc::new(CollectSink::default());
    h.chat.start(&session.id, send("Hi"), sink.clone()).unwrap();
    assert_eq!(wait_finished(&sink).await, RunEnd::Failed);

    assert!(sink.events().iter().any(|e| matches!(
        e,
        RunEvent::Error { code, message, retryable: true } if code == "http" && message.contains("model crashed")
    )));
    let reply = &h.store.active_path(&session.id).unwrap()[1];
    assert_eq!(reply.status, MessageStatus::Error);
    assert!(reply.content.is_empty());
}

#[tokio::test]
async fn thinking_can_be_switched_off_per_chat() {
    let script = Script::new().expect_then(
        vec![Expect::Param(
            "chat_template_kwargs.enable_thinking".into(),
            json!(false),
        )],
        Reply::text("ok"),
    );
    let h = harness(script).await;
    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let patch = SessionPatch {
        params: Some(json!({ "thinking": false, "temperature": 0.3 })),
        ..Default::default()
    };
    h.store.update_session(&session.id, &patch).unwrap();
    let sink = Arc::new(CollectSink::default());
    h.chat.start(&session.id, send("Hi"), sink.clone()).unwrap();
    assert_eq!(
        wait_finished(&sink).await,
        RunEnd::Completed,
        "{:?}",
        sink.events()
    );
    let sent = h.server.last_request().unwrap();
    assert!((sent["temperature"].as_f64().unwrap() - 0.3).abs() < 1e-6);
}

#[tokio::test]
async fn without_a_model_nothing_starts() {
    let h = harness(Script::new()).await;
    h.settings
        .update(&json!({ "models": { "chat": null } }))
        .unwrap();
    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let err = h
        .chat
        .start(&session.id, send("Hi"), Arc::new(CollectSink::default()))
        .unwrap_err();
    assert!(matches!(err, ChatError::NoModel));
    assert!(h.store.active_path(&session.id).unwrap().is_empty());
}

#[tokio::test]
async fn one_reply_at_a_time_per_chat() {
    let h = harness(Script::new().reply(Reply::text("slow").held("h"))).await;
    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let sink = Arc::new(CollectSink::default());
    h.chat
        .start(&session.id, send("one"), sink.clone())
        .unwrap();
    let err = h
        .chat
        .start(&session.id, send("two"), Arc::new(CollectSink::default()))
        .unwrap_err();
    assert!(matches!(err, ChatError::Busy(_)));
    h.server.release("h");
    assert_eq!(wait_finished(&sink).await, RunEnd::Completed);
}

#[tokio::test]
async fn long_chats_are_summarized_to_fit_the_window() {
    let mut script = Script::new()
        .reply(Reply::text("Noted.").usage(1700, 5))
        .reply(Reply::text("SUMMARY: a long list of numbers."))
        .expect_then(
            vec![Expect::SystemContains(
                "SUMMARY: a long list of numbers.".into(),
            )],
            Reply::text("It was a list of numbers."),
        );
    script.n_ctx = 2000;
    let h = harness(script).await;
    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let long: String = (0..1500).map(|n| format!("{n} ")).collect();
    let sink = Arc::new(CollectSink::default());
    h.chat
        .start(
            &session.id,
            ChatAction::Send {
                text: long,
                web: None,
            },
            sink.clone(),
        )
        .unwrap();
    assert_eq!(wait_finished(&sink).await, RunEnd::Completed);

    let sink = Arc::new(CollectSink::default());
    h.chat
        .start(
            &session.id,
            ChatAction::Send {
                text: "What did I send?".into(),
                web: None,
            },
            sink.clone(),
        )
        .unwrap();
    assert_eq!(
        wait_finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
    let path = h.store.active_path(&session.id).unwrap();
    let roles: Vec<MessageRole> = path.iter().map(|m| m.role).collect();
    use MessageRole::{Assistant, Summary, User};
    assert_eq!(roles, [User, Assistant, User, Summary, Assistant]);
    assert_eq!(path[4].content, "It was a list of numbers.");
    assert!(
        sink.events()
            .iter()
            .any(|e| matches!(e, RunEvent::Context { limit: 2000, .. }))
    );
}

const NEWS: &str = "<!doctype html><html><head><title>Hearth 1.0</title></head><body>\
<article><h1>Hearth 1.0</h1><p>Hearth 1.0 ships on 14 November with web search, terminals and \
MCP servers. It runs local models with llama.cpp and keeps everything on your PC.</p>\
<p>The release also brings a context meter and summaries of long conversations, so chats can \
go on for as long as you like.</p></article></body></html>";

fn web_settings(h: &Harness, allow_local: bool) {
    h.settings
        .update(&json!({ "web": {
            "provider": "searxng", "searxngUrl": h.server.url, "allowLocal": allow_local
        } }))
        .unwrap();
}

#[tokio::test]
async fn web_search_in_a_chat_searches_reads_and_then_answers() {
    let h = harness(Script::new()).await;
    let page = format!("{}/pages/news", h.server.url);
    let mut script = Script::new()
        .expect_then(
            vec![
                Expect::HasTool("web_search".into()),
                Expect::NoTool("read_file".into()),
                Expect::SystemContains("Cite the pages you used".into()),
            ],
            Reply::tool_call("web_search", json!({ "query": "hearth 1.0 release" })),
        )
        .expect_then(
            vec![Expect::LastToolResultContains("Hearth 1.0 is out".into())],
            Reply::tool_call("web_fetch", json!({ "url": page })),
        )
        .expect_then(
            vec![Expect::LastToolResultContains(
                "ships on 14 November".into(),
            )],
            Reply::text("Hearth 1.0 ships on 14 November."),
        );
    script.search = vec![json!({ "title": "Hearth 1.0 is out", "url": page, "content": "News." })];
    script.pages.insert("news".into(), NEWS.into());
    h.server.set_script(script);
    web_settings(&h, true);

    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let sink = Arc::new(CollectSink::default());
    h.chat
        .start(
            &session.id,
            ChatAction::Send {
                text: "When does Hearth 1.0 ship?".into(),
                web: Some(true),
            },
            sink.clone(),
        )
        .unwrap();
    assert_eq!(
        wait_finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );

    use MessageRole::{Assistant, Tool, User};
    let path = h.store.active_path(&session.id).unwrap();
    let roles: Vec<MessageRole> = path.iter().map(|m| m.role).collect();
    assert_eq!(roles, [User, Assistant, Tool, Assistant, Tool, Assistant]);
    assert_eq!(
        path[2].tool_display.as_ref().unwrap()["results"][0]["url"],
        json!(page)
    );
    assert_eq!(
        path[4].tool_display.as_ref().unwrap()["pageTitle"],
        json!("Hearth 1.0")
    );
    assert_eq!(path[5].content, "Hearth 1.0 ships on 14 November.");
    assert!(sink.events().iter().any(|e| matches!(
        e,
        RunEvent::ToolStarted { title, .. } if title == "Search the web for “hearth 1.0 release”"
    )));
    // Web search stays on for this chat.
    let saved = h.store.session(&session.id).unwrap();
    assert_eq!(saved.search_mode.as_deref(), Some("web"));
}

#[tokio::test]
async fn chats_dont_read_pages_on_this_pc_unless_allowed() {
    let h = harness(Script::new()).await;
    let page = format!("{}/pages/news", h.server.url);
    let mut script = Script::new()
        .reply(Reply::tool_call("web_fetch", json!({ "url": page })))
        .expect_then(
            vec![Expect::LastToolResultContains("local network".into())],
            Reply::text("I couldn't read that page."),
        );
    script.pages.insert("news".into(), NEWS.into());
    h.server.set_script(script);
    web_settings(&h, false);
    let session = h.store.create_session(Mode::Chat, None).unwrap();
    let sink = Arc::new(CollectSink::default());
    h.chat
        .start(
            &session.id,
            ChatAction::Send {
                text: "Read my router's page".into(),
                web: Some(true),
            },
            sink.clone(),
        )
        .unwrap();
    assert_eq!(
        wait_finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
    let path = h.store.active_path(&session.id).unwrap();
    assert_eq!(path[2].tool_status.as_deref(), Some("error"));
    assert_eq!(path.last().unwrap().content, "I couldn't read that page.");
}
