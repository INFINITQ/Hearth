//! A pseudo console (ConPTY) with a shell in it.
//!
//! The shell starts suspended, joins its own Job Object and only then runs, like every other
//! child (see `proc.rs`), so closing a terminal ends everything started in it.

use std::ffi::{OsStr, OsString, c_void};
use std::fs::File;
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, MutexGuard};

use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION,
    ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use crate::proc::{Job, os_error};

/// What to run in a pseudo console.
pub struct Spawn<'a> {
    pub program: &'a Path,
    pub args: &'a [&'a str],
    pub cwd: &'a Path,
    /// Added to Hearth's own environment (replacing variables of the same name).
    pub env: &'a [(&'a str, &'a str)],
    pub cols: u16,
    pub rows: u16,
}

/// A shell in a pseudo console. [`Pty::kill`] (or dropping it) ends the shell and everything it
/// started.
pub struct Pty {
    console: Mutex<Option<HPCON>>,
    /// Keystrokes, written by a thread of their own so a busy shell never blocks the caller.
    input: Mutex<Sender<Vec<u8>>>,
    process: HANDLE,
    pid: u32,
    job: Job,
}

// SAFETY: the process handle is a kernel handle any thread may use; the console handle is only
// used under its mutex.
unsafe impl Send for Pty {}
// SAFETY: as above; every method takes `&self` and the mutable parts are behind mutexes.
unsafe impl Sync for Pty {}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A handle that's closed when dropped, for the setup steps that can fail.
struct Owned(HANDLE);

impl Owned {
    fn take(mut self) -> HANDLE {
        std::mem::take(&mut self.0)
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: the handle is ours and still open.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

fn size(cols: u16, rows: u16) -> COORD {
    COORD {
        X: cols.clamp(2, 1000) as i16,
        Y: rows.clamp(1, 500) as i16,
    }
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

/// Adds an argument to a command line, quoted the way programs built with the C runtime read it.
fn push_arg(line: &mut Vec<u16>, arg: &OsStr) {
    const SPACE: u16 = b' ' as u16;
    const TAB: u16 = b'\t' as u16;
    const QUOTE: u16 = b'"' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    let arg: Vec<u16> = arg.encode_wide().collect();
    if !line.is_empty() {
        line.push(SPACE);
    }
    if !arg.is_empty() && !arg.iter().any(|&c| c == SPACE || c == TAB || c == QUOTE) {
        line.extend(arg);
        return;
    }
    line.push(QUOTE);
    let mut backslashes = 0;
    for c in arg {
        if c == BACKSLASH {
            backslashes += 1;
        } else {
            if c == QUOTE {
                line.extend(std::iter::repeat_n(BACKSLASH, backslashes + 1));
            }
            backslashes = 0;
        }
        line.push(c);
    }
    line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
    line.push(QUOTE);
}

/// Hearth's environment plus `extra`, as the sorted block CreateProcessW takes.
fn environment(extra: &[(&str, &str)]) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(key, _)| !extra.iter().any(|(name, _)| key.eq_ignore_ascii_case(name)))
        .collect();
    vars.extend(
        extra
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value))),
    );
    vars.sort_by_key(|(key, _)| key.to_ascii_uppercase());
    let mut block = Vec::new();
    for (key, value) in vars {
        block.extend(key.encode_wide());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

/// Starts the program, suspended, attached to `console`.
fn create_process(console: HPCON, spec: &Spawn<'_>) -> io::Result<PROCESS_INFORMATION> {
    let mut bytes = 0usize;
    // SAFETY: asks how big a one-attribute list is (this call "fails" with the size).
    let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut bytes) };
    // `usize`s, so the list is pointer-aligned.
    let mut buffer = vec![0usize; bytes.div_ceil(size_of::<usize>()).max(1)];
    let list = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr().cast());
    // SAFETY: `buffer` holds at least `bytes` bytes and outlives the list.
    unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut bytes) }
        .map_err(os_error)?;
    let created = (|| {
        // SAFETY: the pseudo console attribute takes the console handle itself as its value.
        unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                Some(console.0 as *const c_void),
                size_of::<HPCON>(),
                None,
                None,
            )
        }
        .map_err(os_error)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        // Without this, a shell can inherit Hearth's own redirected output instead of the console.
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
        startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
        startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
        startup.lpAttributeList = list;
        let mut line = Vec::new();
        push_arg(&mut line, spec.program.as_os_str());
        for arg in spec.args {
            push_arg(&mut line, OsStr::new(arg));
        }
        line.push(0);
        let env = environment(spec.env);
        let cwd = wide(spec.cwd.as_os_str());
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: every buffer is nul-terminated and alive for the call; `startup` is a
        // STARTUPINFOEXW, as EXTENDED_STARTUPINFO_PRESENT says.
        unsafe {
            CreateProcessW(
                PCWSTR::null(),
                Some(PWSTR(line.as_mut_ptr())),
                None,
                None,
                false,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
                Some(env.as_ptr().cast()),
                PCWSTR(cwd.as_ptr()),
                &startup.StartupInfo,
                &mut info,
            )
        }
        .map_err(os_error)?;
        Ok(info)
    })();
    // SAFETY: the list was initialized above.
    unsafe { DeleteProcThreadAttributeList(list) };
    created
}

impl Pty {
    /// Starts a program in a new pseudo console. Returns the console's output, which ends once
    /// the console is closed (see [`Pty::close_console`]).
    pub fn spawn(spec: &Spawn<'_>) -> io::Result<(Pty, File)> {
        let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
        // SAFETY: out-pointers to local handles.
        unsafe { CreatePipe(&mut read, &mut write, None, 0) }.map_err(os_error)?;
        let (input_read, input_write) = (Owned(read), Owned(write));
        // SAFETY: as above.
        unsafe { CreatePipe(&mut read, &mut write, None, 0) }.map_err(os_error)?;
        let (output_read, output_write) = (Owned(read), Owned(write));
        // SAFETY: the pipe ends are open; the console keeps its own copies.
        let console = unsafe {
            CreatePseudoConsole(size(spec.cols, spec.rows), input_read.0, output_write.0, 0)
        }
        .map_err(os_error)?;
        drop((input_read, output_write));

        let started = (|| {
            let job = Job::new()?;
            let info = create_process(console, spec)?;
            let process = Owned(info.hProcess);
            let thread = Owned(info.hThread);
            if let Err(err) = job.assign(process.0) {
                // SAFETY: the process is ours and hasn't run yet.
                let _ = unsafe { TerminateProcess(process.0, 1) };
                return Err(err);
            }
            // SAFETY: the thread handle is open; the process now runs inside its job.
            unsafe { ResumeThread(thread.0) };
            Ok((job, process.take(), info.dwProcessId))
        })();
        let (job, process, pid) = match started {
            Ok(started) => started,
            Err(err) => {
                // SAFETY: the console is ours and no longer needed.
                unsafe { ClosePseudoConsole(console) };
                return Err(err);
            }
        };

        // SAFETY: these pipe ends are ours; the files take them over.
        let output = unsafe { File::from_raw_handle(output_read.take().0) };
        // SAFETY: as above.
        let mut input = unsafe { File::from_raw_handle(input_write.take().0) };
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        // Owned before anything else can fail, so dropping it cleans up (Drop below).
        let pty = Pty {
            console: Mutex::new(Some(console)),
            input: Mutex::new(sender),
            process,
            pid,
            job,
        };
        std::thread::Builder::new()
            .name("terminal-input".into())
            .spawn(move || {
                for bytes in receiver {
                    if input
                        .write_all(&bytes)
                        .and_then(|()| input.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok((pty, output))
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Sends keystrokes (or pasted text) to the shell.
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        lock(&self.input)
            .send(bytes.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the terminal has closed"))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        if let Some(console) = *lock(&self.console) {
            // SAFETY: the console is open (it's taken out of the mutex when closed).
            unsafe { ResizePseudoConsole(console, size(cols, rows)) }.map_err(os_error)?;
        }
        Ok(())
    }

    /// Waits for the shell to end; its exit code.
    pub fn wait(&self) -> Option<i64> {
        // SAFETY: the process handle stays open while `self` lives.
        unsafe { WaitForSingleObject(self.process, INFINITE) };
        let mut code = 0u32;
        // SAFETY: as above.
        unsafe { GetExitCodeProcess(self.process, &mut code) }.ok()?;
        Some(i64::from(code))
    }

    /// Closes the console. Its output ends after whatever was still in it.
    pub fn close_console(&self) {
        // Taken out first: closing can wait for the console, and nothing else should.
        let console = lock(&self.console).take();
        if let Some(console) = console {
            // SAFETY: taken out of the mutex, so it's closed exactly once.
            unsafe { ClosePseudoConsole(console) };
        }
    }

    /// Ends the shell and everything it started, and closes the console.
    pub fn kill(&self) {
        self.job.kill();
        self.close_console();
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.kill();
        // SAFETY: the handle is ours; nothing uses it after this.
        let _ = unsafe { CloseHandle(self.process) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(args: &[&str]) -> String {
        let mut line = Vec::new();
        for arg in args {
            push_arg(&mut line, OsStr::new(arg));
        }
        String::from_utf16(&line).unwrap()
    }

    #[test]
    fn arguments_are_quoted_only_when_needed() {
        assert_eq!(line(&["bash.exe", "--login", "-i"]), "bash.exe --login -i");
        assert_eq!(
            line(&[r"C:\Program Files\Git\bin\bash.exe", "-i"]),
            r#""C:\Program Files\Git\bin\bash.exe" -i"#
        );
        assert_eq!(line(&["say", r#"a "b""#]), r#"say "a \"b\"""#);
        assert_eq!(line(&["dir", r"C:\a b\"]), r#"dir "C:\a b\\""#);
        assert_eq!(line(&["x", ""]), r#"x """#);
    }

    #[test]
    fn the_environment_block_is_sorted_and_overrides_win() {
        let block = environment(&[("TERM", "xterm-256color"), ("ZZ_HEARTH_TEST", "1")]);
        let text = String::from_utf16_lossy(&block);
        let vars: Vec<&str> = text.split('\0').filter(|v| !v.is_empty()).collect();
        assert!(vars.contains(&"TERM=xterm-256color"));
        assert_eq!(
            vars.iter()
                .filter(|v| v.to_uppercase().starts_with("TERM="))
                .count(),
            1
        );
        let keys: Vec<String> = vars
            .iter()
            .map(|v| v[..v[1..].find('=').map_or(v.len(), |i| i + 1)].to_uppercase())
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        assert!(block.ends_with(&[0, 0]));
    }
}
