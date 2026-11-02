//! Child processes. Every one runs in its own Windows Job Object and without a console window,
//! so stopping it (or a timeout, or Hearth closing) ends its whole process tree, and a GUI app
//! never flashes console windows.
//!
//! Children start suspended, join their job, and only then run, so not even a grandchild started
//! in the first instant can escape. This is the only place that creates processes (clippy bans
//! `Command::new` everywhere else).

use std::ffi::{OsStr, c_void};
use std::io;

use tokio::process::{Child, Command};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};
use windows::core::PCWSTR;

fn os_error(err: windows::core::Error) -> io::Error {
    io::Error::other(err.message())
}

/// A Windows Job Object. Dropping it ends every process still in it.
pub struct Job(HANDLE);

// SAFETY: a job handle is a kernel handle; any thread may use it.
unsafe impl Send for Job {}
// SAFETY: the job's functions are thread-safe.
unsafe impl Sync for Job {}

impl Job {
    pub fn new() -> io::Result<Self> {
        // SAFETY: plain Win32 call; no attributes, no name.
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(os_error)?;
        let job = Job(handle);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `info` is the structure this information class expects, alive for the call.
        unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast::<c_void>(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(os_error)?;
        Ok(job)
    }

    /// Ends every process in the job at once.
    pub fn kill(&self) {
        // SAFETY: the handle is ours and open.
        let _ = unsafe { TerminateJobObject(self.0, 1) };
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is ours; closing it kills what's left (KILL_ON_JOB_CLOSE).
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// A command that runs without a console window and starts suspended, for [`spawn`].
#[allow(clippy::disallowed_methods)]
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW.0 | CREATE_SUSPENDED.0);
    command.kill_on_drop(true);
    command
}

/// A running child and the job that holds its process tree.
pub struct Process {
    pub child: Child,
    pub job: Job,
}

/// Starts a command made by [`command`]: it joins a new job, then runs.
pub fn spawn(command: &mut Command) -> io::Result<Process> {
    let job = Job::new()?;
    let mut child = command.spawn()?;
    let joined = (|| {
        let handle = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("the process has no handle"))?;
        // SAFETY: both handles are open; the process handle belongs to `child`.
        unsafe { AssignProcessToJobObject(job.0, HANDLE(handle)) }.map_err(os_error)?;
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("the process has already ended"))?;
        resume(pid)
    })();
    if let Err(err) = joined {
        let _ = child.start_kill();
        return Err(err);
    }
    Ok(Process { child, job })
}

/// Resumes the threads of a process started suspended (just its first one, in practice).
fn resume(pid: u32) -> io::Result<()> {
    // SAFETY: a snapshot of all threads; closed below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.map_err(os_error)?;
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut resumed = 0;
    // SAFETY: `entry` is a valid THREADENTRY32 with its size set.
    let mut more = unsafe { Thread32First(snapshot, &mut entry) }.is_ok();
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: opening a thread of our own child to resume it; closed right after.
            if let Ok(thread) =
                unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) }
            {
                unsafe {
                    ResumeThread(thread);
                    let _ = CloseHandle(thread);
                }
                resumed += 1;
            }
        }
        // SAFETY: as for Thread32First.
        more = unsafe { Thread32Next(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: the snapshot handle is ours.
    let _ = unsafe { CloseHandle(snapshot) };
    if resumed == 0 {
        return Err(io::Error::other(
            "the process couldn't be started (no thread to resume)",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    use super::*;

    fn powershell(script: &str) -> Command {
        let mut command = command("powershell.exe");
        command
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    }

    #[tokio::test]
    async fn children_run_and_report_their_output() {
        let process = spawn(&mut powershell("Write-Output 'hello from the job'")).unwrap();
        let output = process.child.wait_with_output().await.unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("hello from the job"));
    }

    #[tokio::test]
    async fn killing_the_job_ends_the_whole_tree() {
        // A child that starts a grandchild and waits for it.
        let script = "$p = Start-Process -PassThru -WindowStyle Hidden powershell.exe '-NoProfile -Command Start-Sleep 60'; \
                      Write-Output $p.Id; Start-Sleep 60";
        let mut process = spawn(&mut powershell(script)).unwrap();
        let mut stdout = process.child.stdout.take().unwrap();
        let mut line = Vec::new();
        let started = Instant::now();
        while !line.contains(&b'\n') && started.elapsed() < Duration::from_secs(20) {
            let mut byte = [0u8; 1];
            if tokio::io::AsyncReadExt::read(&mut stdout, &mut byte)
                .await
                .unwrap()
                == 0
            {
                break;
            }
            line.push(byte[0]);
        }
        let grandchild: u32 = String::from_utf8_lossy(&line).trim().parse().unwrap();
        assert!(alive(grandchild), "the grandchild runs before the kill");

        process.job.kill();
        let status = tokio::time::timeout(Duration::from_secs(10), process.child.wait()).await;
        assert!(status.is_ok(), "the child ended");
        let deadline = Instant::now() + Duration::from_secs(10);
        while alive(grandchild) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(!alive(grandchild), "the grandchild ended too");
    }

    fn alive(pid: u32) -> bool {
        use windows::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        const STILL_ACTIVE: u32 = 259;
        // SAFETY: query-only handle, closed below.
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            return false;
        };
        let mut code = 0u32;
        // SAFETY: `code` outlives the call; the handle is open.
        let running =
            unsafe { GetExitCodeProcess(handle, &mut code) }.is_ok() && code == STILL_ACTIVE;
        // SAFETY: our handle.
        let _ = unsafe { CloseHandle(handle) };
        running
    }
}
