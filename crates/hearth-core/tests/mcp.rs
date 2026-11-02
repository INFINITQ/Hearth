//! MCP servers end to end against the testkit's fake server: starting, listing and calling
//! tools, failures, and the HTTP transport.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hearth_core::mcp::{Called, Mcp, ServerConfig, ServerState, Transport};
use hearth_core::store::Store;
use hearth_testkit::{MockServer, Script};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// `target/debug/fake-mcp-server.exe` (built by `cargo build -p hearth-testkit --bins`).
fn fake_server() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let path = exe
        .parent()
        .and_then(|deps| deps.parent())
        .unwrap()
        .join("fake-mcp-server.exe");
    assert!(
        path.is_file(),
        "{} isn't built: run cargo build -p hearth-testkit --bins",
        path.display()
    );
    path
}

fn stdio(name: &str, env: &[(&str, &str)]) -> ServerConfig {
    ServerConfig {
        id: String::new(),
        name: name.into(),
        enabled: true,
        transport: Transport::Stdio {
            command: fake_server().display().to_string(),
            args: Vec::new(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            cwd: None,
        },
    }
}

/// Waits until the only server has started (or failed).
async fn settled(mcp: &Mcp) -> ServerState {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let state = mcp.servers()[0].status.clone();
        if !matches!(state, ServerState::Starting) {
            return state;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the server never started"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_stdio_server_starts_lists_its_tools_and_runs_them() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mcp = Mcp::new(store.clone());
    mcp.save(stdio("Notes", &[])).unwrap();
    let ServerState::Ready { server, tools } = settled(&mcp).await else {
        panic!("{:#?}", mcp.servers());
    };
    assert_eq!(server, "fake-mcp");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        ["mcp__notes__add", "mcp__notes__echo", "mcp__notes__fail"]
    );
    assert_eq!(mcp.tool_defs().len(), 3);
    assert_eq!(mcp.find("mcp__notes__echo").unwrap().1.tool, "echo");

    let cancel = CancellationToken::new();
    let echoed = mcp
        .call("mcp__notes__echo", json!({ "text": "hi" }), &cancel)
        .await
        .unwrap();
    assert_eq!(
        echoed,
        Called {
            text: "echo: hi".into(),
            is_error: false
        }
    );
    let sum = mcp
        .call("mcp__notes__add", json!({ "a": 2, "b": 3 }), &cancel)
        .await
        .unwrap();
    assert_eq!(sum.text, "5");
    let failed = mcp
        .call("mcp__notes__fail", json!({}), &cancel)
        .await
        .unwrap();
    assert!(failed.is_error);
    assert!(
        mcp.call("mcp__notes__nope", json!({}), &cancel)
            .await
            .is_err()
    );
    assert!(mcp.servers()[0].log.iter().any(|l| l == "fake-mcp ready"));

    // Saved: the next start of Hearth has it.
    let again = Mcp::new(store);
    assert_eq!(again.servers()[0].config.name, "Notes");
    // Names stay unique.
    assert!(mcp.save(stdio("notes", &[])).is_err());
}

#[tokio::test]
async fn a_server_that_cant_start_says_why() {
    let mcp = Mcp::new(Arc::new(Store::open_in_memory().unwrap()));
    mcp.save(stdio("Broken", &[("FAKE_MCP_FAIL", "1")]))
        .unwrap();
    let ServerState::Failed { error } = settled(&mcp).await else {
        panic!("{:#?}", mcp.servers());
    };
    assert!(error.contains("the API key is missing"), "{error}");
    assert!(mcp.tools().is_empty());

    let missing = ServerConfig {
        transport: Transport::Stdio {
            command: "no-such-mcp-server".into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
        },
        ..stdio("Missing", &[])
    };
    let mcp = Mcp::new(Arc::new(Store::open_in_memory().unwrap()));
    mcp.save(missing).unwrap();
    let ServerState::Failed { error } = settled(&mcp).await else {
        panic!("{:#?}", mcp.servers());
    };
    assert!(error.contains("wasn't found"), "{error}");
}

#[tokio::test]
async fn a_server_that_dies_mid_call_is_marked_failed() {
    let mcp = Mcp::new(Arc::new(Store::open_in_memory().unwrap()));
    mcp.save(stdio("Flaky", &[("FAKE_MCP_CRASH", "1")]))
        .unwrap();
    assert!(matches!(settled(&mcp).await, ServerState::Ready { .. }));
    let err = mcp
        .call(
            "mcp__flaky__echo",
            json!({ "text": "x" }),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(err.contains("stopped"), "{err}");
    assert!(matches!(
        mcp.servers()[0].status,
        ServerState::Failed { .. }
    ));
}

#[tokio::test]
async fn a_server_that_quits_on_its_own_shows_as_stopped_without_a_call() {
    let mcp = Mcp::new(Arc::new(Store::open_in_memory().unwrap()));
    mcp.save(stdio("Brief", &[("FAKE_MCP_EXIT_AFTER_MS", "800")]))
        .unwrap();
    assert!(matches!(settled(&mcp).await, ServerState::Ready { .. }));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let error = loop {
        if let ServerState::Failed { error } = mcp.servers()[0].status.clone() {
            break error;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{:#?}",
            mcp.servers()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(error.contains("stopped"), "{error}");
    assert!(mcp.tools().is_empty());
}

#[tokio::test]
async fn disabling_a_server_stops_it_and_enabling_starts_it_again() {
    let mcp = Mcp::new(Arc::new(Store::open_in_memory().unwrap()));
    let id = mcp.save(stdio("Toggle", &[])).unwrap()[0].config.id.clone();
    assert!(matches!(settled(&mcp).await, ServerState::Ready { .. }));
    mcp.set_enabled(&id, false).unwrap();
    assert_eq!(mcp.servers()[0].status, ServerState::Off);
    assert!(mcp.tools().is_empty());
    mcp.set_enabled(&id, true).unwrap();
    assert!(matches!(settled(&mcp).await, ServerState::Ready { .. }));
    mcp.remove(&id).unwrap();
    assert!(mcp.servers().is_empty());
}

#[tokio::test]
async fn http_servers_work_too() {
    let server = MockServer::start(Script::new()).await;
    let mcp = Mcp::new(Arc::new(Store::open_in_memory().unwrap()));
    mcp.save(ServerConfig {
        id: String::new(),
        name: "Remote".into(),
        enabled: true,
        transport: Transport::Http {
            url: format!("{}/mcp", server.url),
            headers: BTreeMap::from([("Authorization".into(), "Bearer test".into())]),
        },
    })
    .unwrap();
    assert!(
        matches!(settled(&mcp).await, ServerState::Ready { .. }),
        "{:#?}",
        mcp.servers()
    );
    let called = mcp
        .call(
            "mcp__remote__echo",
            json!({ "text": "over http" }),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(called.text, "echo: over http");
}
