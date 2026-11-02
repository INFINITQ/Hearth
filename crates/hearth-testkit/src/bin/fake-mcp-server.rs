//! A tiny MCP server over stdio, for tests (see `hearth_testkit::mcp`).
//!
//! - `FAKE_MCP_FAIL=1`: says why on stderr and exits before answering anything.
//! - `FAKE_MCP_CRASH=1`: exits when a tool is called.
//! - `FAKE_MCP_EXIT_AFTER_MS=n`: exits on its own n ms after it starts.

use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::Value;

fn main() {
    if std::env::var_os("FAKE_MCP_FAIL").is_some() {
        eprintln!("fake-mcp: the API key is missing");
        std::process::exit(2);
    }
    if let Some(ms) = std::env::var("FAKE_MCP_EXIT_AFTER_MS")
        .ok()
        .and_then(|ms| ms.parse::<u64>().ok())
    {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            eprintln!("fake-mcp quit");
            std::process::exit(4);
        });
    }
    eprintln!("fake-mcp ready");
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if message["method"] == "tools/call" && std::env::var_os("FAKE_MCP_CRASH").is_some() {
            eprintln!("fake-mcp crashed");
            std::process::exit(3);
        }
        if let Some(answer) = hearth_testkit::mcp::answer(&message)
            && writeln!(stdout, "{answer}")
                .and_then(|()| stdout.flush())
                .is_err()
        {
            break;
        }
    }
}
