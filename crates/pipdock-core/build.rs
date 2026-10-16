//! Bakes the legal-documents hash into the binary.
//!
//! UI-SPEC §4 stores consent "with docs-version hash", and a bump re-triggers the gate. The hash
//! is computed here rather than at runtime because the documents are deliberately *not* shipped —
//! the gate links to the GitHub copies (PRD P0-13), so a runtime hash would be a hash of files
//! that are not on the user's disk.
//!
//! It is a re-consent trigger, not an integrity check: it answers "have the terms changed since
//! this user agreed", which is a question about the repository at build time.

use std::path::{Path, PathBuf};

#[path = "src/legal_hash.rs"]
mod legal_hash;

/// The five documents the gate lists. UI-SPEC §4 says five; `legal/` holds four, and the fifth is
/// the GPL-3.0 `LICENSE` at the repository root.
const DOCUMENTS: &[&str] = &[
    "LICENSE",
    "legal/EULA.md",
    "legal/DISCLAIMER.md",
    "legal/PRIVACY-POLICY.md",
    "legal/THIRD-PARTY-NOTICES.md",
];

fn main() {
    let root = repo_root();
    let mut documents = Vec::with_capacity(DOCUMENTS.len());
    for name in DOCUMENTS {
        let path = root.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        // A missing document must not silently drop out of the hash, or removing a file would
        // leave existing consent looking current. Its name is still hashed, with no bytes.
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            println!("cargo:warning=legal document {name} unreadable ({e}); hashing its absence");
            Vec::new()
        });
        documents.push((*name, bytes));
    }
    let pairs: Vec<(&str, &[u8])> = documents.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    println!(
        "cargo:rustc-env=PIPDOCK_LEGAL_DOCS_HASH={}",
        legal_hash::docs_hash(&pairs)
    );
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/pipdock-core.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}
