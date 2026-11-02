//! How much GPU memory and RAM is in use, for the Models page. GPU numbers come from
//! `nvidia-smi` (installed with NVIDIA's driver); without it only RAM is shown.

use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::io::AsyncReadExt;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

use crate::proc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub name: String,
    pub total_mib: u64,
    pub used_mib: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry {
    pub gpus: Vec<Gpu>,
    pub ram_total_mib: u64,
    pub ram_used_mib: u64,
}

fn ram() -> (u64, u64) {
    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `status` is a valid MEMORYSTATUSEX with its length set.
    if unsafe { GlobalMemoryStatusEx(&mut status) }.is_err() {
        return (0, 0);
    }
    let mib = |bytes: u64| bytes / (1024 * 1024);
    (
        mib(status.ullTotalPhys),
        mib(status.ullTotalPhys - status.ullAvailPhys),
    )
}

pub fn parse_gpus(csv: &str) -> Vec<Gpu> {
    csv.lines()
        .filter_map(|line| {
            let mut parts = line.split(',').map(str::trim);
            let name = parts.next()?.to_owned();
            let total_mib = parts.next()?.parse().ok()?;
            let used_mib = parts.next()?.parse().ok()?;
            (!name.is_empty()).then_some(Gpu {
                name,
                total_mib,
                used_mib,
            })
        })
        .collect()
}

async fn gpus() -> Vec<Gpu> {
    let mut command = proc::command("nvidia-smi.exe");
    command
        .args([
            "--query-gpu=name,memory.total,memory.used",
            "--format=csv,noheader,nounits",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let Ok(mut process) = proc::spawn(&mut command) else {
        return Vec::new();
    };
    let mut out = String::new();
    let read = async {
        if let Some(stdout) = process.child.stdout.as_mut() {
            let _ = stdout.read_to_string(&mut out).await;
        }
        let _ = process.child.wait().await;
    };
    if tokio::time::timeout(Duration::from_secs(3), read)
        .await
        .is_err()
    {
        process.job.kill();
        return Vec::new();
    }
    parse_gpus(&out)
}

pub async fn read() -> Telemetry {
    let (ram_total_mib, ram_used_mib) = ram();
    Telemetry {
        gpus: gpus().await,
        ram_total_mib,
        ram_used_mib,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_smi_lines_are_read() {
        assert_eq!(
            parse_gpus("NVIDIA GeForce RTX 5070, 12227, 911\n\nbroken line\n"),
            [Gpu {
                name: "NVIDIA GeForce RTX 5070".into(),
                total_mib: 12227,
                used_mib: 911
            }]
        );
    }

    #[tokio::test]
    async fn this_pc_has_ram() {
        let telemetry = read().await;
        assert!(
            telemetry.ram_total_mib > 1000 && telemetry.ram_used_mib <= telemetry.ram_total_mib
        );
    }
}
