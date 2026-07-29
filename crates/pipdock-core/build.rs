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
    let mut hasher = Sha256::new();

    for name in DOCUMENTS {
        let path = root.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        // A missing document must not silently drop out of the hash, or removing a file would
        // leave existing consent looking current. Hash the name and the bytes.
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            println!("cargo:warning=legal document {name} unreadable ({e}); hashing its absence");
            Vec::new()
        });
        hasher.update(name.as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        // Normalise line endings: a CRLF checkout must not produce a different hash from an LF
        // one, or every Windows contributor would re-trigger the gate for everybody.
        hasher.update(&normalise(&bytes));
    }

    println!(
        "cargo:rustc-env=PIPDOCK_LEGAL_DOCS_HASH={}",
        hex(&hasher.finish())
    );
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/pipdock-core.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn normalise(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().filter(|b| *b != b'\r').collect()
}

fn hex(digest: &[u8; 32]) -> String {
    digest.iter().fold(String::with_capacity(64), |mut acc, b| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

/// A small SHA-256, so the build script needs no dependencies.
///
/// A build-dependency here would be one more crate in the supply chain of the thing that decides
/// whether users see the legal gate (SECURITY §7). FIPS 180-4.
struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    length: u64,
}

const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09_e667,
                0xbb67_ae85,
                0x3c6e_f372,
                0xa54f_f53a,
                0x510e_527f,
                0x9b05_688c,
                0x1f83_d9ab,
                0x5be0_cd19,
            ],
            buffer: Vec::new(),
            length: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);
        self.buffer.extend_from_slice(data);
        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self.buffer[..64].try_into().unwrap_or([0; 64]);
            self.compress(&block);
            self.buffer.drain(..64);
        }
    }

    fn finish(mut self) -> [u8; 32] {
        let bits = self.length.wrapping_mul(8);
        self.buffer.push(0x80);
        while self.buffer.len() % 64 != 56 {
            self.buffer.push(0);
        }
        let tail = bits.to_be_bytes();
        self.buffer.extend_from_slice(&tail);
        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self.buffer[..64].try_into().unwrap_or([0; 64]);
            self.compress(&block);
            self.buffer.drain(..64);
        }

        let mut out = [0u8; 32];
        for (chunk, word) in out.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (slot, chunk) in w.iter_mut().zip(block.chunks_exact(4)) {
            *slot = u32::from_be_bytes(chunk.try_into().unwrap_or([0; 4]));
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
}
