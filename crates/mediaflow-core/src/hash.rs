//! Streaming SHA-256. Files are never loaded whole into memory.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::error::{MfError, Result};
use crate::util::Control;

/// Read buffer size. 4 MiB keeps syscall overhead low on fast NVMe/CFexpress
/// while staying small enough to run several verifications in parallel.
pub const CHUNK: usize = 4 * 1024 * 1024;

/// A SHA-256 digest, lower-case hex when serialized.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sha256Hex(String);

impl Sha256Hex {
    pub fn from_bytes(b: &[u8]) -> Self {
        Sha256Hex(hex::encode(b))
    }

    /// Parse and validate a hex digest (64 lower/upper hex characters).
    pub fn parse(s: &str) -> Result<Self> {
        if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(Sha256Hex(s.to_ascii_lowercase()))
        } else {
            Err(MfError::InvalidInput(format!(
                "not a SHA-256 digest: {s:?}"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", &self.0[..12.min(self.0.len())])
    }
}

impl fmt::Display for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Incremental hasher used by the copy pipeline (hash while copying).
#[derive(Default)]
pub struct Hasher(Sha256);

impl Hasher {
    pub fn new() -> Self {
        Self(Sha256::new())
    }
    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }
    pub fn finish(self) -> Sha256Hex {
        Sha256Hex::from_bytes(&self.0.finalize())
    }
}

/// Hash a whole file by streaming it. `on_bytes` is called after each chunk
/// with the number of bytes just read (for progress reporting).
pub fn hash_file(
    path: &Path,
    control: &Control,
    mut on_bytes: impl FnMut(u64),
) -> Result<(Sha256Hex, u64)> {
    let mut f = File::open(path).map_err(|e| MfError::io(path, e))?;
    hash_reader(&mut f, path, control, &mut on_bytes)
}

pub fn hash_reader(
    r: &mut impl Read,
    path: &Path,
    control: &Control,
    on_bytes: &mut impl FnMut(u64),
) -> Result<(Sha256Hex, u64)> {
    let mut h = Hasher::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        control.checkpoint()?;
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(MfError::io(path, e)),
        };
        h.update(&buf[..n]);
        total += n as u64;
        on_bytes(n as u64);
    }
    Ok((h.finish(), total))
}

pub fn hash_bytes(data: &[u8]) -> Sha256Hex {
    let mut h = Hasher::new();
    h.update(data);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(
            hash_bytes(b"").as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash_bytes(b"abc").as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn streaming_matches_oneshot_across_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.bin");
        let data: Vec<u8> = (0..(CHUNK * 2 + 123)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let mut seen = 0;
        let (h, n) = hash_file(&p, &Control::new(), |b| seen += b).unwrap();
        assert_eq!(n as usize, data.len());
        assert_eq!(seen as usize, data.len());
        assert_eq!(h, hash_bytes(&data));
    }

    #[test]
    fn parse_validates() {
        assert!(Sha256Hex::parse("zz").is_err());
        let h = hash_bytes(b"x");
        assert_eq!(Sha256Hex::parse(&h.as_str().to_uppercase()).unwrap(), h);
    }

    #[test]
    fn cancel_stops_hashing() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.bin");
        std::fs::write(&p, b"data").unwrap();
        let c = Control::new();
        c.cancel();
        assert!(matches!(hash_file(&p, &c, |_| {}), Err(MfError::Cancelled)));
    }
}
