//! Read-only memory-mapped files.

use std::fs::File;
use std::path::Path;

use memmap2::Mmap;

use crate::error::{Error, Result};

/// A whole file mapped read-only.
pub struct MappedFile {
    map: Mmap,
}

impl std::fmt::Debug for MappedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedFile").field("bytes", &self.map.len()).finish()
    }
}

impl MappedFile {
    /// Maps the whole file at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| Error::io(path, e))?;
        // SAFETY: read-only map; like every mmap reader we assume the recording is not
        // truncated by another process while it is open.
        let map = unsafe { Mmap::map(&file) }.map_err(|e| Error::io(path, e))?;
        Ok(Self { map })
    }

    /// The file's bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.map
    }

    /// File size in bytes.
    pub fn len(&self) -> u64 {
        self.map.len() as u64
    }

    /// `true` for an empty file.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
