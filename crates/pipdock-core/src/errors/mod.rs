//! Error types. See `docs/ERROR-CATALOG.md`.

pub mod catalog;

pub use catalog::{Area, Code, classify_stderr};

/// The error type crossing every boundary: Tauri IPC, CLI output, logs.
///
/// Invariant 4 of `docs/DATA-FLOW.md` §9: every failure surfaced to UI or CLI carries a catalog
/// code. That is why `code` is not optional.
#[derive(Debug, thiserror::Error, serde::Serialize, serde::Deserialize)]
#[error("error[{code}]: {message}")]
pub struct PdError {
    /// The single catalog code for this failure.
    pub code: Code,
    /// Developer-facing detail. **Not** the user-facing string — per `docs/I18N.md` §1 the core
    /// emits codes and structured data only; the localized one-liner is looked up frontend-side
    /// from the code.
    pub message: String,
    /// Tail of the engine's stderr, capped at the 40 lines `docs/ERROR-CATALOG.md` §3 shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
}

impl PdError {
    /// Build an error for `code` with developer-facing detail.
    #[must_use]
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            stderr_tail: None,
        }
    }

    /// Attach an engine stderr tail, truncated to the last `MAX_STDERR_TAIL_LINES` lines.
    #[must_use]
    pub fn with_stderr(mut self, stderr: impl AsRef<str>) -> Self {
        let stderr = stderr.as_ref();
        let tail: Vec<&str> = stderr
            .lines()
            .rev()
            .take(MAX_STDERR_TAIL_LINES)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        self.stderr_tail = Some(tail.join("\n"));
        self
    }

    /// Classify an engine failure from its stderr (SP-2 fills the classifier table).
    #[must_use]
    pub fn from_engine_stderr(stderr: impl AsRef<str>) -> Self {
        let stderr = stderr.as_ref();
        Self::new(classify_stderr(stderr), "engine command failed").with_stderr(stderr)
    }
}

/// `docs/ERROR-CATALOG.md` §3: the GUI's *Details* pane shows at most this many stderr lines.
pub const MAX_STDERR_TAIL_LINES: usize = 40;

/// Convenience alias for core results.
pub type Result<T> = std::result::Result<T, PdError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_tail_keeps_the_last_lines_in_order() {
        let stderr = (1..=60)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let err = PdError::new(Code::BldBackendFailed, "boom").with_stderr(&stderr);
        let tail = err.stderr_tail.unwrap_or_default();
        let lines: Vec<&str> = tail.lines().collect();

        assert_eq!(lines.len(), MAX_STDERR_TAIL_LINES);
        assert_eq!(
            lines.first(),
            Some(&"21"),
            "should start 40 lines from the end"
        );
        assert_eq!(
            lines.last(),
            Some(&"60"),
            "should keep the newest line last"
        );
    }

    #[test]
    fn short_stderr_is_kept_whole() {
        let err = PdError::new(Code::BldBackendFailed, "boom").with_stderr("a\nb");
        assert_eq!(err.stderr_tail.as_deref(), Some("a\nb"));
    }

    #[test]
    fn display_leads_with_the_code() {
        let err = PdError::new(Code::SnpWriteFailed, "disk full");
        assert_eq!(err.to_string(), "error[PD-SNP-001]: disk full");
    }
}
