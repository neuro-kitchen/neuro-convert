//! Which TDT software wrote a block.
//!
//! - **Synapse**: writes `Notes.txt`, `StoresListing.txt` and a zipped `.tin` whose
//!   `Summary.txt` carries the build number (`Versions.Synapse`).
//! - **OpenEx**: writes a `.tnt` note file and no Synapse sidecars.
//!
//! SEV header versions (0–4) are per file and handled with the SEV reader.

use std::fmt;

/// The acquisition software.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Software {
    /// Synapse.
    Synapse {
        /// Build number from the `.tin` (`Versions.Synapse`).
        build: Option<String>,
    },
    /// OpenEx.
    OpenEx,
    /// Neither found.
    Unknown,
}

/// The software and note-file version of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TdtVersion {
    /// Synapse or OpenEx.
    pub software: Software,
    /// `NOTEFILE_VERSION` from the `.tnt`, when present.
    pub notefile: Option<String>,
}

impl TdtVersion {
    /// Synapse when its sidecars or a build number exist, OpenEx when only a `.tnt` note file does.
    pub fn detect(has_synapse_sidecars: bool, synapse_build: Option<String>, notefile: Option<String>) -> Self {
        let software = if has_synapse_sidecars || synapse_build.is_some() {
            Software::Synapse { build: synapse_build }
        } else if notefile.is_some() {
            Software::OpenEx
        } else {
            Software::Unknown
        };
        Self { software, notefile }
    }
}

impl fmt::Display for TdtVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.software {
            Software::Synapse { build: Some(b) } => write!(f, "Synapse {b}"),
            Software::Synapse { build: None } => write!(f, "Synapse"),
            Software::OpenEx => write!(f, "OpenEx"),
            Software::Unknown => write!(f, "unknown TDT software"),
        }
    }
}
