//! Running models: one llama-server per loaded model, on a free local port with a random API
//! key, in its own Job Object. A watcher polls `/health` until the model is ready and notices if
//! the server stops. When another model is needed and the limit is reached, the least recently
//! used one is unloaded first. Failures are turned into plain advice from the server's log.

use std::collections::{HashMap, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::llama::{self, STATUS_DLL_NOT_FOUND, ServerInfo};
use super::scan::{self, LocalModel};
use crate::llm::LlmClient;
use crate::settings::{ModelPreset, SettingsStore};
use crate::{new_id, now_ms, proc};

/// Log lines kept per server.
const LOG_LINES: usize = 400;
/// A model that isn't ready after this long is given up on.
const LOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const HEALTH_EVERY: Duration = Duration::from_millis(250);
/// The context Hearth picks when a preset doesn't say.
const DEFAULT_CONTEXT: u64 = 32_768;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ServerState {
    Loading { since_ms: i64 },
    Ready { since_ms: i64 },
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub model_id: String,
    #[serde(flatten)]
    pub state: ServerState,
    pub port: u16,
    /// The command line it was started with (the API key left out).
    pub command: String,
}

struct Server {
    model_id: String,
    port: u16,
    client: LlmClient,
    command: String,
    state: watch::Sender<ServerState>,
    job: proc::Job,
    logs: Arc<Mutex<VecDeque<String>>>,
    last_used: Mutex<Instant>,
    /// Hearth stopped it (so its exit isn't a failure).
    stopping: AtomicBool,
}

impl Server {
    fn alive(&self) -> bool {
        !matches!(*self.state.borrow(), ServerState::Failed { .. })
    }

    fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.job.kill();
    }
}

type Listener = Arc<dyn Fn() + Send + Sync>;

struct Inner {
    settings: Arc<SettingsStore>,
    log_dir: PathBuf,
    servers: Mutex<HashMap<String, Arc<Server>>>,
    models: Mutex<Option<Vec<LocalModel>>>,
    /// The last probe, and the file it was of (path, size, modified).
    probe: Mutex<Option<(String, ServerInfo)>>,
    /// One start or swap at a time.
    load: tokio::sync::Mutex<()>,
    listener: Mutex<Option<Listener>>,
}

/// Hearth's own llama-server processes.
#[derive(Clone)]
pub struct Runtime(Arc<Inner>);

fn fingerprint(path: &Path) -> String {
    let meta = std::fs::metadata(path).ok();
    let modified = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(|t| format!("{t:?}"))
        .unwrap_or_default();
    format!(
        "{}|{}|{modified}",
        path.display(),
        meta.map_or(0, |m| m.len())
    )
}

fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port())
}

/// Splits extra arguments on spaces, keeping quoted parts together.
fn split_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// The context to use: the preset's, else 32K (or less if the model was trained on less).
pub fn context_for(model: &LocalModel, preset: &ModelPreset) -> u64 {
    preset.context.map(u64::from).unwrap_or_else(|| {
        model
            .context_length
            .unwrap_or(DEFAULT_CONTEXT)
            .min(DEFAULT_CONTEXT)
    })
}

/// llama-server's arguments for a model. Only flags this build lists are used (all of them when
/// `--help` couldn't be read).
pub fn arguments(
    model: &LocalModel,
    preset: &ModelPreset,
    flags: &std::collections::HashSet<String>,
    port: u16,
    key: &str,
) -> Vec<String> {
    let has = |flag: &str| flags.is_empty() || flags.contains(flag);
    let mut args: Vec<String> = vec!["-m".into(), model.path.display().to_string()];
    if preset.vision
        && let Some(mmproj) = &model.mmproj
        && has("--mmproj")
    {
        args.extend(["--mmproj".into(), mmproj.display().to_string()]);
    }
    args.extend([
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "--api-key".into(),
        key.to_owned(),
        "--alias".into(),
        model.id.clone(),
        "-c".into(),
        context_for(model, preset).to_string(),
    ]);
    if has("-np") || has("--parallel") {
        args.extend(["-np".into(), "1".into()]);
    }
    if has("--jinja") {
        args.push("--jinja".into());
    }
    if has("--reasoning-format") {
        args.extend(["--reasoning-format".into(), "deepseek".into()]);
    }
    if has("--no-webui") {
        args.push("--no-webui".into());
    }
    if let Some(kv) = preset.kv_cache.as_deref().filter(|kv| !kv.is_empty())
        && has("--cache-type-k")
    {
        args.extend(["-ctk".into(), kv.to_owned(), "-ctv".into(), kv.to_owned()]);
    }
    if let Some(layers) = preset.gpu_layers {
        args.extend(["-ngl".into(), layers.to_string()]);
    }
    if let Some(moe) = preset.cpu_moe
        && has("--n-cpu-moe")
    {
        args.extend(["--n-cpu-moe".into(), moe.to_string()]);
    }
    // A big model on a 32 GB PC leaves little room for llama.cpp's 8 GB prompt cache.
    if has("--cache-ram") && model.size_bytes > 16 * 1024 * 1024 * 1024 {
        args.extend(["--cache-ram".into(), "2048".into()]);
    }
    args.extend(split_args(&preset.extra_args));
    args
}

/// What went wrong, in words a person can act on, from the server's last log lines.
pub fn explain(logs: &[String], code: Option<i32>, was_ready: bool) -> String {
    let text = logs.join("\n").to_lowercase();
    let line = |needle: &str| {
        logs.iter()
            .rev()
            .find(|l| l.to_lowercase().contains(needle))
            .cloned()
            .unwrap_or_default()
    };
    if code == Some(STATUS_DLL_NOT_FOUND) {
        return "llama-server can't start because a DLL is missing. Extract the cudart zip from the same llama.cpp \
                release into the folder with llama-server.exe."
            .into();
    }
    if [
        "out of memory",
        "failed to allocate",
        "cudamalloc failed",
        "unable to allocate",
        "not enough memory",
    ]
    .iter()
    .any(|n| text.contains(n))
    {
        return "Not enough GPU or system memory for this model with these settings. Lower its context length, use a \
                smaller quantization, or close other programs using the GPU."
            .into();
    }
    if text.contains("unknown argument")
        || text.contains("invalid argument")
        || text.contains("error: unrecognized")
    {
        let bad = line("argument");
        return format!(
            "llama-server didn't accept an argument ({}). Check the model's extra arguments, or update llama.cpp.",
            bad.trim()
        );
    }
    if text.contains("unknown model architecture")
        || text.contains("failed to load model")
        || text.contains("error loading model")
    {
        return "llama-server couldn't load this file. It may be incomplete (download it again), or this llama.cpp is \
                too old for the model (update llama.cpp)."
            .into();
    }
    if text.contains("couldn't bind") || text.contains("address already in use") {
        return "The port Hearth picked was taken. Try loading the model again.".into();
    }
    let tail: Vec<&str> = logs
        .iter()
        .rev()
        .take(3)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let what = if was_ready {
        "stopped unexpectedly"
    } else {
        "stopped while loading"
    };
    let code = code.map_or_else(|| "no exit code".to_owned(), |c| format!("exit code {c}"));
    format!(
        "llama-server {what} ({code}). Its last lines: {}",
        tail.join(" / ")
    )
}

impl Runtime {
    pub fn new(settings: Arc<SettingsStore>, log_dir: PathBuf) -> Self {
        Self(Arc::new(Inner {
            settings,
            log_dir,
            servers: Mutex::new(HashMap::new()),
            models: Mutex::new(None),
            probe: Mutex::new(None),
            load: tokio::sync::Mutex::new(()),
            listener: Mutex::new(None),
        }))
    }

    /// Called whenever a model starts loading, becomes ready, fails or is unloaded.
    pub fn on_change(&self, listener: impl Fn() + Send + Sync + 'static) {
        *lock(&self.0.listener) = Some(Arc::new(listener));
    }

    pub fn status(&self) -> Vec<ModelStatus> {
        let mut list: Vec<ModelStatus> = lock(&self.0.servers)
            .values()
            .map(|server| ModelStatus {
                model_id: server.model_id.clone(),
                state: server.state.borrow().clone(),
                port: server.port,
                command: server.command.clone(),
            })
            .collect();
        list.sort_by(|a, b| a.model_id.cmp(&b.model_id));
        list
    }

    /// Folders searched for models: the configured ones and LM Studio's.
    pub fn model_folders(&self) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = self
            .0
            .settings
            .get()
            .runtime
            .model_folders
            .iter()
            .map(PathBuf::from)
            .collect();
        folders.extend(scan::lm_studio_folders());
        folders
    }

    /// The models found (scanned on first use).
    pub fn models(&self) -> Vec<LocalModel> {
        if let Some(models) = lock(&self.0.models).clone() {
            return models;
        }
        self.rescan()
    }

    pub fn rescan(&self) -> Vec<LocalModel> {
        let models = scan::scan(&self.model_folders());
        *lock(&self.0.models) = Some(models.clone());
        models
    }

    pub fn model(&self, id: &str) -> Option<LocalModel> {
        let found = self.models().into_iter().find(|m| m.id == id);
        found.or_else(|| self.rescan().into_iter().find(|m| m.id == id))
    }

    /// llama-server and what it can do (probed once per version of the file).
    pub async fn server_info(&self, refresh: bool) -> Result<ServerInfo, String> {
        let settings = self.0.settings.get();
        let path = llama::find(settings.runtime.llama_server_path.as_deref()).ok_or_else(|| {
            match settings.runtime.llama_server_path.as_deref().filter(|p| !p.trim().is_empty()) {
                Some(path) => format!("{path} doesn't exist. Choose llama-server.exe again on the Models page."),
                None => "llama-server wasn't found. Install llama.cpp (see the models guide), or choose \
                         llama-server.exe on the Models page."
                    .to_owned(),
            }
        })?;
        let key = fingerprint(&path);
        if !refresh
            && let Some((known, info)) = lock(&self.0.probe).clone()
            && known == key
        {
            return Ok(info);
        }
        let info = llama::probe(&path).await?;
        *lock(&self.0.probe) = Some((key, info.clone()));
        Ok(info)
    }

    /// The context window a model runs with (its preset's, else Hearth's default).
    pub fn context_window(&self, model_id: &str) -> Option<u64> {
        let model = self.model(model_id)?;
        let preset = self
            .0
            .settings
            .get()
            .runtime
            .presets
            .get(model_id)
            .cloned()
            .unwrap_or_default();
        Some(context_for(&model, &preset))
    }

    pub fn is_ready(&self, model_id: &str) -> bool {
        lock(&self.0.servers)
            .get(model_id)
            .is_some_and(|server| matches!(*server.state.borrow(), ServerState::Ready { .. }))
    }

    /// The server's last log lines.
    pub fn logs(&self, model_id: &str) -> Vec<String> {
        lock(&self.0.servers)
            .get(model_id)
            .map(|server| lock(&server.logs).iter().cloned().collect())
            .unwrap_or_default()
    }

    fn notify(inner: &Inner) {
        let listener = lock(&inner.listener).clone();
        if let Some(listener) = listener {
            listener();
        }
    }

    /// Unloads a model.
    pub fn unload(&self, model_id: &str) {
        let removed = lock(&self.0.servers).remove(model_id);
        if let Some(server) = removed {
            server.stop();
            Self::notify(&self.0);
        }
    }

    pub fn unload_all(&self) {
        let all: Vec<Arc<Server>> = lock(&self.0.servers)
            .drain()
            .map(|(_, server)| server)
            .collect();
        for server in &all {
            server.stop();
        }
        if !all.is_empty() {
            Self::notify(&self.0);
        }
    }

    /// The model's server, started (or swapped in) if needed and once it's ready.
    pub async fn ensure(
        &self,
        model_id: &str,
        cancel: &CancellationToken,
    ) -> Result<LlmClient, String> {
        const STOPPED: &str = "Stopped.";
        let server = {
            let _guard = tokio::select! {
                guard = self.0.load.lock() => guard,
                () = cancel.cancelled() => return Err(STOPPED.into()),
            };
            let existing = lock(&self.0.servers).get(model_id).cloned();
            match existing {
                Some(server) if server.alive() => server,
                _ => {
                    lock(&self.0.servers).remove(model_id);
                    self.make_room(model_id);
                    self.start(model_id).await?
                }
            }
        };
        let mut state = server.state.subscribe();
        loop {
            match &*state.borrow_and_update() {
                ServerState::Ready { .. } => {
                    *lock(&server.last_used) = Instant::now();
                    return Ok(server.client.clone());
                }
                ServerState::Failed { error } => return Err(error.clone()),
                ServerState::Loading { .. } => {}
            }
            tokio::select! {
                changed = state.changed() => {
                    if changed.is_err() {
                        return Err("llama-server stopped.".into());
                    }
                }
                () = cancel.cancelled() => return Err(STOPPED.into()),
            }
        }
    }

    /// Unloads the least recently used models until another one fits under the limit.
    fn make_room(&self, keep: &str) {
        let max = self.0.settings.get().runtime.max_loaded.max(1) as usize;
        loop {
            let victim = {
                let servers = lock(&self.0.servers);
                let loaded: Vec<&Arc<Server>> = servers
                    .values()
                    .filter(|s| s.model_id != keep && s.alive())
                    .collect();
                if loaded.len() < max {
                    return;
                }
                loaded
                    .into_iter()
                    .min_by_key(|s| *lock(&s.last_used))
                    .map(|s| s.model_id.clone())
            };
            match victim {
                Some(id) => self.unload(&id),
                None => return,
            }
        }
    }

    async fn start(&self, model_id: &str) -> Result<Arc<Server>, String> {
        let model = self
            .model(model_id)
            .ok_or("That model file isn't in the model folders any more.")?;
        if model.missing_parts > 0 {
            return Err(format!(
                "{} is split into {} files and {} of them are missing. Download the rest into the same folder.",
                model.file_name, model.parts, model.missing_parts
            ));
        }
        let info = self.server_info(false).await?;
        let preset = self
            .0
            .settings
            .get()
            .runtime
            .presets
            .get(model_id)
            .cloned()
            .unwrap_or_default();
        let port = free_port().map_err(|e| format!("No free local port: {e}"))?;
        let key = format!("{}{}", new_id().replace('-', ""), new_id().replace('-', ""));
        let args = arguments(&model, &preset, &info.flags, port, &key);
        let command = std::iter::once(info.path.clone())
            .chain(args.iter().map(|a| {
                if *a == key {
                    "<key>".to_owned()
                } else {
                    a.clone()
                }
            }))
            .collect::<Vec<_>>()
            .join(" ");

        let mut cmd = proc::command(&info.path);
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = Path::new(&info.path).parent() {
            cmd.current_dir(dir);
        }
        let mut process =
            proc::spawn(&mut cmd).map_err(|e| format!("llama-server couldn't start: {e}"))?;
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        let _ = std::fs::create_dir_all(&self.0.log_dir);
        let log_file = std::fs::File::create(self.0.log_dir.join("llama-server.log"))
            .ok()
            .map(|f| Arc::new(Mutex::new(f)));
        if let Some(file) = &log_file {
            let _ = writeln!(lock(file), "{command}");
        }
        for pipe in [
            process
                .child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
            process
                .child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        ]
        .into_iter()
        .flatten()
        {
            let (logs, file) = (logs.clone(), log_file.clone());
            tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(file) = &file {
                        let _ = writeln!(lock(file), "{line}");
                    }
                    let mut logs = lock(&logs);
                    if logs.len() >= LOG_LINES {
                        logs.pop_front();
                    }
                    logs.push_back(line);
                }
            });
        }
        let client = crate::llm::LlmClient::new(&format!("http://127.0.0.1:{port}"), Some(key))
            .map_err(|e| e.to_string())?;
        let (state, _) = watch::channel(ServerState::Loading { since_ms: now_ms() });
        let proc::Process { child, job } = process;
        let server = Arc::new(Server {
            model_id: model_id.to_owned(),
            port,
            client,
            command,
            state,
            job,
            logs,
            last_used: Mutex::new(Instant::now()),
            stopping: AtomicBool::new(false),
        });
        lock(&self.0.servers).insert(model_id.to_owned(), server.clone());
        tokio::spawn(watch(Arc::downgrade(&self.0), server.clone(), child));
        Self::notify(&self.0);
        Ok(server)
    }
}

/// Follows a server: ready when `/health` answers, failed when the process ends.
async fn watch(runtime: Weak<Inner>, server: Arc<Server>, mut child: tokio::process::Child) {
    let started = Instant::now();
    let http = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_millis(800))
        .timeout(Duration::from_secs(2))
        .build();
    let health = format!("http://127.0.0.1:{}/health", server.port);
    let mut timed_out = false;
    loop {
        let loading = matches!(*server.state.borrow(), ServerState::Loading { .. });
        tokio::select! {
            status = child.wait() => {
                let code = status.ok().and_then(|s| s.code());
                let was_ready = matches!(*server.state.borrow(), ServerState::Ready { .. });
                let error = if server.stopping.load(Ordering::SeqCst) {
                    "Unloaded.".to_owned()
                } else if timed_out {
                    "The model didn't finish loading in 15 minutes, so Hearth stopped it.".to_owned()
                } else {
                    // Let the log readers catch the last lines.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let logs: Vec<String> = lock(&server.logs).iter().cloned().collect();
                    explain(&logs, code, was_ready)
                };
                server.state.send_replace(ServerState::Failed { error });
                if let Some(inner) = runtime.upgrade() {
                    Runtime::notify(&inner);
                }
                return;
            }
            () = tokio::time::sleep(HEALTH_EVERY), if loading => {
                let ok = match &http {
                    Ok(http) => http.get(&health).send().await.is_ok_and(|r| r.status().is_success()),
                    Err(_) => false,
                };
                if ok {
                    server.state.send_replace(ServerState::Ready { since_ms: now_ms() });
                    if let Some(inner) = runtime.upgrade() {
                        Runtime::notify(&inner);
                    }
                } else if started.elapsed() > LOAD_TIMEOUT && !timed_out {
                    timed_out = true;
                    server.job.kill();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn model(size_gb: u64, mmproj: bool) -> LocalModel {
        LocalModel {
            id: "m-1".into(),
            path: PathBuf::from(r"D:\Models\qwen\q.gguf"),
            file_name: "q.gguf".into(),
            name: "Qwen".into(),
            quant: None,
            size_bytes: size_gb * 1024 * 1024 * 1024,
            parts: 1,
            missing_parts: 0,
            architecture: "qwen3moe".into(),
            size_label: None,
            context_length: Some(262_144),
            layers: None,
            experts: None,
            tools: true,
            thinking: true,
            mmproj: mmproj.then(|| PathBuf::from(r"D:\Models\qwen\mmproj-F16.gguf")),
        }
    }

    #[test]
    fn arguments_use_the_preset_and_only_known_flags() {
        let preset = ModelPreset {
            context: Some(65_536),
            kv_cache: Some("q8_0".into()),
            cpu_moe: Some(20),
            extra_args: "--temp 0.6 --override-kv \"a=b c\"".into(),
            ..ModelPreset::default()
        };
        let flags: HashSet<String> = [
            "--mmproj",
            "-np",
            "--jinja",
            "--cache-type-k",
            "--cache-ram",
        ]
        .iter()
        .map(|f| (*f).into())
        .collect();
        let args = arguments(&model(20, true), &preset, &flags, 4242, "secret").join(" ");
        assert!(args.starts_with(r"-m D:\Models\qwen\q.gguf --mmproj D:\Models\qwen\mmproj-F16.gguf --host 127.0.0.1 --port 4242"), "{args}");
        assert!(
            args.contains(
                "--api-key secret --alias m-1 -c 65536 -np 1 --jinja -ctk q8_0 -ctv q8_0"
            ),
            "{args}"
        );
        assert!(args.contains("--cache-ram 2048"), "{args}");
        assert!(
            !args.contains("--n-cpu-moe") && !args.contains("--reasoning-format"),
            "unknown flags are left out: {args}"
        );
        assert!(args.ends_with("--temp 0.6 --override-kv a=b c"), "{args}");

        let small = arguments(
            &model(5, false),
            &ModelPreset::default(),
            &HashSet::new(),
            1,
            "k",
        )
        .join(" ");
        assert!(
            small.contains("-c 32768")
                && small.contains("--reasoning-format deepseek")
                && !small.contains("--cache-ram"),
            "{small}"
        );
    }

    #[test]
    fn failures_are_explained() {
        let oom = [
            "load_tensors: offloading",
            "ggml_backend_cuda_buffer_type_alloc_buffer: allocating 9000 MiB on device 0: cudaMalloc failed: out of memory",
        ];
        assert!(
            explain(&oom.map(String::from), Some(1), false)
                .starts_with("Not enough GPU or system memory")
        );
        assert!(explain(&[], Some(STATUS_DLL_NOT_FOUND), false).contains("DLL is missing"));
        let arch = [
            "llama_model_load: error loading model: error loading model architecture: unknown model architecture: 'qwen9'",
        ];
        assert!(explain(&arch.map(String::from), Some(1), false).contains("too old"));
        let other = ["a", "b", "c", "d"].map(String::from);
        assert_eq!(
            explain(&other, Some(3), true),
            "llama-server stopped unexpectedly (exit code 3). Its last lines: b / c / d"
        );
    }
}
