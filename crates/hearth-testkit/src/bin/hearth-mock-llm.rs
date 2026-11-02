//! Standalone mock model server for end-to-end tests.
//!
//! `hearth-mock-llm [--script <file.json>] [--port <n>] [--port-file <path>]`
//!
//! Serves the OpenAI-compatible API on 127.0.0.1 until killed. Tests can replace the script and
//! inspect requests through the `/__mock/*` endpoints.

use std::path::PathBuf;

use hearth_testkit::{MockServer, Script};

#[tokio::main]
async fn main() {
    let mut script = Script::new();
    let mut port = 0u16;
    let mut port_file: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .unwrap_or_else(|| fail(&format!("{arg} needs a value")));
        match arg.as_str() {
            "--script" => {
                let text = std::fs::read_to_string(&value)
                    .unwrap_or_else(|e| fail(&format!("can't read {value}: {e}")));
                script = serde_json::from_str(&text)
                    .unwrap_or_else(|e| fail(&format!("bad script {value}: {e}")));
            }
            "--port" => {
                port = value
                    .parse()
                    .unwrap_or_else(|_| fail("--port needs a number"))
            }
            "--port-file" => port_file = Some(PathBuf::from(value)),
            other => fail(&format!("unknown argument {other}")),
        }
    }
    let server = MockServer::start_on(script, port)
        .await
        .unwrap_or_else(|e| fail(&format!("can't listen: {e}")));
    if let Some(path) = port_file {
        std::fs::write(&path, server.port.to_string())
            .unwrap_or_else(|e| fail(&format!("can't write port file: {e}")));
    }
    println!("hearth-mock-llm listening on {}", server.url);
    let _ = tokio::signal::ctrl_c().await;
}

fn fail(message: &str) -> ! {
    eprintln!("hearth-mock-llm: {message}");
    std::process::exit(2);
}
