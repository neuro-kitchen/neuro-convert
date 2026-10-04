//! Where a session came from: format, files, checksums, warnings.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where a session came from, carried into outputs and conversion reports.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// Input format name (`tdt`, `spikeglx`, …).
    pub format: String,
    /// Detected format version (e.g. `Synapse 53575`).
    pub version: Option<String>,
    /// Every file read.
    pub files: Vec<SourceFile>,
    /// The reader that read it and its version, `<name> <version>` (e.g. `blackrock 0.1.0`); set
    /// by `nc_convert::Registry::open`.
    pub reader: String,
    /// Recoverable problems met while reading (damaged index, missing files, …).
    pub warnings: Vec<String>,
}

/// One file read from the source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    /// Path as opened.
    pub path: PathBuf,
    /// Size in bytes.
    pub bytes: u64,
    /// Checksum the source records for this file (e.g. SpikeGLX `fileSHA1`), checked before
    /// converting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<Checksum>,
}

/// A file checksum recorded by the source format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checksum {
    /// `sha1`.
    pub algorithm: String,
    /// Lowercase hex.
    pub value: String,
}

impl Provenance {
    /// Provenance for `format`; the reader name is set later by the registry.
    pub fn new(format: &str) -> Self {
        Self { format: format.into(), reader: format!("neuro-convert {}", env!("CARGO_PKG_VERSION")), ..Default::default() }
    }

    /// Records a file read (with its size).
    pub fn add_file(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        let bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
        self.files.push(SourceFile { path, bytes, checksum: None });
    }

    /// Records the checksum the source declares for `path` (a file added with
    /// [`add_file`](Self::add_file)).
    pub fn set_checksum(&mut self, path: &std::path::Path, algorithm: &str, value: &str) {
        if let Some(f) = self.files.iter_mut().find(|f| f.path == path) {
            f.checksum = Some(Checksum { algorithm: algorithm.into(), value: value.to_ascii_lowercase() });
        }
    }
}
