//! The Open Ephys reader on neo's public test recordings (GIN
//! `NeuralEnsemble/ephy_testing_data/openephysbinary`): GUI 0.6.0 Neuropixels + NI-DAQ with sync
//! lines and TTL / message events, 0.5.0 with two record nodes × three recordings, 0.4.5 headstage +
//! ADC channels. Skipped when `<data>/raw/openephys` is missing. Every value is cross-checked
//! against neo by `tools/python/compare_openephys.py`.

use std::path::{Path, PathBuf};

use nc_core::{OpenOptions, Reader as _};
use nc_openephys::OpenEphys;

fn data_dir() -> PathBuf {
    std::env::var_os("NC_DATA_DIR").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data"), PathBuf::from).join("raw/openephys")
}

fn names(s: &nc_core::Session) -> Vec<String> {
    s.recordings.iter().map(|r| r.info().name.clone()).collect()
}

#[test]
fn reads_neuropixels_with_sync() {
    let path = data_dir().join("v0.6.x_neuropixels_with_sync");
    if !path.exists() {
        eprintln!("skipped: {} not found (set NC_DATA_DIR)", path.display());
        return;
    }
    let s = nc_core::testkit::check_reader(&OpenEphys, &path, &OpenOptions::default());
    assert_eq!(names(&s), ["PXIe-6341.analog", "ProbeA-AP", "ProbeA-AP.sync", "ProbeA-LFP", "ProbeA-LFP.sync"]);
    let ap = s.recording("ProbeA-AP").unwrap().info();
    assert_eq!((ap.channel_count(), ap.samples, ap.sample_rate), (384, 1000, 30_000.0));
    assert!((ap.channels[0].gain - 0.195e-6).abs() < 1e-12, "bit_volts in µV → volts");
    // Synchronized timestamps put the NI-DAQ first and the probe 1.755 ms later
    assert!((ap.start_time - 0.001755).abs() < 1e-5, "{}", ap.start_time);
    assert_eq!(s.recording("PXIe-6341.analog").unwrap().info().start_time, 0.0);
    // AP and LFP share 384 electrodes placed from settings.xml
    assert_eq!(s.electrodes.len(), 384);
    assert_eq!(s.electrodes[383].position_um, Some([59.0, 3820.0, 0.0]));
    assert_eq!(s.electrodes[0].channels.len(), 2);
    // UTC wall clock from sync_messages.txt
    assert_eq!(s.metadata.start_time.as_deref(), Some("2023-08-31T06:41:36.435000Z"));
    let ttl = s.event_series("PXIe-6341 TTL 2").unwrap();
    assert_eq!((ttl.len(), ttl.offsets.as_ref().unwrap().len()), (560, 560));
    assert_eq!(s.event_series("messages").unwrap().labels[0], "NP OPTO 5 2 1 blue 14");
}

#[test]
fn reads_record_nodes_and_old_formats() {
    let dir = data_dir();
    if !dir.exists() {
        eprintln!("skipped: {} not found", dir.display());
        return;
    }
    let two = dir.join("v0.5.x_two_nodes");
    let containers = OpenEphys.containers(&two);
    assert_eq!(containers.len(), 6);
    assert_eq!(containers[0], "RecordNode103/experiment1/recording1");
    assert!(OpenEphys.open(&two, &OpenOptions::default()).is_err(), "several recordings: choose one");
    let options = OpenOptions { block: Some("RecordNode105/experiment1/recording3".into()), ..Default::default() };
    let s = nc_core::testkit::check_reader(&OpenEphys, &two, &options);
    assert_eq!(names(&s), ["File_Reader-100.0"]);
    assert_eq!(s.recording("File_Reader-100.0").unwrap().info().samples, 20_000);

    let mixed = nc_core::testkit::check_reader(&OpenEphys, &dir.join("neural_and_non_neural_data_mixed"), &OpenOptions::default());
    assert_eq!(names(&mixed), ["Rhythm_FPGA-100.0", "Rhythm_FPGA-100.0.analog"]);
    let adc = mixed.recording("Rhythm_FPGA-100.0.analog").unwrap().info();
    assert_eq!(adc.channel_count(), 2);
    assert!((adc.channels[0].gain - 0.000152588).abs() < 1e-9, "ADC bit_volts are volts: {}", adc.channels[0].gain);
    assert_eq!(mixed.metadata.start_time.as_deref(), Some("2022-07-25T15:30:00"), "settings.xml date");
}
