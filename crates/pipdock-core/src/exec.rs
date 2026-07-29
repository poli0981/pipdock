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

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio_util::sync::CancellationToken;

use crate::engine::{ProgressEvent, ProgressSink};
use crate::errors::{Code, PdError, Result};
use crate::model::{ExecMode, PkgName};

/// Default watchdog. Long resolves on a big environment are normal; hangs are not.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

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
        }
    }

    /// Invoke a Python interpreter with the SP-2 encoding mitigation already applied.
    ///
    /// Use this for **every** `python -m pip …` and `probe.py` call. `PYTHONUTF8=1` covers the
    /// interpreter's own text handling and `PYTHONIOENCODING=utf-8` covers the stdio streams pip's
    /// vendored `rich` writes through.
    #[must_use]
    pub fn python(interpreter: impl Into<PathBuf>) -> Self {
        Self::new(interpreter)
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
    fn stopped(&self, reason: &str) -> PdError {
        PdError::new(
            Code::IntUnexpected,
            format!("{reason}: {}", self.program.display()),
        )
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
        // running: the future went away, the process did not. That was a live leak independent
        // of cancellation, and this is the whole fix for it.
        //
        // Known limit, and it is worse than "the grandchild leaks": measured while writing the
        // tests below, a `cmd.exe /C ping -n 30` cancelled after 200 ms took the full 30 s to
        // return. Killing the shell does not kill `ping`, and the surviving grandchild holds the
        // inherited stdout pipe open, so the read never reaches EOF. Cancellation is not just
        // incomplete there — it does not appear to happen at all.
        //
        // Killing a whole tree on Windows needs a Job Object with
        // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE. `python -m pip` spawns build backends and
        // compilers, so that is a real gap, tracked as follow-up. What this does cover is the
        // common case — a network-blocked engine, which is the one users actually cancel.
        cmd.kill_on_drop(true);
        cmd
    }

    /// Run to completion, capturing output.
    ///
    /// # Errors
    /// `PD-ENG-001` when the program cannot be spawned, `PD-INT-001` when the watchdog fires.
    pub async fn run(&self) -> Result<Output> {
        let out = tokio::select! {
            out = self.build().output() => out,
            () = tokio::time::sleep(self.timeout) => {
                return Err(self.stopped(&format!("timed out after {:?}", self.timeout)));
            }
            () = cancelled(self.cancel.as_ref()) => {
                return Err(self.stopped("cancelled"));
            }
        };
        let out = out.map_err(|e| {
            PdError::new(
                Code::EngNotFound,
                format!("could not run {}: {e}", self.program.display()),
            )
        })?;
        Ok(Output {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
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
        let mut child = self.build().spawn().map_err(|e| {
            PdError::new(
                Code::EngNotFound,
                format!("could not run {}: {e}", self.program.display()),
            )
        })?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let pump = |reader: Option<tokio::process::ChildStdout>| {
            let sink = sink.tx.clone();
            let pkg = pkg.clone();
            async move {
                let mut buf = String::new();
                if let Some(r) = reader {
                    let mut lines = BufReader::new(r).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        // Send failure means the UI stopped listening; keep capturing regardless
                        // so the summary and the log are still complete.
                        let _ = sink.send(ProgressEvent {
                            step,
                            pkg: pkg.clone(),
                            phase,
                            line: line.clone(),
                        });
                        buf.push_str(&line);
                        buf.push('\n');
                    }
                }
                buf
            }
        };
        let pump_err = |reader: Option<tokio::process::ChildStderr>| {
            let sink = sink.tx.clone();
            let pkg = pkg.clone();
            async move {
                let mut buf = String::new();
                if let Some(r) = reader {
                    let mut lines = BufReader::new(r).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let _ = sink.send(ProgressEvent {
                            step,
                            pkg: pkg.clone(),
                            phase,
                            line: line.clone(),
                        });
                        buf.push_str(&line);
                        buf.push('\n');
                    }
                }
                buf
            }
        };

        let run = async {
            let (out, err, status) = tokio::join!(pump(stdout), pump_err(stderr), child.wait());
            (out, err, status)
        };

        let (stdout, stderr, status) = tokio::select! {
            triple = run => triple,
            () = tokio::time::sleep(self.timeout) => {
                return Err(self.stopped(&format!("timed out after {:?}", self.timeout)));
            }
            () = cancelled(self.cancel.as_ref()) => {
                return Err(self.stopped("cancelled"));
            }
        };

        let status = status.map_err(|e| {
            PdError::new(Code::IntUnexpected, format!("waiting on child failed: {e}"))
        })?;

        Ok(Output {
            code: status.code(),
            stdout,
            stderr,
        })
    }
}

/// Write `contents` to a uniquely named temp file and return its path.
///
/// SECURITY §2: `probe.py` is written with a random name per invocation and never installed into
/// the environment. The caller removes it; a leaked file in the temp directory is harmless, an
/// overwritten one would not be.
///
/// # Errors
/// `PD-SYS-002` when the file cannot be written.
pub fn write_temp(prefix: &str, extension: &str, contents: &str) -> Result<PathBuf> {
    use std::hash::{BuildHasher as _, RandomState};

    let dir = std::env::temp_dir();
    // RandomState is seeded by the OS per process; hashing a fresh allocation's address plus a
    // counter gives a unique-enough name without pulling in a rand dependency.
    let nonce = RandomState::new().hash_one(std::time::Instant::now().elapsed().as_nanos());
    let path = dir.join(format!("{prefix}-{nonce:016x}.{extension}"));
    std::fs::write(&path, contents).map_err(|e| {
        PdError::new(
            Code::SysDiskFull,
            format!("could not write {}: {e}", path.display()),
        )
    })?;
    Ok(path)
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
    /// Spawned **directly**, not through `cmd.exe`. Wrapping it makes the sleeper a grandchild,
    /// and killing the shell does not kill the grandchild — the run then blocks until the
    /// grandchild exits on its own. That is exactly the process-tree limitation documented on
    /// `build()`, and putting it in a test fixture would measure the limitation instead of the
    /// behaviour under test.
    fn a_slow_command() -> Command {
        // `ping -n` is the sleep that exists on every Windows install.
        Command::new("ping").args(["-n", "30", "127.0.0.1"])
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

    #[tokio::test]
    async fn the_watchdog_still_fires_without_a_token() {
        // A command with no token must still be stoppable — the token adds a reason to stop, it
        // does not become the only one.
        let cmd = a_slow_command().timeout(Duration::from_millis(300));
        let err = cmd.run().await.expect_err("must time out");
        assert_eq!(err.code, Code::IntUnexpected);
        assert!(err.message.contains("timed out"), "{}", err.message);
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

    #[test]
    fn temp_files_get_unique_names() {
        let a = write_temp("pd-test", "txt", "a").expect("write");
        let b = write_temp("pd-test", "txt", "b").expect("write");
        assert_ne!(a, b, "two probe writes must not collide");
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
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
