//! Shells for the agent's commands, and turning their output into text a model can read.
//!
//! Git Bash is the default: local models know bash best, and Git for Windows is on most
//! developers' PCs. Windows PowerShell (or PowerShell 7 when installed) is the alternative.
//! Commands never go through `-c "<string>"`: bash gets a temporary script file, PowerShell an
//! encoded command, so no quoting rules can mangle them.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use windows::Win32::Globalization::{CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::w;

use super::paths::to_posix;
use crate::{new_id, proc};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellKind {
    #[default]
    GitBash,
    #[serde(rename = "powershell")]
    PowerShell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    pub kind: ShellKind,
    pub path: PathBuf,
}

/// Variables that keep tools from waiting for input nobody will type.
const ENV: &[(&str, &str)] = &[
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GCM_INTERACTIVE", "never"),
    ("GIT_EDITOR", "true"),
    ("GIT_SEQUENCE_EDITOR", "true"),
    ("TERM", "dumb"),
    ("npm_config_yes", "true"),
    ("PYTHONUTF8", "1"),
    ("PYTHONIOENCODING", "utf-8"),
];

/// PowerShell commands longer than this go into a script file.
const MAX_ENCODED: usize = 8000;

/// A script file that's deleted when the command is done with it.
pub struct Script(PathBuf);

impl Drop for Script {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn git_bash() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    // Git for Windows records where it's installed.
    let mut buffer = [0u16; 520];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the buffer and its size in bytes are valid for the call.
    let found = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\GitForWindows"),
            w!("InstallPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if found.is_ok() {
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        candidates
            .push(PathBuf::from(String::from_utf16_lossy(&buffer[..len])).join("bin\\bash.exe"));
    }
    // Next to git.exe on PATH (Git\cmd\git.exe, Git\bin\git.exe or Git\mingw64\bin\git.exe).
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        if dir.join("git.exe").is_file() {
            for up in [&dir.join("..\\bin"), &dir, &dir.join("..\\..\\bin")] {
                candidates.push(up.join("bash.exe"));
            }
        }
    }
    for base in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(base) {
            let root = PathBuf::from(root);
            candidates.push(root.join("Git\\bin\\bash.exe"));
            candidates.push(root.join("Programs\\Git\\bin\\bash.exe"));
        }
    }
    candidates
        .into_iter()
        .filter(|path| !path.to_string_lossy().to_lowercase().contains("system32"))
        .find(|path| path.is_file())
        .and_then(|path| dunce::canonicalize(path).ok())
}

fn powershell() -> Option<PathBuf> {
    let on_path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("pwsh.exe"))
        .find(|path| path.is_file());
    on_path.or_else(|| {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let path = PathBuf::from(root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe");
        path.is_file().then_some(path)
    })
}

impl Shell {
    /// The shell of this kind on this PC, if there is one.
    pub fn find(kind: ShellKind) -> Option<Shell> {
        let path = match kind {
            ShellKind::GitBash => git_bash(),
            ShellKind::PowerShell => powershell(),
        }?;
        Some(Shell { kind, path })
    }

    /// The preferred shell, or the other one if it's missing.
    pub fn choose(preferred: ShellKind) -> Option<Shell> {
        let other = match preferred {
            ShellKind::GitBash => ShellKind::PowerShell,
            ShellKind::PowerShell => ShellKind::GitBash,
        };
        Shell::find(preferred).or_else(|| Shell::find(other))
    }

    pub fn label(&self) -> &'static str {
        match self.kind {
            ShellKind::GitBash => "Git Bash",
            ShellKind::PowerShell if self.path.ends_with("pwsh.exe") => "PowerShell 7",
            ShellKind::PowerShell => "Windows PowerShell 5.1",
        }
    }

    /// A command that runs `script` in `cwd` (not yet started), and the temporary file it needs.
    pub fn command(&self, script: &str, cwd: &Path) -> io::Result<(Command, Option<Script>)> {
        let (mut command, file) = match self.kind {
            ShellKind::GitBash => {
                let path = std::env::temp_dir().join(format!("hearth-{}.sh", new_id()));
                let body = format!("exec 2>&1\n{}\n", script.replace("\r\n", "\n"));
                std::fs::write(&path, body)?;
                let mut command = proc::command(&self.path);
                // A login shell sets up PATH; CHERE_INVOKING keeps it in `cwd`.
                command
                    .arg("-l")
                    .arg(to_posix(&path))
                    .env("CHERE_INVOKING", "1");
                (command, Some(Script(path)))
            }
            ShellKind::PowerShell => {
                let full = format!(
                    "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);$ProgressPreference='SilentlyContinue';\n{script}"
                );
                let mut command = proc::command(&self.path);
                command.args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                ]);
                if full.len() > MAX_ENCODED {
                    let path = std::env::temp_dir().join(format!("hearth-{}.ps1", new_id()));
                    // With a BOM, so Windows PowerShell reads it as UTF-8.
                    std::fs::write(&path, format!("\u{feff}{full}"))?;
                    command.arg("-File").arg(&path);
                    (command, Some(Script(path)))
                } else {
                    let utf16: Vec<u8> = full.encode_utf16().flat_map(u16::to_le_bytes).collect();
                    command.arg("-EncodedCommand").arg(base64(&utf16));
                    (command, None)
                }
            }
        };
        command
            .current_dir(cwd)
            .envs(ENV.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok((command, file))
    }
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Bytes from a console program as text: UTF-8 when it is, else the OEM code page that
/// `cmd.exe`-era programs still write.
pub fn decode(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    // SAFETY: first call measures; the second fills a buffer of exactly that size.
    let len =
        unsafe { MultiByteToWideChar(CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None) };
    if len <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut wide = vec![0u16; len as usize];
    let written = unsafe {
        MultiByteToWideChar(
            CP_OEMCP,
            MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0),
            bytes,
            Some(&mut wide),
        )
    };
    String::from_utf16_lossy(&wide[..written.max(0) as usize])
}

static ANSI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")
        .expect("valid regex")
});

/// Output as a model should see it: no colour codes, progress bars collapsed to their last
/// state, `\n` line endings.
pub fn clean(text: &str) -> String {
    let stripped = ANSI.replace_all(text, "");
    let lines: Vec<&str> = stripped
        .split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            // A `\r` inside a line means the program redrew it: keep what was drawn last.
            line.rsplit('\r')
                .find(|part| !part.is_empty())
                .unwrap_or("")
        })
        .collect();
    lines.join("\n").trim_end().to_owned()
}

/// The first and last lines of a long output, with a note about what's left out.
pub fn head_and_tail(text: &str, head: usize, tail: usize) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() <= head + tail {
        return text.to_owned();
    }
    let skipped = lines.len() - head - tail;
    format!(
        "{}\n… ({skipped} lines left out) …\n{}",
        lines[..head].join("\n"),
        lines[lines.len() - tail..].join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn output_is_cleaned_for_the_model() {
        let raw = "\x1b[32mPASS\x1b[0m test_add\r\nDownloading 10%\rDownloading 55%\rDownloading 100%\n\x1b]0;title\x07done\n\n";
        assert_eq!(clean(raw), "PASS test_add\nDownloading 100%\ndone");
        let long: String = (1..=10)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            head_and_tail(&long, 2, 2),
            "line 1\nline 2\n… (6 lines left out) …\nline 9\nline 10"
        );
        assert_eq!(head_and_tail("a\nb", 2, 2), "a\nb");
    }

    #[test]
    fn oem_output_is_decoded() {
        assert_eq!(decode("héllo".as_bytes()), "héllo");
        // Not UTF-8: decoded with the OEM code page instead of failing.
        assert!(!decode(&[0x82, b'a']).is_empty());
    }

    #[tokio::test]
    async fn both_shells_run_a_script_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("marker.txt"), "x").unwrap();
        for (kind, script) in [
            (
                ShellKind::GitBash,
                "ls; echo \"sum=$((2+3))\"; echo oops >&2",
            ),
            (
                ShellKind::PowerShell,
                "Get-ChildItem -Name; Write-Output \"sum=$(2+3)\"; [Console]::Error.WriteLine('oops')",
            ),
        ] {
            let Some(shell) = Shell::find(kind) else {
                eprintln!("{kind:?} isn't installed here; skipped");
                continue;
            };
            let (mut command, _script) = shell.command(script, dir.path()).unwrap();
            let process = proc::spawn(&mut command).unwrap();
            let output = process.child.wait_with_output().await.unwrap();
            let text = clean(&decode(&[output.stdout, output.stderr].concat()));
            assert!(
                text.contains("marker.txt") && text.contains("sum=5") && text.contains("oops"),
                "{kind:?}: {text}"
            );
        }
    }
}
