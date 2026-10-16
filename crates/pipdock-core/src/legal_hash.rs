//! The legal-documents hash: the re-consent trigger `build.rs` bakes into the binary.
//!
//! A file of its own so that `build.rs` can `#[path]`-include it — a build script cannot use the
//! crate it builds — and so the tests below exercise the very function that runs there, rather
//! than a copy of it.

/// Hash `(name, contents)` pairs into the value consent is stored against (UI-SPEC §4).
///
/// Line endings are normalised **before anything is hashed, the length included**. The length
/// used to be taken of the raw bytes, two lines above a normalisation that existed precisely so a
/// CRLF checkout — Windows' default — would hash like an LF one. It did not: every build from a
/// CRLF checkout carried a different hash from one built on an LF checkout of the same commit,
/// which is a legal gate shown again for nothing (X25).
#[must_use]
pub fn docs_hash(documents: &[(&str, &[u8])]) -> String {
    let mut hasher = Sha256::new();
    for (name, bytes) in documents {
        let text = normalise(bytes);
        hasher.update(name.as_bytes());
        hasher.update(&(text.len() as u64).to_le_bytes());
        hasher.update(&text);
    }
    hex(&hasher.finish())
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
        let (words, _) = out.as_chunks_mut::<4>();
        for (chunk, word) in words.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        let (words, _) = block.as_chunks::<4>();
        for (slot, chunk) in w.iter_mut().zip(words) {
            *slot = u32::from_be_bytes(*chunk);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crlf_checkout_hashes_like_an_lf_one() {
        let lf = docs_hash(&[
            ("LICENSE", b"GPL\nv3\n"),
            ("legal/EULA.md", b"# EULA\n\nterms\n"),
        ]);
        let crlf = docs_hash(&[
            ("LICENSE", b"GPL\r\nv3\r\n"),
            ("legal/EULA.md", b"# EULA\r\n\r\nterms\r\n"),
        ]);
        assert_eq!(lf, crlf);
    }

    #[test]
    fn a_changed_document_changes_the_hash() {
        let before = docs_hash(&[("legal/EULA.md", b"terms\n")]);
        let after = docs_hash(&[("legal/EULA.md", b"terms, amended\n")]);
        assert_ne!(before, after);
    }

    #[test]
    fn a_document_moving_between_files_changes_the_hash() {
        // The length is what separates one document from the next: without it, text moved from
        // the end of one file to the start of the next would hash the same.
        let a = docs_hash(&[("a", b"xy"), ("b", b"z")]);
        let b = docs_hash(&[("a", b"x"), ("b", b"yz")]);
        assert_ne!(a, b);
    }

    #[test]
    fn the_sha256_is_sha256() {
        // The build script carries its own SHA-256 so it needs no dependency (SECURITY §7). The
        // library has sha2 anyway, which makes it the oracle — across block boundaries.
        use sha2::Digest as _;
        for len in [0_usize, 1, 55, 56, 63, 64, 65, 119, 1000] {
            let data: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
            let mut ours = Sha256::new();
            ours.update(&data);
            let theirs: [u8; 32] = sha2::Sha256::digest(&data).into();
            assert_eq!(ours.finish(), theirs, "length {len}");
        }
    }
}
