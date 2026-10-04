//! The SpikeGLX reader on a real Neuropixels 3A AP file (IBL `imec_385_100s`: 384 AP + 1 sync
//! channel, 30 kHz, 100 s). Skipped when the file is not on disk (2.3 GB, not committed).
//!
//! Looked up as `<data>/raw/spikeglx/imec_385_100s`, where `<data>` is `$NC_DATA_DIR` or the workspace's
//! git-ignored `data/` folder. Expected values are cross-checked with SpikeGLX's own conversion
//! (`readSGLX.py`) and probeinterface by `python tools/python/compare spikeglx`.

use std::path::{Path, PathBuf};

use nc_core::OpenOptions;
use nc_spikeglx::SpikeGlx;

fn data_dir() -> PathBuf {
    std::env::var_os("NC_DATA_DIR").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data"), PathBuf::from)
}

#[test]
fn reads_ibl_neuropixels_3a() {
    let path = data_dir().join("raw/spikeglx/imec_385_100s");
    if !path.exists() {
        eprintln!("skipped: {} not found (set NC_DATA_DIR)", path.display());
        return;
    }
    // Detection, invariants and read consistency at the start, middle and end
    let s = nc_core::testkit::check_reader(&SpikeGlx, &path, &OpenOptions::default());
    assert!(s.provenance.warnings.is_empty(), "{:?}", s.provenance.warnings);
    assert_eq!(s.provenance.version.as_deref(), Some("SpikeGLX 20180829"));

    let names: Vec<String> = s.recordings.iter().map(|r| r.info().name.clone()).collect();
    assert_eq!(names, vec!["imec0.ap", "imec0.ap.sync"]);
    let ap = s.recording("imec0.ap").unwrap().info();
    assert_eq!((ap.channel_count(), ap.samples, ap.sample_rate, ap.unit.as_str()), (384, 3_000_000, 30_000.0, "V"));
    // 3A: 0.6 V range, 10-bit (512), AP gain 500 → 2.34375 µV per bit
    assert_eq!(ap.channels[0].gain, 0.6 / 512.0 / 500.0);

    // 384 electrodes on one shank; NP1.0 staggered layout, 20 µm rows
    assert_eq!(s.electrodes.len(), 384);
    let pos: Vec<[f32; 3]> = s.electrodes.iter().take(4).map(|e| e.position_um.unwrap()).collect();
    assert_eq!(pos, vec![[27.0, 0.0, 0.0], [59.0, 0.0, 0.0], [11.0, 20.0, 0.0], [43.0, 20.0, 0.0]]);
    assert_eq!(s.electrodes[383].position_um, Some([43.0, 3820.0, 0.0]));
    assert_eq!(s.metadata.start_time.as_deref(), Some("2019-05-07T17:24:02"));
    assert_eq!(s.metadata.devices[0].model.as_deref(), Some("3A (option 3)"));
    // The .meta's fileSHA1 is recorded for the .bin (this file's content no longer matches it:
    // sha1sum gives 90cf1769…, see .knowledge/status.md)
    let bin = s.provenance.files.iter().find(|f| f.path.extension().is_some_and(|e| e == "bin")).unwrap();
    assert_eq!(bin.checksum.as_ref().map(|c| (c.algorithm.as_str(), c.value.as_str())), Some(("sha1", "d4ce63afa12937a1904d344b93c90b64573782d3")));
}
