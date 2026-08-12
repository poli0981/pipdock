//! The project folder Code Health runs in, and what it declares its dependencies in.
//!
//! CODE-HEALTH-SPEC §3: the folder is persisted per environment, and the detection order for
//! declared dependencies is `pyproject.toml` → `requirements*.txt` → none.

use std::path::Path;

use crate::errors::{Code, PdError, Result};

/// Where the project declares its dependencies, if anywhere.
///
/// deptry needs one of these to have anything to compare against. `None` is not an error — it is
/// the "limited-mode notice" §3 asks the UI to show, and the other two tools do not care.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeclaredSource {
    /// A `pyproject.toml`, which is what deptry prefers.
    Pyproject,
    /// One or more `requirements*.txt`, named so the UI can say which.
    #[serde(rename_all = "camelCase")]
    Requirements {
        /// File names, sorted, relative to the project folder.
        files: Vec<String>,
    },
    /// Neither. deptry runs in limited mode and its findings mean less.
    None,
}

impl DeclaredSource {
    /// Whether deptry has anything to compare imports against.
    #[must_use]
    pub const fn is_declared(&self) -> bool {
        !matches!(self, Self::None)
    }
}

/// What `project` declares its dependencies in (CODE-HEALTH-SPEC §3's detection order).
///
/// `pyproject.toml` wins outright when present, rather than being merged with any
/// `requirements*.txt` beside it. That is deptry's own precedence, and reporting both would invite
/// the UI to explain a distinction deptry did not make.
///
/// # Errors
/// `PD-ENV-003` when the folder cannot be read at all — a path that is not a directory, or one the
/// user cannot list. Health has nothing to run against in either case.
pub fn declared_source(project: &Path) -> Result<DeclaredSource> {
    let unreadable = |e: &dyn std::fmt::Display| {
        PdError::new(
            Code::EnvProbeFailed,
            format!(
                "could not read the project folder {}: {e}",
                project.display()
            ),
        )
    };

    if !project.is_dir() {
        return Err(unreadable(&"not a directory"));
    }
    if project.join("pyproject.toml").is_file() {
        return Ok(DeclaredSource::Pyproject);
    }

    let mut files: Vec<String> = std::fs::read_dir(project)
        .map_err(|e| unreadable(&e))?
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_requirements(name))
        .collect();

    if files.is_empty() {
        return Ok(DeclaredSource::None);
    }
    files.sort();
    Ok(DeclaredSource::Requirements { files })
}

/// `requirements*.txt`, matched case-insensitively.
///
/// Windows filesystems are case-insensitive, so `Requirements.txt` is the same file to everyone
/// except a case-sensitive comparison — which would report "no declared dependencies" for a
/// project that plainly has some.
fn is_requirements(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("requirements") && lower.ends_with(".txt")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn pyproject_wins_over_requirements_beside_it() {
        // deptry's own precedence. Reporting both would invite the UI to explain a distinction
        // deptry did not make.
        let dir = scratch("both");
        write(&dir, "pyproject.toml");
        write(&dir, "requirements.txt");

        assert_eq!(
            declared_source(&dir).expect("readable"),
            DeclaredSource::Pyproject
        );
    }

    #[test]
    fn every_requirements_file_is_named_and_sorted() {
        let dir = scratch("reqs");
        write(&dir, "requirements-dev.txt");
        write(&dir, "requirements.txt");
        write(&dir, "notes.txt"); // not a requirements file
        write(&dir, "requirements.in"); // not .txt

        assert_eq!(
            declared_source(&dir).expect("readable"),
            DeclaredSource::Requirements {
                files: vec!["requirements-dev.txt".into(), "requirements.txt".into()],
            }
        );
    }

    #[test]
    fn casing_does_not_hide_a_requirements_file() {
        // Windows filesystems are case-insensitive; a case-sensitive match would report "nothing
        // declared" for a project that plainly declares things.
        let dir = scratch("case");
        write(&dir, "Requirements.TXT");

        assert!(declared_source(&dir).expect("readable").is_declared());
    }

    #[test]
    fn a_folder_declaring_nothing_is_a_state_not_a_failure() {
        // §3's limited-mode notice: deptry still runs, its findings just mean less.
        let dir = scratch("bare");
        write(&dir, "main.py");

        assert_eq!(
            declared_source(&dir).expect("readable"),
            DeclaredSource::None
        );
        assert!(!DeclaredSource::None.is_declared());
    }

    #[test]
    fn a_path_that_is_not_a_folder_is_a_catalog_code() {
        let err = declared_source(&scratch("missing").join("nope")).expect_err("not a directory");
        assert_eq!(err.code, Code::EnvProbeFailed);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pd-proj-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn write(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), "").expect("write");
    }
}
