//! Source integrity: before converting, files whose format records a checksum (SpikeGLX
//! `fileSHA1`) are hashed and compared, so a truncated or corrupted recording is caught before
//! it becomes a valid-looking NWB file.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use nc_core::{Error, Provenance, Result};
use nc_nwb::Progress;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

/// The result of checking one source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceCheck {
    /// The file.
    pub path: PathBuf,
    /// `sha1`.
    pub algorithm: String,
    /// Checksum the source records (lowercase hex).
    pub expected: String,
    /// Checksum of the file as read (lowercase hex).
    pub actual: String,
}

impl SourceCheck {
    /// `true` when the checksums match.
    pub fn ok(&self) -> bool {
        self.expected == self.actual
    }
}

/// Bytes of the files `provenance` has checksums for.
pub fn checked_bytes(provenance: &Provenance) -> u64 {
    provenance.files.iter().filter(|f| f.checksum.is_some()).map(|f| f.bytes).sum()
}

/// Hashes every file with a recorded checksum (`progress` counts bytes; `cancel` stops between
/// reads with [`Error::Cancelled`]). Unknown algorithms are skipped.
pub fn check(provenance: &Provenance, cancel: &AtomicBool, progress: &dyn Fn(Progress)) -> Result<Vec<SourceCheck>> {
    let started = Instant::now();
    let total = checked_bytes(provenance);
    let mut done = 0u64;
    let mut last = Instant::now();
    let mut out = Vec::new();
    let mut buf = vec![0u8; 8 << 20];
    for f in &provenance.files {
        let Some(c) = f.checksum.as_ref().filter(|c| c.algorithm == "sha1") else { continue };
        let mut file = std::fs::File::open(&f.path).map_err(|e| Error::io(&f.path, e))?;
        let mut h = Sha1::new();
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let n = file.read(&mut buf).map_err(|e| Error::io(&f.path, e))?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            done += n as u64;
            if last.elapsed().as_millis() >= 250 {
                progress(Progress { done, total, elapsed: started.elapsed() });
                last = Instant::now();
            }
        }
        let actual: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        out.push(SourceCheck { path: f.path.clone(), algorithm: c.algorithm.clone(), expected: c.value.clone(), actual });
    }
    progress(Progress { done: total, total, elapsed: started.elapsed() });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use nc_core::Provenance;

    use super::check;

    #[test]
    fn test_sha1_match_and_mismatch() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sources-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("abc.bin");
        std::fs::write(&path, b"abc").unwrap();
        let mut p = Provenance::new("test");
        p.add_file(&path);
        // SHA-1("abc"), upper case as SpikeGLX writes it
        p.set_checksum(&path, "sha1", "A9993E364706816ABA3E25717850C26C9CD0D89D");
        let checks = check(&p, &AtomicBool::new(false), &|_| {}).unwrap();
        assert!(checks[0].ok(), "{checks:?}");
        p.set_checksum(&path, "sha1", "0000000000000000000000000000000000000000");
        assert!(!check(&p, &AtomicBool::new(false), &|_| {}).unwrap()[0].ok());
    }
}
