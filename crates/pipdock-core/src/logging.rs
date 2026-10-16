//! Where PipDock's own log goes: one file per day under `data\logs\`, kept for
//! [`crate::LOG_RETENTION_DAYS`].
//!
//! Until 1.4.0 there was none. `tracing` was a dependency of three crates and called zero times,
//! the CLI's `--verbose` and `--log-file` were parsed and ignored, `LOG_RETENTION_DAYS` had no
//! reader — and the privacy policy described "log files (14-day rotation)" that did not exist. A
//! bug report could attach the last run's engine output and nothing about what PipDock did.
//!
//! This module owns the parts both heads share: the daily file, the sweep, reading the tail back.
//! The subscriber — which levels go where — is each head's, because only the head knows whether a
//! terminal is listening. **Nothing here ever writes to stdout**: stdout is the CLI's contract.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The log directory under the app-data root.
#[must_use]
pub fn log_dir(app_data: &Path) -> PathBuf {
    app_data.join("logs")
}

/// One file per (UTC) day: `<prefix>.<YYYY-MM-DD>.log`, opened on first write and reopened when
/// the day changes. Cloning shares the open file.
///
/// UTC rather than local time, so a file's name and the timestamps inside it agree without
/// consulting a time-zone database.
#[derive(Debug, Clone)]
pub struct DailyFile {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Debug)]
struct Inner {
    dir: PathBuf,
    prefix: &'static str,
    open: Option<(jiff::civil::Date, std::fs::File)>,
}

impl DailyFile {
    /// Log under `dir` as `<prefix>.<date>.log` — `pipdock` for the GUI, `pipdock-cli` for the CLI,
    /// so two heads running at once never interleave lines in one file.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>, prefix: &'static str) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                dir: dir.into(),
                prefix,
                open: None,
            })),
        }
    }

    /// Today's file — what a user is pointed at when something went wrong.
    #[must_use]
    pub fn current_path(&self) -> PathBuf {
        let inner = self.lock();
        file_for(&inner.dir, inner.prefix, today())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl std::io::Write for DailyFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut inner = self.lock();
        let day = today();
        if inner.open.as_ref().is_none_or(|(d, _)| *d != day) {
            std::fs::create_dir_all(&inner.dir)?;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file_for(&inner.dir, inner.prefix, day))?;
            inner.open = Some((day, file));
        }
        match &mut inner.open {
            Some((_, file)) => {
                file.write_all(buf)?;
                Ok(buf.len())
            }
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.lock().open {
            Some((_, file)) => file.flush(),
            None => Ok(()),
        }
    }
}

fn today() -> jiff::civil::Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}

fn file_for(dir: &Path, prefix: &str, day: jiff::civil::Date) -> PathBuf {
    dir.join(format!("{prefix}.{day}.log"))
}

/// The date in a file name this module wrote, if it is one.
fn date_of(name: &str) -> Option<jiff::civil::Date> {
    let stem = name.strip_suffix(".log")?;
    let (_, date) = stem.rsplit_once('.')?;
    date.parse().ok()
}

/// Delete log files older than `keep_days`. Returns how many went.
///
/// Only files whose name carries a date this module could have written are considered, so a
/// file a user dropped into the folder is never touched. A file that cannot be removed — open in
/// an editor — is left for the next sweep.
pub fn sweep(dir: &Path, keep_days: u32, now: jiff::Timestamp) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
    let Ok(oldest) = today.checked_sub(jiff::Span::new().days(i64::from(keep_days))) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .and_then(date_of)
                .is_some_and(|d| d < oldest)
        })
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

/// The end of PipDock's own log, for Settings' Logs row (ARCHITECTURE §7).
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct LogTail {
    /// The folder the log files are in. A path — data, never localized (I18N §2).
    pub dir: String,
    /// The most recent lines, oldest first. Empty when nothing has been logged yet.
    pub lines: Vec<String>,
}

/// The last `max_lines` lines across the newest log files in `dir`, oldest first.
///
/// Reads backwards through the files by date, so a tail that straddles midnight still has the
/// lines from before it. Bounded: at most a few files are read.
#[must_use]
pub fn tail(dir: &Path, max_lines: usize) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(jiff::civil::Date, String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_owned();
            Some((date_of(&name)?, name, e.path()))
        })
        .collect();
    // Newest first; within a day, by name, so the two heads' files stay in a fixed order.
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));

    let mut lines: Vec<String> = Vec::new();
    for (_, _, path) in files.iter().take(4) {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut chunk: Vec<String> = text.lines().map(str::to_owned).collect();
        chunk.append(&mut lines);
        lines = chunk;
        if lines.len() >= max_lines {
            break;
        }
    }
    let start = lines.len().saturating_sub(max_lines);
    lines.split_off(start)
}

/// Record a panic in the log before the default hook prints it.
///
/// A GUI panic otherwise vanishes with the window: there is no terminal to print to, and the next
/// launch has no trace of why the last one ended.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(target: "pipdock::panic", "{info}");
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pd-logs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn lines_land_in_todays_file_and_never_anywhere_else() {
        let dir = scratch("write");
        let mut log = DailyFile::new(&dir, "pipdock-cli");
        writeln!(log, "first").expect("write");
        writeln!(log.clone(), "second").expect("write through a clone");
        let path = log.current_path();
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("pipdock-cli.")
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "first\nsecond\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sweep_keeps_fourteen_days_and_nothing_it_did_not_write() {
        let dir = scratch("sweep");
        std::fs::create_dir_all(&dir).expect("dir");
        for name in [
            "pipdock.2026-09-30.log",
            "pipdock-cli.2026-10-01.log",
            "pipdock.2026-10-02.log",
            "pipdock.2026-10-16.log",
            "notes.txt",
            "pipdock.not-a-date.log",
        ] {
            std::fs::write(dir.join(name), "x").expect("seed");
        }
        let now: jiff::Timestamp = "2026-10-16T12:00:00Z".parse().expect("now");
        assert_eq!(sweep(&dir, 14, now), 2);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("read")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "notes.txt",
                "pipdock.2026-10-02.log",
                "pipdock.2026-10-16.log",
                "pipdock.not-a-date.log"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_tail_spans_files_and_keeps_order() {
        let dir = scratch("tail");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("pipdock.2026-10-15.log"), "a\nb\nc\n").expect("seed");
        std::fs::write(dir.join("pipdock.2026-10-16.log"), "d\ne\n").expect("seed");
        assert_eq!(tail(&dir, 3), ["c", "d", "e"]);
        assert_eq!(tail(&dir, 50), ["a", "b", "c", "d", "e"]);
        assert!(tail(&scratch("absent"), 5).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
