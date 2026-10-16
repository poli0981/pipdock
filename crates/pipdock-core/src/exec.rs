//! Subprocess execution — the single chokepoint for every external command.
//!
//! Two invariants live here rather than in the callers, so they cannot be forgotten one call site
//! at a time:
//!
//! 1. **argv arrays, never a shell** (SECURITY §2). [`Command`] takes a program and a `Vec<String>`
//!    and has no path that accepts a command line, so quoting and injection classes are
//!    structurally absent.
//! 2. **pip always runs with UTF-8 stdio** (spike SP-2). `pip install --dry-run --report -`
//!    crashes with `UnicodeEncodeError` on Windows under cp1252 — confirmed on pip 25.0.1 and
//!    26.1.2 — because pip writes the report through its vendored `rich` and the legacy Windows
//!    console codec. It is data-dependent: an all-ASCII report succeeds, so this would pass
//!    testing and fail on users' machines. [`Command::python`] sets the mitigation unconditionally.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command as TokioCommand;
use tokio_util::sync::CancellationToken;

use crate::engine::{EventSink, ProgressEvent, ProgressSink, Stream};
use crate::errors::{Code, PdError, Result};
use crate::model::{ExecMode, PkgName};

/// Default watchdog, for anything that does not name its own in [`watchdog`].
pub const DEFAULT_TIMEOUT: Duration = watchdog::RESOLVE;

/// Watchdogs by kind of work.
///
/// One 600 s budget used to cover everything, which was wrong in both directions: a hung `py -0p`
/// held the Environments screen for ten minutes, and a source build of scipy -- legitimately slow
/// -- was killed at the same ten. A watchdog is a unit, not a number (CLAUDE.md, from P1-1): each
/// constant here is sized for one kind of operation.
pub mod watchdog {
    use std::time::Duration;

    /// `pip install` / `uv pip install` / uninstall. Building an sdist can take many minutes.
    pub const INSTALL: Duration = Duration::from_secs(30 * 60);
    /// A dry-run resolve, including pip's backtracking on a large environment.
    pub const RESOLVE: Duration = Duration::from_secs(10 * 60);
    /// `list --outdated`, which makes one index request per installed package.
    pub const OUTDATED: Duration = Duration::from_secs(10 * 60);
    /// Local queries: `list`, `freeze`, `check`, `--version`.
    pub const QUERY: Duration = Duration::from_secs(3 * 60);
    /// `probe.py` against one interpreter. It reads metadata files and nothing else.
    pub const PROBE: Duration = Duration::from_secs(60);
    /// Discovery listings: `py -0p`, `uv python list`.
    pub const DISCOVERY: Duration = Duration::from_secs(30);
}

/// Why a run ended before its program did.
///
/// Carried on the [`PdError`] such a run returns ([`PdError::stop`]), so a caller asks a typed
/// question. Three modules used to read the *message* back instead -- `"cancelled:"`, `"timed
/// out"` -- each with its own copy of the prefixes `Command::stopped` happened to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The command's [`CancellationToken`] was tripped.
    Cancelled,
    /// The watchdog fired after this long.
    TimedOut(Duration),
}

/// Wait for cancellation, or never, when no token was supplied.
///
/// `select!` needs a future in every arm. Without a token the arm has to be one that never
/// resolves — returning immediately would abort every command that did not ask to be cancellable.
async fn cancelled(token: Option<&CancellationToken>) {
    match token {
        Some(t) => t.cancelled().await,
        None => std::future::pending().await,
    }
}

/// What a finished command produced.
#[derive(Debug, Clone)]
pub struct Output {
    /// Exit status, `None` if the process was killed by a signal or the watchdog.
    pub code: Option<i32>,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr. For uv this carries the **plan**, not just errors (SP-1).
    pub stderr: String,
}

impl Output {
    /// True when the process exited zero.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// Turn a non-zero exit into a classified [`PdError`].
    ///
    /// # Errors
    /// Always returns an error; call only after checking [`Output::ok`].
    pub fn into_error(self) -> PdError {
        PdError::from_engine_stderr(&self.stderr)
    }
}

/// A subprocess invocation.
///
/// Deliberately minimal: there is no `arg_line`, no shell, and no way to pass a string that gets
/// split. If you find yourself wanting one, the answer is another `arg()`.
#[derive(Debug, Clone)]
pub struct Command {
    program: PathBuf,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: HashMap<String, String>,
    timeout: Duration,
    cancel: Option<CancellationToken>,
    /// The program is a Python interpreter, so "not found" means the environment is gone
    /// (`PD-ENV-001`) rather than that an engine needs installing (`PD-ENG-001`).
    interpreter: bool,
}

impl Command {
    /// Invoke an arbitrary program.
    #[must_use]
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: HashMap::new(),
            timeout: DEFAULT_TIMEOUT,
            cancel: None,
            interpreter: false,
        }
    }

    /// Invoke a Python interpreter with the SP-2 encoding mitigation already applied.
    ///
    /// Use this for **every** `python -m pip …` and `probe.py` call. `PYTHONUTF8=1` covers the
    /// interpreter's own text handling and `PYTHONIOENCODING=utf-8` covers the stdio streams pip's
    /// vendored `rich` writes through.
    ///
    /// **Not enough on its own for a `-I` run.** Isolated mode implies `-E`, which ignores every
    /// `PYTHON*` variable, so `probe.py` must also pass `-X utf8` (see `envs::probe`).
    #[must_use]
    pub fn python(interpreter: impl Into<PathBuf>) -> Self {
        let mut cmd = Self::new(interpreter)
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1");
        cmd.interpreter = true;
        cmd
    }

    /// Invoke a Python **console script** -- vulture, deptry, pip-audit -- with the same mitigation.
    ///
    /// Their `.exe` is a launcher for the tools venv's interpreter, so the variables reach Python
    /// exactly as they do for [`Command::python`]. Spawned with [`Command::new`] they were missing:
    /// a non-ASCII file name in the user's project could crash vulture's report on cp1252 and read
    /// as PD-HLT-002, "the tool stopped with an error".
    #[must_use]
    pub fn python_tool(exe: impl Into<PathBuf>) -> Self {
        Self::new(exe)
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1")
    }

    /// Append one argument. One call, one argv entry — never a space-separated string.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Append several arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set the working directory. Code Health runs its tools with the project folder as CWD.
    #[must_use]
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Set an environment variable for the child.
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Override the watchdog.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Abort when `token` is cancelled (ARCHITECTURE §7's `plan_cancel`).
    ///
    /// Without one the command still honours the watchdog; the token just adds a second reason
    /// to stop early, and both take the same path out.
    #[must_use]
    pub fn cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }

    /// Why a run ended early. Both arms kill the child on the way out.
    ///
    /// The message keeps its old wording for the log, but nothing may read it: the typed answer is
    /// [`PdError::stop`]. A cancellation keeps `PD-INT-001` because it is never meant to reach a
    /// user -- every caller turns a cancelled step into a *skipped* one -- while a timeout is
    /// `PD-ENG-004`, which callers on other paths (Code Health, the audit) remap to their own code.
    fn stopped(&self, stop: Stop) -> PdError {
        tracing::warn!(target: "pipdock::exec", program = %self.program.display(), ?stop, "stopped");
        let program = self.program.display();
        match stop {
            Stop::Cancelled => PdError::new(Code::IntUnexpected, format!("cancelled: {program}")),
            Stop::TimedOut(after) => PdError::new(
                Code::EngTimedOut,
                format!("timed out after {after:?}: {program}"),
            ),
        }
        .with_stop(stop)
    }

    /// Map a failure to *start* the program onto the code whose advice fits.
    ///
    /// Everything used to be `PD-ENG-001`, "install the engine": for a deleted venv's
    /// `python.exe` that should be `PD-ENV-001`, and for a command line past Windows' 32,767
    /// characters (`ERROR_FILENAME_EXCED_RANGE`, 206) it is a path-length problem.
    fn spawn_error(&self, e: &std::io::Error) -> PdError {
        const ERROR_FILENAME_EXCED_RANGE: i32 = 206;
        let program = self.program.display();
        let code = if e.raw_os_error() == Some(ERROR_FILENAME_EXCED_RANGE) {
            Code::SysPathTooLong
        } else if e.kind() == std::io::ErrorKind::NotFound && self.interpreter {
            Code::EnvInterpreterMissing
        } else {
            Code::EngNotFound
        };
        tracing::warn!(target: "pipdock::exec", %program, error = %e, "could not start");
        PdError::new(code, format!("could not run {program}: {e}"))
    }

    /// The argv this command would run, for logging and for the bug-report ring buffer.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        let mut out = vec![self.program.display().to_string()];
        out.extend(self.args.iter().cloned());
        out
    }

    fn build(&self) -> TokioCommand {
        let mut cmd = TokioCommand::new(&self.program);
        cmd.args(&self.args);
        if let Some(dir) = &self.cwd {
            cmd.current_dir(dir);
        }
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        #[cfg(windows)]
        {
            // Do not flash a console window when the GUI spawns an engine.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        // Both stop paths below work by dropping the future that owns the child, and a tokio
        // `Child` does *not* kill on drop by default. Without this the 600 s watchdog left pip
        // running: the future went away, the process did not.
        //
        // This reaches the direct child. Everything it spawned is [`TreeGuard`]'s job.
        cmd.kill_on_drop(true);
        cmd
    }

    /// Run to completion, capturing output. See [`TreeGuard`] for what "stop" means here.
    ///
    /// # Errors
    /// `PD-ENG-001` when the program cannot be spawned, `PD-ENG-004` when the watchdog fires.
    pub async fn run(&self) -> Result<Output> {
        let started = self.logged_start();
        // Spawned rather than `.output()`ed, so the child can be put in a job before it is waited
        // on. `.output()` owns the child and never lends out a handle.
        let child = self.build().spawn().map_err(|e| self.spawn_error(&e))?;
        let tree = TreeGuard::attach(&child);

        let out = tokio::select! {
            out = child.wait_with_output() => out,
            () = tokio::time::sleep(self.timeout) => {
                return Err(self.stopped(Stop::TimedOut(self.timeout)));
            }
            () = cancelled(self.cancel.as_ref()) => {
                return Err(self.stopped(Stop::Cancelled));
            }
        };
        drop(tree);
        let out = out.map_err(|e| {
            PdError::new(
                Code::IntUnexpected,
                format!("waiting on {} failed: {e}", self.program.display()),
            )
        })?;
        self.logged_exit(started, out.status.code());
        Ok(Output {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// The argv, at INFO: what PipDock ran is the first thing a bug report needs.
    fn logged_start(&self) -> std::time::Instant {
        tracing::info!(target: "pipdock::exec", argv = ?self.argv(), "spawn");
        std::time::Instant::now()
    }

    fn logged_exit(&self, started: std::time::Instant, code: Option<i32>) {
        tracing::info!(
            target: "pipdock::exec",
            program = %self.program.display(),
            ?code,
            ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "exited"
        );
    }

    /// Run while streaming each output line to `sink`, for the live console drawer.
    ///
    /// Output is still captured and returned, because the parsers need the whole document and the
    /// bug-report ring buffer needs the tail.
    ///
    /// # Errors
    /// As [`Command::run`].
    pub async fn run_streaming(
        &self,
        sink: &ProgressSink,
        pkg: Option<PkgName>,
        phase: ExecMode,
    ) -> Result<Output> {
        // The step index comes from the caller's sink rather than a parameter: an adapter cannot
        // know its own position in the plan, which is why every call site used to pass 0.
        let step = sink.step;
        let started = self.logged_start();
        let mut child = self.build().spawn().map_err(|e| self.spawn_error(&e))?;
        // Held until this function returns. Dropping it — which the timeout and cancel arms below
        // do by returning — terminates the whole tree, not just `child`.
        let _tree = TreeGuard::attach(&child);

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let tag = |stream| LineTag {
            tx: sink.tx.clone(),
            step,
            pkg: pkg.clone(),
            phase,
            stream,
        };

        let run = async {
            let (out, err, status) = tokio::join!(
                pump(stdout, tag(Stream::Stdout)),
                pump(stderr, tag(Stream::Stderr)),
                child.wait()
            );
            (out, err, status)
        };

        let (stdout, stderr, status) = tokio::select! {
            triple = run => triple,
            () = tokio::time::sleep(self.timeout) => {
                return Err(self.stopped(Stop::TimedOut(self.timeout)));
            }
            () = cancelled(self.cancel.as_ref()) => {
                return Err(self.stopped(Stop::Cancelled));
            }
        };

        let status = status.map_err(|e| {
            PdError::new(Code::IntUnexpected, format!("waiting on child failed: {e}"))
        })?;
        self.logged_exit(started, status.code());

        Ok(Output {
            code: status.code(),
            stdout,
            stderr,
        })
    }
}

/// Where one stream's lines are sent, and what each is tagged with.
struct LineTag {
    tx: EventSink,
    step: usize,
    pkg: Option<PkgName>,
    phase: ExecMode,
    stream: Stream,
}

/// Read one stream to its end, forwarding each line to the drawer, and return it whole.
///
/// **Bytes, decoded lossily -- never `lines()`.** tokio's `Lines` returns an error at the first
/// byte that is not UTF-8, and the loop this replaced treated that as end of stream: it stopped
/// reading and dropped the pipe, so the child died on a broken pipe mid-install and the stderr the
/// classifier needed was cut off at that point. MSVC writes its diagnostics in the OEM code page,
/// so a non-English Windows was one failed source build away from it.
async fn pump<R: AsyncRead + Unpin>(reader: Option<R>, tag: LineTag) -> String {
    let mut buf = String::new();
    let Some(reader) = reader else {
        return buf;
    };
    let mut reader = BufReader::new(reader);
    let mut raw = Vec::new();
    loop {
        raw.clear();
        match reader.read_until(b'\n', &mut raw).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&raw);
        let line = text.trim_end_matches(['\n', '\r']);
        // The engine's own words, at DEBUG: everything a run printed is in the log when asked for
        // (`PIPDOCK_LOG=debug`, `--verbose`), and a default log stays readable.
        tracing::debug!(target: "pipdock::engine", stream = ?tag.stream, "{line}");
        // Send failure means the UI stopped listening; keep capturing regardless so the summary
        // and the log are still complete.
        let _ = tag.tx.send(ProgressEvent::Line {
            step: tag.step,
            pkg: tag.pkg.clone(),
            phase: tag.phase,
            stream: tag.stream,
            line: line.to_owned(),
        });
        buf.push_str(line);
        buf.push('\n');
    }
    buf
}

/// Kills a spawned command **and everything it spawned** when dropped.
///
/// `kill_on_drop` reaches the direct child only, and for an engine that is not enough:
/// `python -m pip` spawns build backends and compilers for any source distribution, so killing
/// pip leaves those running. Worse than a leak — a surviving grandchild holds the stdout pipe it
/// inherited, so the parent's read never reaches EOF and cancellation does not appear to happen at
/// all. Measured before this existed: `cmd.exe /C ping -n 30` cancelled after 200 ms took the full
/// 30 s to return.
///
/// A Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` fixes it: a process assigned to
/// a job puts its own children in the same job, and closing the last handle terminates all of
/// them. Dropping this guard closes that handle.
///
/// Deferred from Stage 1 to the slice that could verify it — cancelling a build is the case that
/// needs it, and S3 is where builds happen.
#[cfg(windows)]
struct TreeGuard(#[allow(dead_code, reason = "the drop is the whole point")] win32job::Job);

#[cfg(windows)]
impl TreeGuard {
    /// Put `child`, and everything it goes on to spawn, into a job that dies with this guard.
    ///
    /// `None` when the job cannot be created or the process cannot be assigned — which is not
    /// worth failing a command over, because the fallback is exactly the previous behaviour:
    /// `kill_on_drop` still ends the direct child.
    fn attach(child: &tokio::process::Child) -> Option<Self> {
        let handle = child.raw_handle()?;
        let mut info = win32job::ExtendedLimitInfo::new();
        info.limit_kill_on_job_close();
        let job = win32job::Job::create_with_limit_info(&info).ok()?;
        // win32job takes the HANDLE as an isize; `raw_handle()` hands back a pointer.
        job.assign_process(handle as isize).ok()?;
        Some(Self(job))
    }
}

/// Non-Windows builds have no job objects; v1 is Windows-only, and this keeps the call sites
/// platform-free rather than sprinkling `cfg` through them.
#[cfg(not(windows))]
struct TreeGuard;

#[cfg(not(windows))]
impl TreeGuard {
    fn attach(_child: &tokio::process::Child) -> Option<Self> {
        None
    }
}

/// A file in the temp directory that is deleted when this is dropped.
///
/// SECURITY §2: what PipDock hands an interpreter or an engine -- `probe.py`, a requirements file,
/// a freeze for pip-audit -- is written under a fresh random name and never installed anywhere.
/// The name is created with `create_new`, so an existing file is never written through: a leaked
/// temp file is harmless, an overwritten one would not be. (The function this replaced documented
/// that rule and then called `fs::write`, which overwrites.)
#[derive(Debug)]
pub struct TempFile {
    path: PathBuf,
}

impl TempFile {
    /// Write `contents` to a new, uniquely named file.
    ///
    /// # Errors
    /// `PD-SYS-002` when the temp directory is full, `PD-SYS-001` for a path Windows refuses as too
    /// long, `PD-PRM-001` when it is not writable, `PD-INT-001` otherwise.
    pub fn create(prefix: &str, extension: &str, contents: &str) -> Result<Self> {
        use std::io::Write as _;

        let dir = std::env::temp_dir();
        let mut last = None;
        // A collision needs the same 64-bit nonce twice; the retry is for that theoretical case
        // and for a planted file, which `create_new` refuses rather than writes through.
        for _ in 0..8 {
            let path = dir.join(format!("{prefix}-{:016x}.{extension}", nonce()));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    return match file.write_all(contents.as_bytes()) {
                        Ok(()) => Ok(Self { path }),
                        Err(e) => {
                            drop(file);
                            let _ = std::fs::remove_file(&path);
                            Err(io_error(&path, &e))
                        }
                    };
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(io_error(&path, &e)),
            }
        }
        Err(PdError::new(
            Code::IntUnexpected,
            format!(
                "could not create a unique temp file in {}: {}",
                dir.display(),
                last.map(|e| e.to_string()).unwrap_or_default()
            ),
        ))
    }

    /// Where the file is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        // Best effort: a file the OS still holds open (an antivirus scan, say) stays behind,
        // which is untidy and harmless.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A per-call nonce: the process id, a process-wide counter and the clock, hashed with a
/// `RandomState` (randomly keyed per process). Unique without a `rand` dependency.
fn nonce() -> u64 {
    use std::hash::{BuildHasher as _, RandomState};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    RandomState::new().hash_one((std::process::id(), count, now))
}

/// Map a filesystem failure onto the catalog code whose advice fits it.
///
/// Shared so the shell stops answering every read or write failure with "not enough free disk
/// space" (`PD-SYS-002`), which it did for permission errors and missing files alike.
#[must_use]
pub fn io_error(path: &Path, e: &std::io::Error) -> PdError {
    const ERROR_DISK_FULL: i32 = 112;
    const ERROR_FILENAME_EXCED_RANGE: i32 = 206;
    let code = match (e.raw_os_error(), e.kind()) {
        (Some(ERROR_DISK_FULL), _) | (_, std::io::ErrorKind::StorageFull) => Code::SysDiskFull,
        (Some(ERROR_FILENAME_EXCED_RANGE), _) => Code::SysPathTooLong,
        (_, std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem) => {
            Code::PrmSitePackagesReadOnly
        }
        _ => Code::IntUnexpected,
    };
    PdError::new(code, format!("{}: {e}", path.display()))
}

/// [`io_error`] for a file **the user chose** — an export, a saved report, a requirements file —
/// where "PipDock is not allowed to write to this environment" (`PD-PRM-001`) would be the wrong
/// advice. The shell answered every such failure with `PD-SYS-002`, "not enough free disk space",
/// for a read-only folder and a missing file alike (U25).
#[must_use]
pub fn user_file_error(path: &Path, e: &std::io::Error) -> PdError {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    let code = match (e.raw_os_error(), e.kind()) {
        (Some(ERROR_SHARING_VIOLATION), _) => Code::PrmFileLocked,
        (_, std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem) => {
            Code::PrmSourceReadOnly
        }
        _ => return io_error(path, e),
    };
    PdError::new(code, format!("{}: {e}", path.display()))
}

/// Canonicalize an interpreter path for identity purposes.
///
/// **Spike SP-6.** The same interpreter reports different casing depending on how it was launched
/// — a Chocolatey shim yields `c:\python314\python.exe` where the direct install yields
/// `C:\Python314\python.exe`. `env_hash` is derived from this string (ARCHITECTURE §6), so without
/// case folding one environment would silently split its pins and snapshot history in two.
#[must_use]
pub fn canonical_interpreter(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.display().to_string();
    // Strip the Windows verbatim prefix canonicalize() adds; it is noise in logs and in the hash.
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_is_one_token_per_argument() {
        let cmd = Command::new("python.exe")
            .arg("-m")
            .arg("pip")
            .args(["install", "requests==2.32.3"]);
        assert_eq!(
            cmd.argv(),
            ["python.exe", "-m", "pip", "install", "requests==2.32.3"]
        );
    }

    #[test]
    fn a_path_with_spaces_stays_a_single_argument() {
        // There is no quoting step to get wrong, because there is no command line.
        let cmd = Command::new(r"C:\Program Files\Python312\python.exe").arg("--version");
        assert_eq!(cmd.argv()[0], r"C:\Program Files\Python312\python.exe");
        assert_eq!(cmd.argv().len(), 2);
    }

    #[test]
    fn python_commands_carry_the_sp2_encoding_mitigation() {
        // Without these, `pip install --dry-run --report -` crashes on Windows/cp1252.
        let cmd = Command::python("python.exe");
        assert_eq!(
            cmd.env.get("PYTHONIOENCODING").map(String::as_str),
            Some("utf-8")
        );
        assert_eq!(cmd.env.get("PYTHONUTF8").map(String::as_str), Some("1"));
    }

    #[tokio::test]
    async fn runs_a_real_process_and_captures_output() {
        let cmd = Command::new(std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into()))
            .args(["/C", "echo", "pipdock"]);
        let out = cmd.run().await.expect("cmd.exe should run on Windows");
        assert!(out.ok(), "exit {:?}", out.code);
        assert!(
            out.stdout.contains("pipdock"),
            "stdout was {:?}",
            out.stdout
        );
    }

    /// A command that sleeps far longer than any test should wait.
    ///
    /// Spawned **directly**, so this measures cancellation of the child itself.
    /// [`a_slow_grandchild`] is the tree case.
    fn a_slow_command() -> Command {
        // `ping -n` is the sleep that exists on every Windows install.
        Command::new("ping").args(["-n", "30", "127.0.0.1"])
    }

    /// The same sleep, one level down: `cmd.exe` is the child, `ping` is the grandchild.
    ///
    /// The shape that matters in production — `python -m pip` is the child and a build backend is
    /// the grandchild. `marker` is a loopback address used only to identify this exact process
    /// again in [`grandchildren_alive`].
    #[cfg(windows)]
    fn a_slow_grandchild(marker: &str) -> Command {
        Command::new("cmd").args(["/C", &format!("ping -n 30 {marker}")])
    }

    /// How many processes are still running with `marker` on their command line.
    ///
    /// **Wall-clock timing cannot answer this.** Both the cancel and the watchdog arm return as
    /// soon as their future resolves, so `run()` comes back in milliseconds whether or not the
    /// tree died — the first version of this test passed while taking 29 seconds, which is how
    /// the mistake surfaced. Whether the grandchild is still alive is the only observable that
    /// distinguishes the two outcomes, so ask the OS.
    #[cfg(windows)]
    /// Wait until `grandchildren_alive` satisfies `want`, or give up.
    ///
    /// **Replaces a fixed sleep, which is what made this flake.** The setup step asserted the
    /// fixture had started after 700 ms and the teardown asserted the tree was gone after 800 ms;
    /// both are fine on an idle developer machine and neither is a promise a loaded CI runner
    /// keeps. It failed on `fmt · clippy · test` once, on the *setup* assertion — the fixture
    /// simply had not spawned yet, so the test proved nothing and said so.
    ///
    /// Polling for the condition keeps the assertions exactly as strict while removing the
    /// timing guess: a tree that never dies still fails, it just takes the timeout to say so.
    /// Poll until `want` holds, or give up and report the last answer we actually got.
    ///
    /// Sixty seconds rather than twenty, and the number is not a guess about the *fixture* — it is
    /// about the **poll**. Each iteration spawns PowerShell, so twenty seconds of wall clock was
    /// only a handful of samples on a busy runner rather than the two hundred the 100 ms sleep
    /// suggests. A failed query is never counted as an observation: it retries, and if none ever
    /// succeeds the test says so instead of concluding from a number nobody measured.
    async fn wait_for(marker: &str, want: impl Fn(usize) -> bool) -> usize {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let mut last = None;
        loop {
            if let Some(n) = grandchildren_alive(marker) {
                last = Some(n);
                if want(n) {
                    return n;
                }
            }
            if std::time::Instant::now() >= deadline {
                return last.expect("the process query never gave a usable answer");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// How many `PING.EXE` grandchildren carry the marker, or `None` when the query itself failed.
    ///
    /// **`None` is not zero**, and conflating the two is what made this pair of tests flaky in
    /// both directions. Each call spawns PowerShell and a CIM query, which costs seconds on a
    /// loaded runner; `.unwrap_or(0)` turned a slow or failed invocation into a confident "no
    /// processes". At setup that reads as *the fixture never started* — which is how `main` went
    /// red at `bee85dc` on a change that touched nothing here. At teardown it is worse, because
    /// the assertion there wants zero: a failed query would have **passed** the test over a
    /// process tree that was still alive.
    fn grandchildren_alive(marker: &str) -> Option<usize> {
        let out = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "@(Get-CimInstance Win32_Process -Filter \"Name='PING.EXE'\" | \
                     Where-Object {{ $_.CommandLine -like '*{marker}*' }}).Count"
                ),
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }

    #[tokio::test]
    async fn a_cancelled_command_stops_promptly() {
        let token = CancellationToken::new();
        let cmd = a_slow_command().cancel(token.clone());

        let started = std::time::Instant::now();
        let handle = tokio::spawn(async move { cmd.run().await });
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();

        let result = handle.await.expect("task joins");
        assert!(result.is_err(), "a cancelled run must not report success");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "cancellation should not wait for the process: took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn cancelling_kills_the_child_rather_than_orphaning_it() {
        // The bug this guards: `tokio::time::timeout` drops the future, and a tokio `Child` does
        // not kill on drop by default — so the watchdog used to leave pip running. Dropping the
        // future has to actually end the process.
        //
        // Measured by wall clock, which is the observable that actually distinguishes the two
        // outcomes: a killed child returns in milliseconds, an orphaned one holds its stdout pipe
        // and the run does not finish until it exits on its own — 30 s here.
        let token = CancellationToken::new();
        let cmd = a_slow_command().cancel(token.clone());

        let started = std::time::Instant::now();
        let handle = tokio::spawn(async move { cmd.run().await });
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();
        let result = handle.await.expect("task joins");

        assert!(result.is_err(), "a cancelled run must not report success");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the child outlived cancellation: the run took {:?}, and the command sleeps for 30 s",
            started.elapsed()
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cancelling_kills_the_grandchildren_too() {
        // The gap `TreeGuard` closes, and the one that matters for a real cancel: `python -m pip`
        // spawns build backends, so killing pip alone leaves compilers running.
        let marker = "127.0.0.77";
        let token = CancellationToken::new();
        let cmd = a_slow_grandchild(marker).cancel(token.clone());

        let handle = tokio::spawn(async move { cmd.run().await });
        assert!(
            wait_for(marker, |n| n > 0).await > 0,
            "the fixture never started, so this test proves nothing"
        );

        token.cancel();
        let result = handle.await.expect("task joins");
        assert!(result.is_err(), "a cancelled run must not report success");

        // Termination is asynchronous, so this polls rather than sleeping a guessed interval.
        assert_eq!(
            wait_for(marker, |n| n == 0).await,
            0,
            "a grandchild outlived cancellation — the job object did not take the tree with it"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn the_watchdog_also_reaches_a_grandchild() {
        // Same tree, stopped by the other path. Both exits work by dropping the future that owns
        // the guard, but only one of them was ever exercised.
        let marker = "127.0.0.78";
        let cmd = a_slow_grandchild(marker).timeout(Duration::from_millis(600));

        let result = cmd.run().await;
        assert!(result.is_err(), "the watchdog must stop the run");

        // Polled rather than slept, for its sibling's reason: 800 ms is fine on an idle machine
        // and is not a promise a loaded CI runner keeps.
        assert_eq!(
            wait_for(marker, |n| n == 0).await,
            0,
            "a grandchild outlived the watchdog"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn each_line_says_which_stream_it_came_from() {
        // The console drawer renders the two differently, and for uv the distinction is not a
        // judgement: uv writes its *plan* to stderr (SP-1), so stderr does not mean failure.
        // Needs a real subprocess — the `FakeEngine` used elsewhere returns results without ever
        // producing output, which is how the first version of this test proved nothing.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = ProgressSink::new(tx, 1, CancellationToken::new());

        Command::new("cmd")
            .args(["/C", "echo to-stdout& echo to-stderr 1>&2"])
            .run_streaming(&sink, None, ExecMode::Batch)
            .await
            .expect("the command runs");
        drop(sink);

        let mut seen = Vec::new();
        while let Some(event) = rx.recv().await {
            if let ProgressEvent::Line { stream, line, .. } = event {
                seen.push((stream, line.trim().to_owned()));
            }
        }

        assert!(
            seen.contains(&(Stream::Stdout, "to-stdout".to_owned())),
            "stdout was not tagged as stdout: {seen:?}"
        );
        assert!(
            seen.contains(&(Stream::Stderr, "to-stderr".to_owned())),
            "stderr was not tagged as stderr: {seen:?}"
        );
    }

    #[tokio::test]
    async fn the_watchdog_still_fires_without_a_token() {
        // A command with no token must still be stoppable — the token adds a reason to stop, it
        // does not become the only one.
        let cmd = a_slow_command().timeout(Duration::from_millis(300));
        let err = cmd.run().await.expect_err("must time out");
        // An engine timeout, not "PipDock hit an internal error" — and said in the type, so no
        // caller has to read the message to know.
        assert_eq!(err.code, Code::EngTimedOut);
        assert!(err.is_timeout(), "{err:?}");
        assert!(!err.is_cancelled());
    }

    #[tokio::test]
    async fn a_cancelled_run_says_so_in_its_type() {
        let token = CancellationToken::new();
        let cmd = a_slow_command().cancel(token.clone());
        let handle = tokio::spawn(async move { cmd.run().await });
        tokio::time::sleep(Duration::from_millis(200)).await;
        token.cancel();
        let err = handle
            .await
            .expect("task joins")
            .expect_err("a cancelled run must not report success");
        assert!(err.is_cancelled(), "{err:?}");
        assert!(!err.is_timeout());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn the_pump_reads_past_bytes_that_are_not_utf8() {
        // The bug: tokio's `lines()` errors at the first non-UTF-8 byte, the old loop took that as
        // end of stream and dropped the pipe — so everything after it was lost, and a real child
        // would die writing to a closed pipe. MSVC prints in the OEM code page, so this is a
        // source build on a non-English Windows. `type` writes the bytes back verbatim.
        let file = TempFile::create("pd-test-bytes", "txt", "").expect("temp file");
        std::fs::write(
            file.path(),
            b"before\r\nL\xf6sung \xe0 \x81\x8d broken\r\nafter the bad line\r\n",
        )
        .expect("write raw bytes");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = ProgressSink::new(tx, 1, CancellationToken::new());
        let out = Command::new("cmd")
            .args(["/C", "type", &file.path().display().to_string()])
            .run_streaming(&sink, None, ExecMode::Batch)
            .await
            .expect("the command runs");
        drop(sink);

        assert!(
            out.stdout.contains("after the bad line"),
            "output after the invalid bytes was lost: {:?}",
            out.stdout
        );
        let mut lines = 0;
        while let Some(event) = rx.recv().await {
            if matches!(event, ProgressEvent::Line { .. }) {
                lines += 1;
            }
        }
        assert_eq!(lines, 3, "every line reaches the drawer, lossily decoded");
    }

    #[tokio::test]
    async fn an_untouched_token_does_not_abort_the_command() {
        // `select!` needs a future in every arm; a token-less arm that resolved immediately
        // would abort every command that did not opt in.
        let cmd = Command::new(std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into()))
            .args(["/C", "echo", "pipdock"])
            .cancel(CancellationToken::new());
        let out = cmd
            .run()
            .await
            .expect("an uncancelled token must not stop it");
        assert!(out.stdout.contains("pipdock"), "{:?}", out.stdout);
    }

    #[tokio::test]
    async fn a_missing_program_is_an_engine_error() {
        let err = Command::new("pipdock-definitely-not-a-real-binary")
            .run()
            .await
            .expect_err("missing program must fail");
        assert_eq!(err.code, Code::EngNotFound);
    }

    #[tokio::test]
    async fn a_missing_interpreter_is_an_environment_error() {
        // A deleted venv's python.exe used to be PD-ENG-001, "install the engine". The engine is
        // fine; the environment is gone.
        let err = Command::python(r"C:\pipdock\no\such\venv\Scripts\python.exe")
            .arg("--version")
            .run()
            .await
            .expect_err("missing interpreter must fail");
        assert_eq!(err.code, Code::EnvInterpreterMissing);
    }

    #[test]
    fn temp_files_get_unique_names_and_go_away_on_drop() {
        let a = TempFile::create("pd-test", "txt", "a").expect("write");
        let b = TempFile::create("pd-test", "txt", "b").expect("write");
        assert_ne!(a.path(), b.path(), "two probe writes must not collide");
        assert_eq!(std::fs::read_to_string(a.path()).expect("read"), "a");
        let (pa, pb) = (a.path().to_path_buf(), b.path().to_path_buf());
        drop(a);
        drop(b);
        assert!(
            !pa.exists() && !pb.exists(),
            "dropping a TempFile removes it"
        );
    }

    #[test]
    fn filesystem_failures_carry_the_code_whose_advice_fits() {
        use std::io::{Error, ErrorKind};
        let p = Path::new(r"C:\x");
        assert_eq!(
            io_error(p, &Error::from_raw_os_error(112)).code,
            Code::SysDiskFull
        );
        assert_eq!(
            io_error(p, &Error::from_raw_os_error(206)).code,
            Code::SysPathTooLong
        );
        assert_eq!(
            io_error(p, &Error::from(ErrorKind::PermissionDenied)).code,
            Code::PrmSitePackagesReadOnly
        );
        // A missing file is not "the disk is full", which every shell command used to say.
        assert_eq!(
            io_error(p, &Error::from(ErrorKind::NotFound)).code,
            Code::IntUnexpected
        );
    }

    #[test]
    fn interpreter_identity_is_case_folded_on_windows() {
        // SP-6: the Chocolatey shim and the direct install differ only in casing, and env_hash is
        // built from this string.
        let a = canonical_interpreter(Path::new(r"C:\Python314\python.exe"));
        let b = canonical_interpreter(Path::new(r"c:\python314\python.exe"));
        if cfg!(windows) {
            assert_eq!(a, b, "casing must not produce two identities");
        }
    }

    #[test]
    fn canonical_paths_have_no_verbatim_prefix() {
        let p = canonical_interpreter(Path::new("."));
        assert!(!p.starts_with(r"\\?\"), "got {p}");
    }
}
