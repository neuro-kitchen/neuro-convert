//! Finding the files of one SpikeGLX run.
//!
//! SpikeGLX names files `<run>_g<gate>_t<trigger>.<stream>[.<band>].{meta,bin}`, e.g.
//! `myrun_g0_t0.imec0.ap.bin`, `myrun_g0_t0.imec0.lf.bin`, `myrun_g0_t0.nidq.bin` (3A:
//! `.imec.ap`; OneBox: `.obx0.obx`; CatGT output: `_tcat`). Since 2019 each gate has a run folder
//! (`myrun_g0/`) with optional per-probe folders (`myrun_g0_imec0/`). Renamed files (e.g. IBL's
//! `imec_385_100s.ap.bin`) keep the band suffix only; the stream then comes from the metadata.
//!
//! A run is one gate: its triggers (`_t0`, `_t1`, … — separate files for the trigger windows
//! of one acquisition) are opened together.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One `.meta` and the parts of its name.
#[derive(Debug, Clone, PartialEq)]
pub struct MetaFile {
    pub path: PathBuf,
    /// `<run>_g<gate>` (everything before the trigger and stream tokens).
    pub run: String,
    /// `0`, `1`, … or `cat` (CatGT) when the name has a `_t<trigger>` token.
    pub trigger: Option<String>,
    /// `imec0`, `nidq`, `obx0`, … when the name says so.
    pub stream: Option<String>,
    /// `ap` or `lf` when the name says so.
    pub band: Option<String>,
}

impl MetaFile {
    pub fn parse(path: &Path) -> Option<Self> {
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("meta")) {
            return None;
        }
        let stem = path.file_stem()?.to_string_lossy().into_owned();
        let mut parts: Vec<&str> = stem.split('.').collect();
        let band = parts.last().filter(|b| matches!(**b, "ap" | "lf")).map(|b| b.to_string());
        if band.is_some() {
            parts.pop();
        }
        // OneBox repeats its type after the stream (`.obx0.obx`)
        if parts.len() > 2 && parts.last() == Some(&"obx") {
            parts.pop();
        }
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        let is_stream = |p: &str| p == "nidq" || p == "imec" || p.strip_prefix("imec").is_some_and(digits) || p.strip_prefix("obx").is_some_and(digits);
        let stream = parts.last().filter(|p| parts.len() > 1 && is_stream(p)).map(|p| p.to_string());
        if stream.is_some() {
            parts.pop();
        }
        // 3A writes `.imec` for its only probe
        let stream = stream.map(|s| if s == "imec" { "imec0".to_string() } else { s });
        let mut run = parts.join(".");
        let trigger = run.rsplit_once("_t").filter(|(_, t)| *t == "cat" || digits(t)).map(|(r, t)| (r.to_string(), t.to_string()));
        let trigger = trigger.map(|(r, t)| {
            run = r;
            t
        });
        Some(Self { path: path.to_path_buf(), run, trigger, stream, band })
    }

    /// The data file next to this `.meta`.
    pub fn bin(&self) -> PathBuf {
        self.path.with_extension("bin")
    }
}

/// `.meta` files in `dir` and its sub-folders, up to `depth` levels down.
fn metas_under(dir: &Path, depth: usize, out: &mut Vec<MetaFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            if depth > 0 {
                metas_under(&p, depth - 1, out);
            }
        } else if let Some(m) = MetaFile::parse(&p) {
            out.push(m);
        }
    }
}

/// Runs (gates) reachable from `path`: a `.meta` / `.bin` file selects its own run among its
/// folder's files (and its probe folders' siblings); a folder contributes every run found in it
/// and two levels below (run and probe folders).
pub fn runs(path: &Path) -> BTreeMap<String, Vec<MetaFile>> {
    let mut found = Vec::new();
    let file_run = if path.is_file() {
        let meta = path.with_extension("meta");
        let own = MetaFile::parse(&meta).filter(|_| meta.exists());
        // The run folder: a file in a probe folder (`run_g0_imec0/`) reaches its siblings
        let dir = path.parent().unwrap_or(Path::new("."));
        let probe_folder = dir.file_name().is_some_and(|n| n.to_string_lossy().contains("_imec"));
        let root = if probe_folder { dir.parent().unwrap_or(dir) } else { dir };
        metas_under(root, if probe_folder { 1 } else { 0 }, &mut found);
        own.map(|m| m.run)
    } else {
        metas_under(path, 2, &mut found);
        None
    };
    let mut runs: BTreeMap<String, Vec<MetaFile>> = BTreeMap::new();
    for m in found.into_iter().filter(|m| file_run.as_ref().is_none_or(|r| *r == m.run)) {
        runs.entry(m.run.clone()).or_default().push(m);
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_names() {
        let m = MetaFile::parse(Path::new("x/myrun_g0_t0.imec1.ap.meta")).unwrap();
        assert_eq!((m.run.as_str(), m.trigger.as_deref(), m.stream.as_deref(), m.band.as_deref()), ("myrun_g0", Some("0"), Some("imec1"), Some("ap")));
        let m = MetaFile::parse(Path::new("myRun_g0_t0.obx0.obx.meta")).unwrap();
        assert_eq!((m.run.as_str(), m.stream.as_deref(), m.band), ("myRun_g0", Some("obx0"), None));
        let m = MetaFile::parse(Path::new("5-19-2022-CI0_g0_tcat.imec0.lf.meta")).unwrap();
        assert_eq!((m.run.as_str(), m.trigger.as_deref()), ("5-19-2022-CI0_g0", Some("cat")));
        let m = MetaFile::parse(Path::new("myrun_g0_t0.imec.lf.meta")).unwrap();
        assert_eq!((m.stream.as_deref(), m.band.as_deref()), (Some("imec0"), Some("lf")));
        let m = MetaFile::parse(Path::new("myrun_g2_t1.nidq.meta")).unwrap();
        assert_eq!((m.run.as_str(), m.trigger.as_deref(), m.stream.as_deref(), m.band), ("myrun_g2", Some("1"), Some("nidq"), None));
        // Renamed (IBL): only the band survives
        let m = MetaFile::parse(Path::new("imec_385_100s.ap.meta")).unwrap();
        assert_eq!((m.run.as_str(), m.trigger.as_deref(), m.stream.as_deref(), m.band.as_deref()), ("imec_385_100s", None, None, Some("ap")));
        assert_eq!(m.bin(), Path::new("imec_385_100s.ap.bin"));
        assert!(MetaFile::parse(Path::new("notes.txt")).is_none());
    }
}
