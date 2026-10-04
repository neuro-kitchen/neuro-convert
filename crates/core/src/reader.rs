//! What a format crate implements: recognizing its files and opening them into a [`Session`].
//! Readers are collected by `nc-convert`'s registry; nothing here knows which ones exist.

use std::path::Path;

use nc_base::Result;

use crate::options::OpenOptions;
use crate::session::Session;

/// How far a reader has been checked (shown by `neuro-convert formats` and the app).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Maturity {
    /// Its reference comparison passes on real data (values, times, spikes, events against the
    /// vendor's reader or neo, through the whole pipeline; see docs/readers/README.md §4).
    Verified,
    /// Works on its fixtures; not yet compared with a reference reader on real data.
    Experimental,
    /// Maintained outside neuro-convert.
    Community,
}

impl Maturity {
    /// Lowercase name (`verified`).
    pub fn label(&self) -> &'static str {
        match self {
            Maturity::Verified => "verified",
            Maturity::Experimental => "experimental",
            Maturity::Community => "community",
        }
    }

    /// One line for users.
    pub fn explain(&self) -> &'static str {
        match self {
            Maturity::Verified => "Values, times and spikes checked against a reference reader on real recordings",
            Maturity::Experimental => "Works on test files; not yet checked against a reference reader",
            Maturity::Community => "Maintained outside neuro-convert",
        }
    }
}

/// A reader for one family of files. `Send + Sync` so programs can open recordings on a
/// background thread.
pub trait Reader: Send + Sync {
    /// Short id used on the command line (`tdt`).
    fn name(&self) -> &'static str;
    /// The reader's own version (its crate's `VERSION`), recorded in each session's provenance
    /// so a conversion can be traced to the reader code that read it.
    fn version(&self) -> &'static str;
    /// One line: what the format is.
    fn description(&self) -> &'static str;
    /// What a user selects to open this format, in a sentence (shown by front ends).
    fn opens(&self) -> &'static str {
        ""
    }
    /// How far it has been checked; [`Maturity::Verified`] only once its reference comparison
    /// passes (a new reader starts experimental).
    fn maturity(&self) -> Maturity {
        Maturity::Experimental
    }
    /// Human-readable list of the versions / variants this reader handles.
    fn versions(&self) -> &'static [&'static str];
    /// Whether `path` looks like this format.
    fn detect(&self, path: &Path) -> Option<Detection>;
    /// When `path` holds several recordings (a TDT tank's blocks, a folder with several
    /// SpikeGLX runs): their names, to pass as [`OpenOptions::block`]. Empty when `path` is a
    /// single recording.
    fn containers(&self, _path: &Path) -> Vec<String> {
        Vec::new()
    }
    /// Reads `path` into a [`Session`]. Continuous data and snippet waveforms are not read here, only served on demand.
    fn open(&self, path: &Path, options: &OpenOptions) -> Result<Session>;
}

/// A reader's claim on a path.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    /// The reader's [`Reader::name`].
    pub format: &'static str,
    /// Format version found in the files, when they record one.
    pub version: Option<String>,
    /// 0..1; the highest claim wins.
    pub confidence: f32,
}
