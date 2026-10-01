//! What a format crate implements: recognizing its files and opening them into a [`Session`].
//! Readers are collected by `nc-convert`'s registry; nothing here knows which ones exist.

use std::path::Path;

use nc_base::Result;

use crate::options::OpenOptions;
use crate::session::Session;

/// A reader for one family of files.
pub trait Reader: Sync {
    /// Short id used on the command line (`tdt`).
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// Human-readable list of the versions / variants this reader handles.
    fn versions(&self) -> &'static [&'static str];
    /// Whether `path` looks like this format.
    fn detect(&self, path: &Path) -> Option<Detection>;
    fn open(&self, path: &Path, options: &OpenOptions) -> Result<Session>;
}

/// A reader's claim on a path.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub format: &'static str,
    pub version: Option<String>,
    /// 0..1; the highest claim wins.
    pub confidence: f32,
}
