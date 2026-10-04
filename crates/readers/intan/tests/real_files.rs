//! The Intan reader on neo's public test files (GIN `NeuralEnsemble/ephy_testing_data/intan`):
//! traditional RHD 1.5 and RHS 1.0 files, RHX 3.3 one-file-per-signal-type (RHD, RHS) and
//! one-file-per-channel (RHD). Skipped when `<data>/raw/intan` is missing (`$NC_DATA_DIR` or the
//! workspace's git-ignored `data/`). Every value is cross-checked against neo by
//! `python tools/python/compare intan`.

use std::path::{Path, PathBuf};

use nc_core::OpenOptions;
use nc_intan::Intan;

fn data_dir() -> PathBuf {
    std::env::var_os("NC_DATA_DIR").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data"), PathBuf::from).join("raw/intan")
}

fn names(s: &nc_core::Session) -> Vec<String> {
    let mut v: Vec<String> = s.recordings.iter().map(|r| r.info().name.clone()).collect();
    v.sort();
    v
}

#[test]
fn reads_every_layout() {
    let dir = data_dir();
    if !dir.exists() {
        eprintln!("skipped: {} not found (set NC_DATA_DIR)", dir.display());
        return;
    }
    let cases: [(&str, &[&str], usize, u64); 5] = [
        ("intan_rhd_test_1.rhd", &["amplifier", "analog_in", "supply"], 192, 30_000),
        ("intan_rhs_test_1.rhs", &["amplifier", "analog_in", "analog_out", "stim"], 32, 64_000),
        ("intan_fps_test_231117_052500", &["amplifier", "analog_in", "aux"], 64, 24_320),
        ("intan_fpc_test_231117_052630", &["amplifier", "analog_in", "aux"], 64, 24_320),
        ("intan_fps_rhs_test_240329_091536", &["amplifier", "analog_in", "analog_out", "dc_amplifier", "stim"], 64, 57_088),
    ];
    for (name, streams, channels, samples) in cases {
        let path = dir.join(name);
        // Detection, invariants and read consistency at the start, middle and end
        let s = nc_core::testkit::check_reader(&Intan, &path, &OpenOptions::default());
        assert_eq!(names(&s), streams.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{name}");
        let amp = s.recording("amplifier").unwrap().info();
        assert_eq!((amp.channel_count(), amp.samples, amp.sample_rate, amp.unit.as_str()), (channels, samples, 30_000.0, "V"), "{name}");
        assert_eq!(amp.channels[0].gain, 0.195e-6);
        // One electrode per amplifier channel, grouped by port
        assert_eq!(s.electrodes.len(), channels, "{name}");
        assert!(s.electrode_groups.iter().all(|g| g.name.starts_with("port ")));
        assert!(s.provenance.warnings.iter().all(|w| w.contains("start time")), "{name}: {:?}", s.provenance.warnings);
    }

    // Start time from the RHX folder name; one event series per digital line that went high
    let a = nc_core::testkit::check_reader(&Intan, &dir.join("intan_fps_test_231117_052500"), &OpenOptions::default());
    assert_eq!(a.metadata.start_time.as_deref(), Some("2023-11-17T05:25:00"));
    assert_eq!(a.events.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["DIGITAL-IN-01"]);
    // RHS: DC amplifier channels share the amplifier electrodes
    let rhs = nc_core::testkit::check_reader(&Intan, &dir.join("intan_fps_rhs_test_240329_091536"), &OpenOptions::default());
    assert_eq!(rhs.electrodes[0].channels.iter().map(|c| c.recording.as_str()).collect::<Vec<_>>(), ["amplifier", "dc_amplifier"]);
    let mut v = vec![0f32; 4];
    rhs.recording("dc_amplifier").unwrap().read(&[0], 0..4, &mut v).unwrap();
    assert!(v.iter().all(|x| x.abs() < 10.0), "DC amplifier within ±10 V: {v:?}");
}
