//! The LLM client against the scripted mock server: streaming, tool calls, framing quirks,
//! errors, cancelling and held replies.

use std::time::Duration;

use futures_util::StreamExt;
use hearth_core::llm::{
    Assembler, ChatMessage, ChatRequest, FinishReason, Health, LlmClient, LlmError, StreamEvent,
};
use hearth_testkit::{Expect, Fault, MockServer, Reply, Script};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn request() -> ChatRequest {
    ChatRequest {
        model: "mock-model".into(),
        messages: vec![ChatMessage::system("Be brief."), ChatMessage::user("Hi")],
        ..Default::default()
    }
}

async fn collect(client: &LlmClient) -> Result<Vec<StreamEvent>, LlmError> {
    let mut stream = client
        .stream_chat(&request(), CancellationToken::new())
        .await?;
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event?);
    }
    Ok(events)
}

fn texts(events: &[StreamEvent]) -> (String, String) {
    let mut reasoning = String::new();
    let mut content = String::new();
    for event in events {
        match event {
            StreamEvent::Reasoning(text) => reasoning.push_str(text),
            StreamEvent::Content(text) => content.push_str(text),
            _ => {}
        }
    }
    (reasoning, content)
}

async fn client_for(script: Script) -> (MockServer, LlmClient) {
    let server = MockServer::start(script).await;
    let client = LlmClient::new(&server.url, None).unwrap();
    (server, client)
}

#[tokio::test]
async fn streams_reasoning_content_and_usage() {
    let reply = Reply::text("Hello there, friend!")
        .with_reasoning("The user greets me.")
        .chunked(3)
        .usage(120, 6)
        .timings(json!({ "predicted_per_second": 41.5, "cache_n": 100 }));
    let (server, client) = client_for(Script::new().reply(reply)).await;

    let events = collect(&client).await.unwrap();
    assert_eq!(
        texts(&events),
        ("The user greets me.".into(), "Hello there, friend!".into())
    );
    assert!(events.contains(&StreamEvent::Finish(FinishReason::Stop)));
    let usage = events
        .iter()
        .find_map(|e| match e {
            StreamEvent::Usage(usage) => Some(usage.clone()),
            _ => None,
        })
        .expect("usage arrives at the end");
    assert_eq!(usage.prompt_tokens, 120);
    assert_eq!(usage.completion_tokens, 6);
    assert_eq!(usage.cached_tokens, Some(100));
    assert_eq!(usage.predicted_per_second, Some(41.5));

    let sent = server.last_request().unwrap();
    assert_eq!(sent["stream"], json!(true));
    assert_eq!(sent["stream_options"]["include_usage"], json!(true));
    assert_eq!(
        sent["messages"][1],
        json!({ "role": "user", "content": "Hi" })
    );
}

#[tokio::test]
async fn assembles_streamed_tool_calls() {
    let reply = Reply::tool_call("read_file", json!({ "path": "src/main.rs" }))
        .and_tool_call("grep", json!({ "pattern": "fn main" }))
        .chunked(2);
    let (_server, client) = client_for(Script::new().reply(reply)).await;

    let events = collect(&client).await.unwrap();
    let mut assembler = Assembler::new();
    for event in &events {
        assembler.push(event);
    }
    let reply = assembler.finish();
    assert_eq!(reply.finish, Some(FinishReason::ToolCalls));
    let calls: Vec<(String, Value)> = reply
        .tool_calls
        .iter()
        .map(|c| {
            (
                c.function.name.clone(),
                serde_json::from_str(&c.function.arguments).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        calls,
        [
            ("read_file".to_owned(), json!({ "path": "src/main.rs" })),
            ("grep".to_owned(), json!({ "pattern": "fn main" })),
        ]
    );
}

#[tokio::test]
async fn unicode_survives_split_characters_and_crlf_framing() {
    let text = "héllo wörld — ✓ 日本語";
    let script = Script::new()
        .reply(Reply::text(text).with_fault(Fault::SplitUtf8))
        .reply(Reply::text(text).with_fault(Fault::CrlfSse));
    let (_server, client) = client_for(script).await;
    assert_eq!(texts(&collect(&client).await.unwrap()).1, text);
    assert_eq!(texts(&collect(&client).await.unwrap()).1, text);
}

#[tokio::test]
async fn a_reply_without_done_still_completes() {
    let (_server, client) =
        client_for(Script::new().reply(Reply::text("All good.").with_fault(Fault::NoDone))).await;
    let events = collect(&client).await.unwrap();
    assert_eq!(texts(&events).1, "All good.");
}

#[tokio::test]
async fn a_dropped_connection_is_reported() {
    let reply = Reply::text("This reply never finishes properly.")
        .with_fault(Fault::DisconnectAfterEvents(3));
    let (_server, client) = client_for(Script::new().reply(reply)).await;
    assert_eq!(
        collect(&client).await.unwrap_err(),
        LlmError::ConnectionLost
    );
}

#[tokio::test]
async fn server_errors_map_to_clear_kinds() {
    let script = Script::new()
        .reply(Reply::fault(Fault::ContextExceeded {
            n_ctx: 4096,
            n_prompt: 5000,
        }))
        .reply(Reply::fault(Fault::Http {
            status: 401,
            body: json!({ "error": { "message": "Invalid API Key" } }),
        }))
        .reply(Reply::fault(Fault::Loading))
        .reply(Reply::fault(Fault::Http {
            status: 500,
            body: json!({ "error": { "message": "boom" } }),
        }));
    let (_server, client) = client_for(script).await;

    assert_eq!(
        collect(&client).await.unwrap_err(),
        LlmError::ContextExceeded {
            n_prompt: 5000,
            n_ctx: 4096
        }
    );
    assert_eq!(collect(&client).await.unwrap_err(), LlmError::Unauthorized);
    assert_eq!(collect(&client).await.unwrap_err(), LlmError::Loading);
    let err = collect(&client).await.unwrap_err();
    assert_eq!(
        err,
        LlmError::Http {
            status: 500,
            message: "boom".into()
        }
    );
    assert!(err.retryable());
}

#[tokio::test]
async fn an_unreachable_server_is_reported() {
    let client = LlmClient::new("http://127.0.0.1:9", None).unwrap();
    let err = client
        .stream_chat(&request(), CancellationToken::new())
        .await
        .err()
        .unwrap();
    assert!(matches!(err, LlmError::Unreachable { .. }), "{err:?}");
    assert!(err.retryable());
}

#[tokio::test]
async fn cancelling_stops_the_stream_and_hangs_up() {
    let long = "word ".repeat(100);
    let (server, client) =
        client_for(Script::new().reply(Reply::text(long).chunked(5).delayed(15))).await;
    let cancel = CancellationToken::new();
    let mut stream = client
        .stream_chat(&request(), cancel.clone())
        .await
        .unwrap();
    loop {
        if let Some(Ok(StreamEvent::Content(_))) = stream.next().await {
            break;
        }
    }
    cancel.cancel();
    assert_eq!(stream.next().await, Some(Err(LlmError::Cancelled)));
    drop(stream);

    // The server notices the hang-up, which is what makes llama-server stop generating.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while server.disconnects() == 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(server.disconnects(), 1);
}

#[tokio::test]
async fn held_replies_wait_for_release() {
    let (server, client) = client_for(Script::new().reply(Reply::text("done").held("final"))).await;
    let task = tokio::spawn(async move { collect(&client).await });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !task.is_finished(),
        "the reply should be waiting at its hold point"
    );
    server.release("final");
    let events = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(texts(&events).1, "done");
}

#[tokio::test]
async fn expectation_failures_surface_as_errors() {
    let script = Script::new().expect_then(
        vec![Expect::SystemContains("AGENTS.md".into())],
        Reply::text("x"),
    );
    let (_server, client) = client_for(script).await;
    match collect(&client).await.unwrap_err() {
        LlmError::Http {
            status: 500,
            message,
        } => assert!(message.contains("mock expectation failed"), "{message}"),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn whole_replies_models_props_and_health() {
    let script = Script {
        n_ctx: 8192,
        ..Script::new()
    }
    .reply(Reply::text("A title").with_reasoning("short"));
    let (_server, client) = client_for(script).await;

    let reply = client.complete(&request()).await.unwrap();
    assert_eq!(
        (reply.content.as_str(), reply.reasoning.as_str()),
        ("A title", "short")
    );
    assert_eq!(reply.finish, Some(FinishReason::Stop));
    assert_eq!(client.models().await.unwrap(), ["mock-model"]);
    assert_eq!(
        client.props().await.unwrap()["default_generation_settings"]["n_ctx"],
        json!(8192)
    );
    assert_eq!(client.health().await.unwrap(), Health::Ready);
}
