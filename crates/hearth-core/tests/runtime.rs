//! The managed runtime against the testkit's fake llama-server: probing it, loading models on
//! first use, swapping them, explaining failures, and chatting through a local model.
//!
//! One test runs the scenarios in order, because the fake is configured through environment
//! variables that every child process inherits.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hearth_core::chat::{Chat, ChatAction, StoreResolver};
use hearth_core::runs::{CollectSink, RunEnd, RunEvent, RunManager};
use hearth_core::runtime::gguf::fixture::{V, gguf};
use hearth_core::runtime::{Runtime, ServerState};
use hearth_core::settings::SettingsStore;
use hearth_core::store::{Mode, Store};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// target/debug/fake-llama-server.exe, next to this test's own folder.
fn fake_server() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.parent()?.join("fake-llama-server.exe");
    if path.is_file() {
        Some(path)
    } else {
        eprintln!(
            "{} isn't built (cargo build -p hearth-testkit --bins); skipped",
            path.display()
        );
        None
    }
}

fn set_env(key: &str, value: &str) {
    // SAFETY: this test binary has one test, and nothing else reads these variables meanwhile.
    unsafe { std::env::set_var(key, value) };
}

async fn finished(sink: &CollectSink) -> RunEnd {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(end) = sink.events().iter().find_map(|e| match e {
            RunEvent::Finished { status } => Some(*status),
            _ => None,
        }) {
            return end;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no finish: {:#?}",
            sink.events()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn models_load_swap_fail_and_chat() {
    let Some(server) = fake_server() else { return };
    let data = tempfile::tempdir().unwrap();
    let models = data.path().join("models");
    std::fs::create_dir_all(models.join("qwen")).unwrap();
    std::fs::create_dir_all(models.join("gemma")).unwrap();
    let header = |arch: &str, name: &str| {
        gguf(
            &[
                ("general.architecture", V::Str(arch)),
                ("general.name", V::Str(name)),
                ("general.file_type", V::U32(15)),
                (
                    "tokenizer.chat_template",
                    V::Str("{% if tools %}{% endif %}"),
                ),
            ],
            64,
        )
    };
    std::fs::write(
        models.join("qwen/Qwen3.6-35B-A3B-Q4_K_M.gguf"),
        header("qwen3moe", "Qwen3.6 35B A3B"),
    )
    .unwrap();
    std::fs::write(
        models.join("gemma/gemma-4-26B-A4B-it-Q4_K_M.gguf"),
        header("gemma4", "Gemma 4 26B A4B"),
    )
    .unwrap();
    let argv = data.path().join("argv.txt");
    set_env("FAKE_LLAMA_LOAD_MS", "300");
    set_env("FAKE_LLAMA_ARGV_FILE", &argv.display().to_string());

    let settings = Arc::new(SettingsStore::open(data.path().join("settings.json")));
    settings
        .update(&json!({ "runtime": {
            "llamaServerPath": server.display().to_string(),
            "modelFolders": [models.display().to_string()],
            "maxLoaded": 1,
            "presets": {}
        } }))
        .unwrap();
    let runtime = Runtime::new(settings.clone(), data.path().join("logs"));
    let changes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = changes.clone();
    runtime.on_change(move || {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });

    // Probing.
    let info = runtime.server_info(false).await.unwrap();
    assert_eq!(info.version.as_deref(), Some("build 9999 (fa4e11a)"));
    assert_eq!(info.devices[0].name, "NVIDIA GeForce RTX 5070");
    assert!(info.warnings.is_empty());

    // Scanning.
    let found = runtime.models();
    let names: Vec<&str> = found.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Gemma 4 26B A4B", "Qwen3.6 35B A3B"]);
    let (gemma, qwen) = (found[0].id.clone(), found[1].id.clone());

    // Loading on first use, with the right arguments.
    let cancel = CancellationToken::new();
    let client = runtime.ensure(&qwen, &cancel).await.unwrap();
    assert!(runtime.is_ready(&qwen));
    assert_eq!(client.models().await.unwrap(), ["mock-model"]);
    let args = std::fs::read_to_string(&argv).unwrap().replace('\n', " ");
    assert!(
        args.contains(&format!(
            "--alias {qwen} -c 32768 -np 1 --jinja --reasoning-format deepseek --no-webui"
        )),
        "{args}"
    );
    assert!(
        changes.load(std::sync::atomic::Ordering::SeqCst) >= 2,
        "loading and ready were reported"
    );

    // Another model swaps the first out (one at a time).
    runtime.ensure(&gemma, &cancel).await.unwrap();
    let status = runtime.status();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].model_id, gemma);
    assert!(matches!(status[0].state, ServerState::Ready { .. }));

    // A chat with a local model loads it and answers through it.
    runtime.unload_all();
    let store = Arc::new(Store::open_in_memory().unwrap());
    settings
        .update(&json!({ "models": { "chat": { "kind": "local", "modelId": qwen } } }))
        .unwrap();
    let runs = Arc::new(RunManager::new());
    let chat = Chat::new(
        store.clone(),
        runs,
        Arc::new(StoreResolver::with_runtime(store.clone(), runtime.clone())),
        settings.clone(),
    );
    let session = store.create_session(Mode::Chat, None).unwrap();
    let sink = Arc::new(CollectSink::default());
    chat.start(
        &session.id,
        ChatAction::Send {
            text: "Hi".into(),
            web: None,
        },
        sink.clone(),
    )
    .unwrap();
    assert_eq!(
        finished(&sink).await,
        RunEnd::Completed,
        "{:#?}",
        sink.events()
    );
    let events = sink.events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunEvent::ModelLoading { model } if model == "Qwen3.6 35B A3B"))
    );
    let reply = store.active_path(&session.id).unwrap().pop().unwrap();
    assert_eq!(reply.content, format!("Hello from {qwen}."));
    assert_eq!(reply.model_label.as_deref(), Some("Qwen3.6 35B A3B"));

    // Running out of memory is explained.
    runtime.unload_all();
    set_env("FAKE_LLAMA_FAIL", "oom");
    let error = runtime.ensure(&gemma, &cancel).await.unwrap_err();
    assert!(
        error.starts_with("Not enough GPU or system memory"),
        "{error}"
    );
    assert!(
        runtime
            .logs(&gemma)
            .iter()
            .any(|line| line.contains("cudaMalloc failed"))
    );

    // A crash after loading is noticed.
    set_env("FAKE_LLAMA_FAIL", "crash-after-ms:300");
    runtime.ensure(&gemma, &cancel).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    match &runtime.status()[0].state {
        ServerState::Failed { error } => assert!(error.contains("stopped unexpectedly"), "{error}"),
        other => panic!("{other:?}"),
    }
    set_env("FAKE_LLAMA_FAIL", "");
    // …and the next request starts it again.
    runtime.ensure(&gemma, &cancel).await.unwrap();
    runtime.unload_all();
    assert!(runtime.status().is_empty());

    // A wrong path is explained.
    settings
        .update(&json!({ "runtime": { "llamaServerPath": "C:\\nowhere\\llama-server.exe" } }))
        .unwrap();
    let error = runtime.server_info(true).await.unwrap_err();
    assert!(error.contains("doesn't exist"), "{error}");
}
