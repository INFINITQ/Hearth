//! Finding llama-server and learning what this build can do. Flags change between llama.cpp
//! releases, so Hearth reads `--help` and only passes flags the build lists. `--list-devices`
//! shows whether the GPU is used (a missing CUDA DLL means a silent CPU fallback).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde::Serialize;
use tokio::io::AsyncReadExt;

use crate::proc;

/// Windows' "a DLL was not found" exit status.
pub const STATUS_DLL_NOT_FOUND: i32 = 0xC000_0135_u32 as i32;
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// `CUDA0`, `Vulkan0`…
    pub id: String,
    pub name: String,
    pub total_mib: u64,
    pub free_mib: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub path: String,
    /// `build 7960 (a1b2c3d)`.
    pub version: Option<String>,
    pub devices: Vec<Device>,
    /// Problems worth telling the user: no GPU, a missing DLL.
    pub warnings: Vec<String>,
    /// The flags this build accepts (empty when `--help` couldn't be read).
    #[serde(skip)]
    pub flags: HashSet<String>,
}

static VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"version:\s*(\d+)\s*\(([0-9a-fA-F]+)\)").expect("valid regex"));
static DEVICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*([A-Za-z]+\d+):\s*(.+?)\s*\((\d+)\s*MiB,\s*(\d+)\s*MiB free\)")
        .expect("valid regex")
});
static FLAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s,])(-{1,2}[a-zA-Z][a-zA-Z0-9-]*)").expect("valid regex"));

/// llama-server.exe: the configured one, else on PATH, else in the usual folders.
pub fn find(configured: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = configured.map(str::trim).filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    let on_path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("llama-server.exe"))
        .find(|path| path.is_file());
    on_path.or_else(|| {
        let mut folders = vec![
            PathBuf::from("C:\\Tools\\llama.cpp"),
            PathBuf::from("C:\\llama.cpp"),
        ];
        for var in ["LOCALAPPDATA", "USERPROFILE", "ProgramFiles"] {
            if let Some(root) = std::env::var_os(var) {
                folders.push(PathBuf::from(root).join("llama.cpp"));
            }
        }
        folders
            .into_iter()
            .map(|dir| dir.join("llama-server.exe"))
            .find(|path| path.is_file())
    })
}

/// Runs llama-server with `args` and returns its exit code and everything it printed.
async fn run(path: &Path, args: &[&str]) -> Result<(Option<i32>, String), String> {
    let mut command = proc::command(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process =
        proc::spawn(&mut command).map_err(|e| format!("llama-server couldn't start: {e}"))?;
    let mut stdout = process.child.stdout.take();
    let mut stderr = process.child.stderr.take();
    let read = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(stdout) = stdout.as_mut() {
            let _ = stdout.read_to_end(&mut out).await;
        }
        if let Some(stderr) = stderr.as_mut() {
            let _ = stderr.read_to_end(&mut err).await;
        }
        let status = process.child.wait().await.ok();
        (status.and_then(|s| s.code()), [out, err].concat())
    };
    match tokio::time::timeout(PROBE_TIMEOUT, read).await {
        Ok((code, bytes)) => Ok((code, String::from_utf8_lossy(&bytes).into_owned())),
        Err(_) => {
            process.job.kill();
            Err("llama-server didn't answer in time.".into())
        }
    }
}

pub fn parse_version(text: &str) -> Option<String> {
    VERSION
        .captures(text)
        .map(|found| format!("build {} ({})", &found[1], &found[2]))
}

pub fn parse_devices(text: &str) -> Vec<Device> {
    DEVICE
        .captures_iter(text)
        .map(|found| Device {
            id: found[1].to_owned(),
            name: found[2].to_owned(),
            total_mib: found[3].parse().unwrap_or(0),
            free_mib: found[4].parse().unwrap_or(0),
        })
        .collect()
}

pub fn parse_flags(text: &str) -> HashSet<String> {
    FLAG.captures_iter(text)
        .map(|found| found[1].to_owned())
        .collect()
}

/// What this llama-server is and can do.
pub async fn probe(path: &Path) -> Result<ServerInfo, String> {
    let mut info = ServerInfo {
        path: path.display().to_string(),
        ..ServerInfo::default()
    };
    let (code, version) = run(path, &["--version"]).await?;
    if code == Some(STATUS_DLL_NOT_FOUND) {
        return Err(
            "llama-server can't start because a DLL is missing. Extract the cudart-llama-bin-win-cuda zip from the \
             same llama.cpp release into the same folder as llama-server.exe."
                .into(),
        );
    }
    info.version = parse_version(&version);
    if info.version.is_none() {
        return Err(format!(
            "{} doesn't look like llama-server (it printed no version).",
            path.display()
        ));
    }
    if let Ok((_, devices)) = run(path, &["--list-devices"]).await {
        info.devices = parse_devices(&devices);
    }
    if !info
        .devices
        .iter()
        .any(|d| d.id.starts_with("CUDA") || d.id.starts_with("Vulkan"))
    {
        info.warnings.push(
            "llama-server found no GPU, so models will run on the CPU (slowly). Use the CUDA build of llama.cpp \
             and check that both zips were extracted into one folder."
                .into(),
        );
    }
    if let Ok((_, help)) = run(path, &["--help"]).await {
        info.flags = parse_flags(&help);
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_version_devices_and_flags() {
        let version = "version: 7960 (3f2a1bc)\nbuilt with MSVC 19.44.35217.0 for x64";
        assert_eq!(
            parse_version(version).as_deref(),
            Some("build 7960 (3f2a1bc)")
        );
        let devices = "ggml_cuda_init: found 1 CUDA devices:\nAvailable devices:\n  CUDA0: NVIDIA GeForce RTX 5070 (12227 MiB, 11043 MiB free)\n";
        assert_eq!(
            parse_devices(devices),
            [Device {
                id: "CUDA0".into(),
                name: "NVIDIA GeForce RTX 5070".into(),
                total_mib: 12227,
                free_mib: 11043
            }]
        );
        let help = "-m,    --model FNAME\n-c,    --ctx-size N\n--n-cpu-moe N\n-ctk, --cache-type-k TYPE\n--no-webui";
        let flags = parse_flags(help);
        for flag in [
            "-m",
            "--model",
            "-c",
            "--ctx-size",
            "--n-cpu-moe",
            "-ctk",
            "--cache-type-k",
            "--no-webui",
        ] {
            assert!(flags.contains(flag), "{flag}");
        }
        assert_eq!(STATUS_DLL_NOT_FOUND, -1_073_741_515);
    }
}
