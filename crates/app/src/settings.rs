//! User preferences and recent files, kept as JSON in the user's config folder
//! (`$XDG_CONFIG_HOME/neuro-convert/settings.json`, else `~/.config/neuro-convert/`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many recent recordings / metadata files are kept.
const RECENT: usize = 8;
/// How many typed values are remembered per field.
const REMEMBERED: usize = 12;

/// Which side panels are open (remembered).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Panels {
    /// Contents: the tree of the recording.
    pub tree: bool,
    /// Contents: the settings of the selected item.
    pub inspector: bool,
    /// Review: the NWB structure.
    pub structure: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Self { tree: true, inspector: true, structure: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where outputs go by default; `None` = next to the recording.
    pub output_dir: Option<PathBuf>,
    /// gzip level for new conversions (`None` = uncompressed).
    pub gzip: Option<u32>,
    /// ~10 MB chunks (`auto`) instead of 1 s.
    pub auto_chunks: bool,
    /// How much written data is compared with the source (sampled: fast enough to always run).
    pub verify: nc_convert::nwb::VerifyLevel,
    /// CPUs left free for the UI while converting.
    pub reserved_threads: usize,
    /// `light` / `dark`; `None` follows the system.
    pub theme: Option<String>,
    pub recent_recordings: Vec<PathBuf>,
    pub recent_metadata: Vec<PathBuf>,
    /// The user plans to upload to DANDI: its recommendations count as issues.
    pub dandi: bool,
    pub panels: Panels,
    /// Values typed before, per field (`lab`, `species`, `location`, …), newest first.
    pub remembered: BTreeMap<String, Vec<String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { output_dir: None, gzip: Some(1), auto_chunks: false, verify: nc_convert::nwb::VerifyLevel::Sampled, reserved_threads: 2, theme: None, recent_recordings: Vec::new(), recent_metadata: Vec::new(), dandi: true, panels: Panels::default(), remembered: BTreeMap::new() }
    }
}

impl Settings {
    /// The settings file, when a config folder can be found.
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("neuro-convert").join("settings.json"))
    }

    /// Loads from `path`; defaults when it is missing or unreadable.
    pub fn load(path: Option<&Path>) -> Self {
        path.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).expect("settings are plain data"))
    }

    pub fn remember_recording(&mut self, path: &Path) {
        remember(&mut self.recent_recordings, path);
    }

    pub fn remember_metadata(&mut self, path: &Path) {
        remember(&mut self.recent_metadata, path);
    }

    /// Remembers a typed value of `field` (empty values are ignored).
    pub fn remember_value(&mut self, field: &str, value: &str) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        let list = self.remembered.entry(field.to_string()).or_default();
        list.retain(|v| v != value);
        list.insert(0, value.to_string());
        list.truncate(REMEMBERED);
    }

    pub fn remembered(&self, field: &str) -> &[String] {
        self.remembered.get(field).map_or(&[], Vec::as_slice)
    }
}

/// Moves `path` to the front, without duplicates, keeping at most [`RECENT`].
fn remember(list: &mut Vec<PathBuf>, path: &Path) {
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(RECENT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_trip_and_recent_lists() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/app-test/settings");
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("settings.json");
        assert_eq!(Settings::load(Some(&file)), Settings::default(), "missing file → defaults");

        let mut s = Settings { gzip: None, theme: Some("dark".into()), ..Default::default() };
        for i in 0..10 {
            s.remember_recording(Path::new(&format!("r{i}")));
        }
        s.remember_recording(Path::new("r5"));
        assert_eq!(s.recent_recordings.len(), RECENT);
        assert_eq!(s.recent_recordings[0], Path::new("r5"));
        assert_eq!(s.recent_recordings.iter().filter(|p| *p == Path::new("r5")).count(), 1);

        s.remember_value("lab", "Lab A");
        s.remember_value("lab", " ");
        s.remember_value("lab", "Lab B");
        s.remember_value("lab", "Lab A");
        assert_eq!(s.remembered("lab"), ["Lab A", "Lab B"]);
        s.save(&file).unwrap();
        assert_eq!(Settings::load(Some(&file)), s);
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(Settings::load(Some(&file)), Settings::default(), "unreadable → defaults");
    }
}
