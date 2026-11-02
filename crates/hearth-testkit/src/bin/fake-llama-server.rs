//! A stand-in for llama.cpp's llama-server, for tests. It answers `--version`, `--list-devices`
//! and `--help` like the real one, accepts its flags, checks that the model is a GGUF file,
//! "loads" for a while (nothing listens yet), then serves the mock model's API on `--port`.
//!
//! Environment:
//! - `FAKE_LLAMA_LOAD_MS`: how long loading takes (default 800).
//! - `FAKE_LLAMA_FAIL`: `oom` (fails while loading, as llama.cpp does when memory runs out) or
//!   `crash-after-ms:N` (exits N ms after it's ready).
//! - `FAKE_LLAMA_ARGV_FILE`: the arguments it got are written there, one per line.
//! - `FAKE_LLAMA_SCRIPT`: a mock script (JSON file) for its replies; by default every reply is
//!   "Hello from <alias>." with a little thinking.
//! - `FAKE_LLAMA_DEVICES=none`: `--list-devices` finds no GPU.

use std::time::Duration;

use hearth_testkit::{MockServer, Reply, Script};

const HELP: &str = "----- common params -----
-h,    --help, --usage                  print usage and exit
--version                               show version and build info
--list-devices                          print list of available devices and exit
-t,    --threads N                      number of threads
-c,    --ctx-size N                     size of the prompt context
-n,    --predict, --n-predict N         number of tokens to predict
-b,    --batch-size N                   logical maximum batch size
-ub,   --ubatch-size N                  physical maximum batch size
-fa,   --flash-attn [on|off|auto]       set Flash Attention use
-ctk,  --cache-type-k TYPE              KV cache data type for K
-ctv,  --cache-type-v TYPE              KV cache data type for V
-np,   --parallel N                     number of parallel sequences to decode
--mmproj FILE                           path to a multimodal projector file
-ngl,  --gpu-layers, --n-gpu-layers N   max. number of layers to store in VRAM
-cmoe, --cpu-moe                        keep all MoE weights in the CPU
-ncmoe, --n-cpu-moe N                   keep the MoE weights of the first N layers in the CPU
--fit [on|off]                          fit unset arguments to the device memory
-m,    --model FNAME                    model path
-a,    --alias STRING                   set alias for model name
--host HOST                             ip address to listen
--port PORT                             port to listen
--api-key KEY                           API key to use for authentication
--jinja                                 use jinja template for chat
--reasoning-format FORMAT               controls whether thought tags are allowed and/or extracted
--no-webui                              disable the Web UI
--cache-ram N                           set the maximum cache size in MiB";

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(file) = std::env::var_os("FAKE_LLAMA_ARGV_FILE") {
        let _ = std::fs::write(file, args.join("\n"));
    }
    match args.first().map(String::as_str) {
        Some("--version") => {
            eprintln!("version: 9999 (fa4e11a)\nbuilt with fake toolchain for x64");
            return;
        }
        Some("--list-devices") => {
            if std::env::var("FAKE_LLAMA_DEVICES").as_deref() == Ok("none") {
                println!("Available devices:");
            } else {
                println!(
                    "Available devices:\n  CUDA0: NVIDIA GeForce RTX 5070 (12227 MiB, 11000 MiB free)"
                );
            }
            return;
        }
        Some("--help" | "-h") => {
            println!("{HELP}");
            return;
        }
        _ => {}
    }

    let value = |names: &[&str]| -> Option<String> {
        args.iter()
            .position(|a| names.contains(&a.as_str()))
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let known: Vec<&str> = HELP
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| w.starts_with('-'))
        .collect();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg.starts_with('-') && !known.contains(&arg) {
            fail(&format!("error: invalid argument: {arg}"));
        }
        i += 1;
    }
    let model = value(&["-m", "--model"]).unwrap_or_else(|| fail("error: no model given (-m)"));
    let port: u16 = value(&["--port"])
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let alias = value(&["-a", "--alias"]).unwrap_or_else(|| "model".into());
    eprintln!("main: loading model {model}");
    match std::fs::read(&model) {
        Ok(bytes) if bytes.starts_with(b"GGUF") => {}
        Ok(_) => fail("llama_model_load: error loading model: invalid magic (not a GGUF file)"),
        Err(err) => fail(&format!("llama_model_load: error loading model: {err}")),
    }
    let load_ms: u64 = std::env::var("FAKE_LLAMA_LOAD_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(800);
    let failure = std::env::var("FAKE_LLAMA_FAIL").unwrap_or_default();

    let script = match std::env::var_os("FAKE_LLAMA_SCRIPT") {
        Some(path) => {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| fail(&format!("can't read the script: {e}")));
            serde_json::from_str(&text).unwrap_or_else(|e| fail(&format!("bad script: {e}")))
        }
        None => {
            let mut script = Script::new();
            for _ in 0..200 {
                script = script.reply(
                    Reply::text(format!("Hello from {alias}."))
                        .with_reasoning("The user said hi.")
                        .usage(120, 6)
                        .timings(serde_json::json!({ "predicted_per_second": 38.5, "prompt_per_second": 900.0 })),
                );
            }
            script
        }
    };
    // Like llama-server: the port is open at once, and /health answers 503 while loading.
    let server = MockServer::start_on(script, port)
        .await
        .unwrap_or_else(|e| fail(&format!("couldn't bind to port {port}: {e}")));
    server.set_loading(true);
    if failure == "oom" {
        tokio::time::sleep(Duration::from_millis(load_ms.min(300))).await;
        eprintln!("load_tensors: offloading 48 repeating layers to GPU");
        fail(
            "ggml_backend_cuda_buffer_type_alloc_buffer: allocating 21000.00 MiB on device 0: cudaMalloc failed: out of memory",
        );
    }
    tokio::time::sleep(Duration::from_millis(load_ms)).await;
    server.set_loading(false);
    eprintln!("main: server is listening on {}", server.url);
    if let Some(ms) = failure
        .strip_prefix("crash-after-ms:")
        .and_then(|n| n.parse::<u64>().ok())
    {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        eprintln!("GGML_ASSERT failed: fake crash");
        std::process::exit(3);
    }
    std::future::pending::<()>().await;
}
