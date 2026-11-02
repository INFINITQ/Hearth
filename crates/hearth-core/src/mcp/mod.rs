//! MCP servers: their settings (kept in the database), running them, and their tools for the
//! agent, named `mcp__<server>__<tool>`.
//!
//! Servers start when Hearth starts (and again when their settings change). Each stdio server
//! runs in its own Job Object, so it ends with Hearth. A server that fails shows why, with the
//! last lines it wrote to stderr.

mod client;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

pub use client::{Client, PROTOCOL_VERSION, find_program};

use crate::llm::ToolDef;
use crate::new_id;
use crate::store::Store;

/// Where the servers' settings are kept (the `kv` table).
const KEY: &str = "mcp.servers";
/// Results longer than this are cut (their start and end are kept).
const RESULT_CHARS: usize = 40_000;

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(flatten)]
    pub transport: Transport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transport {
    /// A program Hearth starts, talking over its stdin and stdout.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        cwd: Option<String>,
    },
    /// A server reached over Streamable HTTP.
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    /// What the agent calls it.
    pub name: String,
    /// The server's own name for it.
    pub tool: String,
    pub description: String,
    #[serde(skip)]
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ServerState {
    Off,
    Starting,
    Ready {
        /// The server's own name for itself.
        server: String,
        tools: Vec<ToolInfo>,
    },
    Failed {
        error: String,
    },
}

/// A server as the Settings page shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    #[serde(flatten)]
    pub config: ServerConfig,
    pub status: ServerState,
    pub log: Vec<String>,
}

/// What a tool call came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Called {
    pub text: String,
    pub is_error: bool,
}

struct Live {
    config: ServerConfig,
    state: ServerState,
    client: Option<Arc<Client>>,
    /// Bumped by every (re)start, so a slow start that was replaced can't win.
    generation: u64,
}

struct Inner {
    store: Option<Arc<Store>>,
    servers: Mutex<Vec<Live>>,
    listener: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

/// The MCP servers. Cheap to clone.
#[derive(Clone)]
pub struct Mcp(Arc<Inner>);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Lowercase letters, digits, `-` and `_` (what tool names may hold).
fn slug(text: &str, lower: bool) -> String {
    let mut out = String::new();
    for c in text.trim().chars() {
        let c = if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            if lower { c.to_ascii_lowercase() } else { c }
        } else {
            '_'
        };
        if !(c == '_' && out.ends_with('_')) {
            out.push(c);
        }
    }
    out.trim_matches('_').to_owned()
}

/// The agent's name for a server's tool (at most 64 characters, as APIs require).
pub fn tool_name(server: &str, tool: &str) -> String {
    let name = format!("mcp__{}__{}", slug(server, true), slug(tool, false));
    if name.len() <= 64 {
        return name;
    }
    // FNV-1a keeps long names apart.
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    format!("{}_{hash:08x}", &name[..55])
}

fn tool_infos(server: &str, tools: &[Value]) -> Vec<ToolInfo> {
    let mut infos: Vec<ToolInfo> = tools
        .iter()
        .filter_map(|tool| {
            let name = tool["name"].as_str()?;
            let schema = match tool.get("inputSchema") {
                Some(schema) if schema.is_object() => schema.clone(),
                _ => json!({ "type": "object", "properties": {} }),
            };
            Some(ToolInfo {
                name: tool_name(server, name),
                tool: name.to_owned(),
                description: tool["description"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
                schema,
            })
        })
        .collect();
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    infos
}

/// A tool result as the model reads it.
fn result_text(result: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    for item in result["content"].as_array().into_iter().flatten() {
        let text = match item["type"].as_str() {
            Some("text") => item["text"].as_str().unwrap_or_default().to_owned(),
            Some(kind @ ("image" | "audio")) => format!(
                "({kind}, {}, not shown)",
                item["mimeType"].as_str().unwrap_or("unknown type")
            ),
            Some("resource") => {
                let resource = &item["resource"];
                resource["text"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        format!("(resource {})", resource["uri"].as_str().unwrap_or("?"))
                    })
            }
            Some("resource_link") => format!(
                "{} ({})",
                item["name"].as_str().unwrap_or("a resource"),
                item["uri"].as_str().unwrap_or("?")
            ),
            _ => item.to_string(),
        };
        parts.push(text);
    }
    if parts.is_empty()
        && let Some(structured) = result.get("structuredContent")
    {
        parts.push(structured.to_string());
    }
    let text = parts.join("\n");
    let count = text.chars().count();
    if count <= RESULT_CHARS {
        return text;
    }
    let head: String = text.chars().take(RESULT_CHARS * 3 / 4).collect();
    let tail: String = text.chars().skip(count - RESULT_CHARS / 4).collect();
    format!(
        "{head}\n… ({} characters left out) …\n{tail}",
        count - RESULT_CHARS
    )
}

/// Servers from a Claude Desktop style config: `{ "mcpServers": { "<name>": { "command",
/// "args", "env" } | { "url", "headers" } } }` (the inner object alone works too).
pub fn parse_import(text: &str) -> Result<Vec<ServerConfig>, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|err| format!("This isn't valid JSON: {err}"))?;
    let servers = value
        .get("mcpServers")
        .or_else(|| value.get("servers"))
        .unwrap_or(&value);
    let object = servers
        .as_object()
        .ok_or("Expected an object of servers, like { \"mcpServers\": { … } }.")?;
    let strings = |value: &Value| -> BTreeMap<String, String> {
        value
            .as_object()
            .map(|map| {
                map.iter()
                    .map(|(k, v)| {
                        let v = v
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string());
                        (k.clone(), v)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut configs = Vec::new();
    for (name, entry) in object {
        let transport = if let Some(url) = entry["url"].as_str() {
            Transport::Http {
                url: url.to_owned(),
                headers: strings(&entry["headers"]),
            }
        } else if let Some(command) = entry["command"].as_str() {
            Transport::Stdio {
                command: command.to_owned(),
                args: entry["args"]
                    .as_array()
                    .map(|args| {
                        args.iter()
                            .map(|a| {
                                a.as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| a.to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                env: strings(&entry["env"]),
                cwd: entry["cwd"].as_str().map(str::to_owned),
            }
        } else {
            return Err(format!("“{name}” has neither a command nor a url."));
        };
        configs.push(ServerConfig {
            id: String::new(),
            name: name.clone(),
            enabled: !entry["disabled"].as_bool().unwrap_or(false),
            transport,
        });
    }
    if configs.is_empty() {
        return Err("No servers were found in it.".into());
    }
    Ok(configs)
}

fn check(config: &ServerConfig) -> Result<(), String> {
    if config.name.trim().is_empty() || slug(&config.name, true).is_empty() {
        return Err("Give the server a name (letters or digits).".into());
    }
    match &config.transport {
        Transport::Stdio { command, .. } if command.trim().is_empty() => {
            Err("Say which command starts the server.".into())
        }
        Transport::Http { url, .. } if reqwest::Url::parse(url.trim()).is_err() => {
            Err(format!("“{url}” isn't a web address."))
        }
        _ => Ok(()),
    }
}

/// An error with the last lines the server wrote to stderr (usually the reason).
async fn with_log(client: &Client, err: String) -> String {
    // Let the last lines arrive.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let log = client.log();
    let start = log.len().saturating_sub(4);
    if log.is_empty() {
        err
    } else {
        format!("{err}\n{}", log[start..].join("\n"))
    }
}

async fn connect(config: &ServerConfig) -> Result<(Arc<Client>, String, Vec<ToolInfo>), String> {
    let client = Arc::new(match &config.transport {
        Transport::Stdio {
            command,
            args,
            env,
            cwd,
        } => Client::stdio(command, args, env, cwd.as_deref().map(Path::new))?,
        Transport::Http { url, headers } => Client::http(url.trim(), headers)?,
    });
    let info = match client.initialize().await {
        Ok(info) => info,
        Err(err) => return Err(with_log(&client, err).await),
    };
    let tools = match client.list_tools().await {
        Ok(tools) => tools,
        Err(err) => return Err(with_log(&client, err).await),
    };
    let server = info["name"].as_str().unwrap_or(&config.name).to_owned();
    Ok((client.clone(), server, tool_infos(&config.name, &tools)))
}

impl Mcp {
    /// The servers saved in `store` (not started yet: see [`Mcp::start_all`]).
    pub fn new(store: Arc<Store>) -> Self {
        let configs: Vec<ServerConfig> = store
            .kv_get(KEY)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        let servers = configs
            .into_iter()
            .map(|config| Live {
                state: if config.enabled {
                    ServerState::Starting
                } else {
                    ServerState::Off
                },
                config,
                client: None,
                generation: 0,
            })
            .collect();
        Self(Arc::new(Inner {
            store: Some(store),
            servers: Mutex::new(servers),
            listener: Mutex::new(None),
        }))
    }

    /// No servers at all (tests, and agents without MCP).
    pub fn disabled() -> Self {
        Self(Arc::new(Inner {
            store: None,
            servers: Mutex::new(Vec::new()),
            listener: Mutex::new(None),
        }))
    }

    /// Called whenever a server's state changes.
    pub fn on_change(&self, listener: impl Fn() + Send + Sync + 'static) {
        *lock(&self.0.listener) = Some(Arc::new(listener));
    }

    fn changed(&self) {
        let listener = lock(&self.0.listener).clone();
        if let Some(listener) = listener {
            listener();
        }
    }

    pub fn servers(&self) -> Vec<ServerView> {
        lock(&self.0.servers)
            .iter()
            .map(|live| ServerView {
                config: live.config.clone(),
                status: live.state.clone(),
                log: live.client.as_ref().map(|c| c.log()).unwrap_or_default(),
            })
            .collect()
    }

    fn persist(&self) -> Result<(), String> {
        let Some(store) = &self.0.store else {
            return Ok(());
        };
        let configs: Vec<ServerConfig> = lock(&self.0.servers)
            .iter()
            .map(|live| live.config.clone())
            .collect();
        let json = serde_json::to_string(&configs).map_err(|err| err.to_string())?;
        store
            .kv_set(KEY, Some(&json))
            .map_err(|err| err.to_string())
    }

    /// Adds a server, or changes one (same id), and (re)starts it.
    pub fn save(&self, mut config: ServerConfig) -> Result<Vec<ServerView>, String> {
        check(&config)?;
        config.name = config.name.trim().to_owned();
        {
            let mut servers = lock(&self.0.servers);
            let taken = servers.iter().any(|live| {
                live.config.id != config.id
                    && slug(&live.config.name, true) == slug(&config.name, true)
            });
            if taken {
                return Err(format!(
                    "There's already a server called “{}”.",
                    config.name
                ));
            }
            if config.id.is_empty() {
                config.id = new_id();
            }
            match servers.iter_mut().find(|live| live.config.id == config.id) {
                Some(live) => live.config = config.clone(),
                None => servers.push(Live {
                    config: config.clone(),
                    state: ServerState::Off,
                    client: None,
                    generation: 0,
                }),
            }
        }
        self.persist()?;
        self.restart(&config.id);
        Ok(self.servers())
    }

    pub fn remove(&self, id: &str) -> Result<Vec<ServerView>, String> {
        lock(&self.0.servers).retain(|live| live.config.id != id);
        self.persist()?;
        self.changed();
        Ok(self.servers())
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<Vec<ServerView>, String> {
        if let Some(live) = lock(&self.0.servers)
            .iter_mut()
            .find(|live| live.config.id == id)
        {
            live.config.enabled = enabled;
        }
        self.persist()?;
        self.restart(id);
        Ok(self.servers())
    }

    /// Adds the servers in a Claude Desktop style config (replacing ones of the same name).
    pub fn import(&self, text: &str) -> Result<Vec<ServerView>, String> {
        for mut config in parse_import(text)? {
            let existing = lock(&self.0.servers)
                .iter()
                .find(|live| slug(&live.config.name, true) == slug(&config.name, true))
                .map(|live| live.config.id.clone());
            config.id = existing.unwrap_or_default();
            self.save(config)?;
        }
        Ok(self.servers())
    }

    /// Starts every enabled server.
    pub fn start_all(&self) {
        let ids: Vec<String> = lock(&self.0.servers)
            .iter()
            .map(|live| live.config.id.clone())
            .collect();
        for id in ids {
            self.restart(&id);
        }
    }

    /// Stops a server and, if it's enabled, starts it again.
    pub fn restart(&self, id: &str) {
        let start = {
            let mut servers = lock(&self.0.servers);
            let Some(live) = servers.iter_mut().find(|live| live.config.id == id) else {
                return;
            };
            live.generation += 1;
            live.client = None;
            live.state = if live.config.enabled {
                ServerState::Starting
            } else {
                ServerState::Off
            };
            live.config
                .enabled
                .then(|| (live.config.clone(), live.generation))
        };
        self.changed();
        let Some((config, generation)) = start else {
            return;
        };
        let this = self.clone();
        tokio::spawn(async move {
            let started = connect(&config).await;
            {
                let mut servers = lock(&this.0.servers);
                let Some(live) = servers
                    .iter_mut()
                    .find(|live| live.config.id == config.id && live.generation == generation)
                else {
                    return;
                };
                match started {
                    Ok((client, server, tools)) => {
                        live.client = Some(client);
                        live.state = ServerState::Ready { server, tools };
                    }
                    Err(error) => live.state = ServerState::Failed { error },
                }
            }
            this.changed();
        });
    }

    /// Every ready server's tools.
    pub fn tools(&self) -> Vec<ToolInfo> {
        lock(&self.0.servers)
            .iter()
            .filter_map(|live| match &live.state {
                ServerState::Ready { tools, .. } => Some(tools.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    pub fn tool_defs(&self) -> Vec<ToolDef> {
        self.tools()
            .into_iter()
            .map(|tool| {
                let description = if tool.description.is_empty() {
                    format!("The {} tool of an MCP server.", tool.tool)
                } else {
                    tool.description
                };
                ToolDef::new(&tool.name, &description, tool.schema)
            })
            .collect()
    }

    /// The server (its name) and tool behind an agent tool name.
    pub fn find(&self, name: &str) -> Option<(String, ToolInfo)> {
        lock(&self.0.servers)
            .iter()
            .find_map(|live| match &live.state {
                ServerState::Ready { tools, .. } => tools
                    .iter()
                    .find(|tool| tool.name == name)
                    .map(|tool| (live.config.name.clone(), tool.clone())),
                _ => None,
            })
    }

    /// Calls a tool by its agent name.
    pub async fn call(
        &self,
        name: &str,
        arguments: Value,
        cancel: &CancellationToken,
    ) -> Result<Called, String> {
        let (id, client, tool) = lock(&self.0.servers)
            .iter()
            .find_map(|live| match (&live.state, &live.client) {
                (ServerState::Ready { tools, .. }, Some(client)) => tools
                    .iter()
                    .find(|tool| tool.name == name)
                    .map(|tool| (live.config.id.clone(), client.clone(), tool.tool.clone())),
                _ => None,
            })
            .ok_or_else(|| {
                format!("The MCP tool {name} isn't available (its server isn't running).")
            })?;
        let arguments = if arguments.is_object() {
            arguments
        } else {
            json!({})
        };
        let result = client.call_tool(&tool, arguments, cancel).await;
        if client.is_closed() {
            // The server died: say so on the Settings page too.
            if let Some(live) = lock(&self.0.servers)
                .iter_mut()
                .find(|live| live.config.id == id)
            {
                live.state = ServerState::Failed {
                    error: format!(
                        "It stopped. {}",
                        client.log().last().cloned().unwrap_or_default()
                    )
                    .trim()
                    .to_owned(),
                };
            }
            self.changed();
        }
        let result = result?;
        Ok(Called {
            text: result_text(&result),
            is_error: result["isError"].as_bool().unwrap_or(false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_are_safe_and_short() {
        assert_eq!(
            tool_name("GitHub", "create_issue"),
            "mcp__github__create_issue"
        );
        assert_eq!(
            tool_name("my files!", "read file"),
            "mcp__my_files__read_file"
        );
        let long = tool_name("a-very-long-server-name-indeed", &"x".repeat(60));
        assert_eq!(long.len(), 64);
        assert_ne!(
            long,
            tool_name("a-very-long-server-name-indeed", &"y".repeat(60))
        );
    }

    #[test]
    fn claude_desktop_configs_import() {
        let configs = parse_import(
            r#"{ "mcpServers": {
                "files": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "C:\\Notes"] },
                "remote": { "url": "https://mcp.example.com/mcp", "headers": { "Authorization": "Bearer x" } },
                "off": { "command": "uvx", "args": ["thing"], "env": { "TOKEN": "t", "PORT": 1 }, "disabled": true }
            } }"#,
        )
        .unwrap();
        assert_eq!(configs.len(), 3);
        let files = configs.iter().find(|c| c.name == "files").unwrap();
        assert!(
            matches!(&files.transport, Transport::Stdio { command, args, .. }
            if command == "npx" && args.len() == 3)
        );
        let remote = configs.iter().find(|c| c.name == "remote").unwrap();
        assert!(matches!(&remote.transport, Transport::Http { headers, .. }
            if headers["Authorization"] == "Bearer x"));
        let off = configs.iter().find(|c| c.name == "off").unwrap();
        assert!(!off.enabled);
        assert!(matches!(&off.transport, Transport::Stdio { env, .. } if env["PORT"] == "1"));
        assert!(parse_import("{}").is_err());
        assert!(parse_import(r#"{ "x": { "nothing": 1 } }"#).is_err());
        assert!(parse_import("not json").is_err());
    }

    #[test]
    fn configs_round_trip_as_json() {
        let config = ServerConfig {
            id: "1".into(),
            name: "notes".into(),
            enabled: true,
            transport: Transport::Stdio {
                command: "node".into(),
                args: vec!["server.js".into()],
                env: BTreeMap::new(),
                cwd: None,
            },
        };
        let json = serde_json::to_value(&config).unwrap();
        assert_eq!(json["type"], "stdio");
        assert_eq!(json["command"], "node");
        assert_eq!(
            serde_json::from_value::<ServerConfig>(json).unwrap(),
            config
        );
    }

    #[test]
    fn results_read_as_text() {
        let result = json!({ "content": [
            { "type": "text", "text": "line one" },
            { "type": "image", "data": "AAAA", "mimeType": "image/png" },
            { "type": "resource", "resource": { "uri": "file:///a.txt", "text": "file text" } },
            { "type": "resource_link", "uri": "file:///b.txt", "name": "b.txt" }
        ] });
        assert_eq!(
            result_text(&result),
            "line one\n(image, image/png, not shown)\nfile text\nb.txt (file:///b.txt)"
        );
        assert_eq!(
            result_text(&json!({ "structuredContent": { "n": 3 } })),
            r#"{"n":3}"#
        );
        let long = json!({ "content": [{ "type": "text", "text": "x".repeat(50_000) }] });
        assert!(result_text(&long).contains("characters left out"));
    }

    #[test]
    fn servers_need_a_name_and_a_way_to_reach_them() {
        let stdio = |command: &str| ServerConfig {
            id: String::new(),
            name: "n".into(),
            enabled: true,
            transport: Transport::Stdio {
                command: command.into(),
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd: None,
            },
        };
        assert!(check(&stdio("node")).is_ok());
        assert!(check(&stdio(" ")).is_err());
        assert!(
            check(&ServerConfig {
                name: "!!".into(),
                ..stdio("node")
            })
            .is_err()
        );
    }
}
