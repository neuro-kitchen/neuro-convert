//! NWB output on a small synthetic session. Writes `<workspace>/target/nwb-test/small.nwb.zarr`,
//! which `tools/python/validate_nwb.py` reads back with pynwb + hdmf-zarr (from the workspace root):
//! `uv run --no-project --with pynwb --with hdmf-zarr --with nwbinspector tools/python/validate_nwb.py target/nwb-test/small.nwb.zarr --small`

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nc_core::{Device, EventSeries, MemoryRecording, MetadataFile, Session, SnippetSeries, Table};
use nc_nwb::{self as nwb, NwbOptions};

/// `<workspace>/target/nwb-test`: stores are kept for the Python read-back.
fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/nwb-test")
}

fn session() -> Session {
    let mut s = Session::default();
    s.metadata.start_time = Some("2025-02-26T15:25:56".into());
    s.metadata.experiment = Some("synthetic".into());
    s.metadata.subject.id = Some("rat1".into());
    s.metadata.devices = vec![Device { name: "RZ2(1)".into(), description: "RZn Processor".into(), manufacturer: Some("TDT".into()), model: Some("RZ2".into()) }];

    // 3-channel "EMG" (value = ch * 1000 + t) and a 1-channel "Temp", 1000 samples each
    let emg: Vec<f32> = (0..3).flat_map(|c| (0..1000).map(move |t| (c * 1000 + t) as f32)).collect();
    s.recordings.push(Arc::new(MemoryRecording::new("EMG1", emg, 3, 1000.0, "V").unwrap()));
    s.recordings.push(Arc::new(MemoryRecording::new("Temp", (0..1000).map(|t| t as f32 * 0.5).collect(), 1, 100.0, "a.u.").unwrap()));
    s.recordings.push(Arc::new(MemoryRecording::new("Skip", vec![0.0; 10], 1, 10.0, "a.u.").unwrap()));

    s.events.push(EventSeries { name: "MET/".into(), onsets: vec![0.5, 1.5], offsets: Some(vec![0.6, 1.6]), values: vec![1.0, 2.0], channels: 1, ..Default::default() });
    s.events.push(EventSeries { name: "Tick".into(), onsets: vec![0.0, 1.0, 2.0], values: vec![0.0, 1.0, 2.0], channels: 1, ..Default::default() });
    s.events.push(EventSeries {
        name: "Note".into(),
        onsets: vec![0.2, 0.9],
        values: vec![1.0, 2.0],
        channels: 1,
        labels: vec!["sleep".into(), "Bottle In".into()],
        ..Default::default()
    });
    s.events.push(EventSeries { name: "eS1p".into(), onsets: vec![0.25], values: vec![0.5, -750.0], channels: 2, ..Default::default() });
    // Sorted snippets on the EMG electrodes: channels 1, 2, 1, 3; sort codes 0, 1, 1, 2
    s.snippets.push(SnippetSeries {
        name: "eNe1".into(),
        sample_rate: 1000.0,
        samples_per_snippet: 4,
        timestamps: vec![0.1, 0.2, 0.3, 0.4],
        channels: vec![1, 2, 1, 3],
        sort_codes: vec![0, 1, 1, 2],
        data: (0..16).map(|v| v as f32).collect(),
        unit: "V".into(),
        ..Default::default()
    });
    s.tables.push(Table {
        name: "Z_EMG".into(),
        description: "impedance".into(),
        columns: vec!["TIME (S)".into(), "R1 (kOhm)".into(), "note".into()],
        rows: vec![vec!["59".into(), "0.96".into(), "ok".into()], vec!["63".into(), "-1.00".into(), "n/a".into()]],
    });
    s
}

const META: &str = r#"
session:
  description: "Synthetic session for the NWB writer test."
  timezone: "-05:00"
  lab: "Example Lab"
  institution: "Example University"
  experimenters: ["Tester"]
subject: { species: "Rattus norvegicus", sex: U, age: P90D }
electrode_groups:
  - { name: EMG, description: "3-channel EMG", location: diaphragm, impedance: { table: Z_EMG } }
streams:
  "*": { type: timeseries, unit: a.u. }
  EMG1: { type: electrical, electrode_group: EMG, name: EMG, conversion: 1.0e-6 }
  Skip: { include: false }
snippets:
  eNe1: { electrode_group: EMG }
"#;

#[test]
fn writes_small_nwb_zarr() {
    let mut s = session();
    let meta = MetadataFile::parse(META).unwrap();
    let plan = nwb::plan(&mut s, &meta, || "test-id".into());
    assert!(!plan.has_errors(), "{:?}", plan.issues);
    assert_eq!(plan.series[0].electrodes, Some(vec![0, 1, 2]), "electrodes come from the session after apply");
    assert_eq!(plan.file.start_time, "2025-02-26T15:25:56-05:00");
    assert_eq!(plan.series.len(), 2);
    assert_eq!(plan.skipped, vec!["stream Skip".to_string()]);

    let dest = out_dir().join("small.nwb.zarr");
    let options = NwbOptions { overwrite: true, threads: 2, chunks: nwb::ChunkPolicy::Seconds(0.3), gzip: Some(1), ..Default::default() };
    assert_eq!(plan.snippets.len(), 1);
    assert_eq!(plan.snippets[0].rows, vec![(1, 0), (2, 1), (3, 2)], "channel c is the group's c-th electrode");
    let summary = nwb::write(&s, &plan, &dest, &options, &|_| {}).unwrap();
    for c in 1..=3 {
        let g: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dest.join(format!("acquisition/eNe1_ch{c}/zarr.json"))).unwrap()).unwrap();
        assert_eq!(g["attributes"]["neurodata_type"], "SpikeEventSeries");
    }
    let units: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dest.join("units/zarr.json")).unwrap()).unwrap();
    assert_eq!(units["attributes"]["neurodata_type"], "Units");
    let model: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dest.join("general/devices/models/RZ2/zarr.json")).unwrap()).unwrap();
    assert_eq!(model["attributes"]["manufacturer"], "TDT");
    assert_eq!(summary.samples, 3000 + 1000);

    // Spot checks of the layout hdmf-zarr expects
    let root: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dest.join("zarr.json")).unwrap()).unwrap();
    assert_eq!(root["attributes"]["neurodata_type"], "NWBFile");
    assert_eq!(root["attributes"][".specloc"], "specifications");
    let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dest.join("acquisition/EMG/data/zarr.json")).unwrap()).unwrap();
    assert_eq!(data["shape"], serde_json::json!([1000, 3]));
    assert_eq!(data["attributes"]["unit"], "volts");
    assert!(dest.join("specifications/core/2.11.0/nwb.ecephys/zarr.json").exists());
    assert!(dest.join("events/MET/timestamp").exists());
    assert!(dest.join("events/MET/duration").exists());
    assert!(dest.join("general/extracellular_ephys/electrodes/imp").exists());
    // Events without offsets: no duration column
    assert!(dest.join("events/Tick/timestamp").exists());
    assert!(!dest.join("events/Tick/duration").exists());

    let issues = nwb::validate::validate(&dest).unwrap();
    assert!(issues.is_empty(), "{issues:?}");

    // Damage a copy (the intact store stays for tests/validate_nwb.py): point the series at a
    // wrong table and drop an electrodes column
    let damaged = dest.with_file_name("damaged.nwb.zarr");
    let _ = std::fs::remove_dir_all(&damaged);
    copy_dir(&dest, &damaged);
    let dest = damaged;
    let region_meta = dest.join("acquisition/EMG/electrodes/zarr.json");
    let text = std::fs::read_to_string(&region_meta).unwrap().replace("/general/extracellular_ephys/electrodes", "/general/nowhere");
    std::fs::write(&region_meta, text).unwrap();
    std::fs::remove_dir_all(dest.join("general/extracellular_ephys/electrodes/channel_name")).unwrap();
    let issues = nwb::validate::validate(&dest).unwrap();
    let text: Vec<&str> = issues.iter().map(|i| i.message.as_str()).collect();
    assert!(text.iter().any(|m| m.contains("does not reference")), "{text:?}");
    assert!(text.iter().any(|m| m.contains("column channel_name listed but missing")), "{text:?}");
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

/// float64 source with native stored reads.
struct F64Recording {
    info: nc_core::RecordingInfo,
    data: Vec<f64>,
}

impl nc_core::Recording for F64Recording {
    fn info(&self) -> &nc_core::RecordingInfo {
        &self.info
    }
    fn read(&self, channels: &[usize], samples: std::ops::Range<u64>, out: &mut [f32]) -> nc_core::Result<()> {
        let n = (samples.end - samples.start) as usize;
        for (i, &c) in channels.iter().enumerate() {
            for t in 0..n {
                out[i * n + t] = self.data[c * self.info.samples as usize + samples.start as usize + t] as f32;
            }
        }
        Ok(())
    }
    fn read_stored(&self, channels: &[usize], samples: std::ops::Range<u64>, out: &mut [u8]) -> nc_core::Result<bool> {
        let n = (samples.end - samples.start) as usize;
        for (i, &c) in channels.iter().enumerate() {
            for t in 0..n {
                let v = self.data[c * self.info.samples as usize + samples.start as usize + t];
                out[(i * n + t) * 8..(i * n + t + 1) * 8].copy_from_slice(&v.to_le_bytes());
            }
        }
        Ok(true)
    }
}

#[test]
fn float64_sources_stay_float64() {
    let mut s = session();
    let template = MemoryRecording::new("P", vec![0.0; 2 * 500], 2, 1000.0, "a.u.").unwrap();
    let mut info = nc_core::Recording::info(&template).clone();
    info.name = "Precise".into();
    info.stored_as = nc_core::SampleType::F64;
    // Values that float32 cannot hold exactly
    let data: Vec<f64> = (0..1000).map(|i| 1.0 + i as f64 * 1e-9).collect();
    s.recordings.push(Arc::new(F64Recording { info, data: data.clone() }));

    let meta = MetadataFile::parse(META).unwrap();
    let plan = nwb::plan(&mut s, &meta, || "f64-id".into());
    assert!(!plan.has_errors(), "{:?}", plan.issues);
    let dest = out_dir().join("f64.nwb.zarr");
    nwb::write(&s, &plan, &dest, &NwbOptions { overwrite: true, threads: 2, chunks: nwb::ChunkPolicy::Seconds(0.3), gzip: None, ..Default::default() }, &|_| {}).unwrap();

    let meta_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dest.join("acquisition/Precise/data/zarr.json")).unwrap()).unwrap();
    assert_eq!(meta_json["data_type"], "float64");
    let store = std::sync::Arc::new(zarrs::filesystem::FilesystemStore::new(&dest).unwrap());
    let array = zarrs::array::Array::open(store, "/acquisition/Precise/data").unwrap();
    let written: Vec<f64> = array.retrieve_array_subset::<Vec<f64>>(&array.subset_all()).unwrap();
    // [time, channel] layout
    let data = &data;
    let expected: Vec<f64> = (0..500).flat_map(|t| (0..2).map(move |c| data[c * 500 + t])).collect();
    assert_eq!(written, expected);
}

#[test]
fn uncalibrated_streams_without_conversion_are_flagged() {
    let mut s = session();
    let template = MemoryRecording::new("MonA", vec![0.0; 100], 1, 24_414.0625, "a.u.").unwrap();
    let mut info = nc_core::Recording::info(&template).clone();
    info.stored_as = nc_core::SampleType::I16;
    info.calibration = nc_core::Calibration::Unknown { note: "stored as int16 with TDT scale \"Milli\"".into() };
    s.recordings.push(Arc::new(F64Recording { info, data: vec![0.0; 100] }));
    let meta = MetadataFile::parse(META).unwrap();
    let plan = nwb::plan(&mut s, &meta, || "id".into());
    assert!(plan.issues.iter().any(|i| i.message.contains("MonA") && i.message.contains("Milli")), "{:?}", plan.issues);

    // Declaring the conversion clears it (planning again on the same session: apply is idempotent)
    let with_conv = META.replace("  Skip: { include: false }\n", "  Skip: { include: false }\n  MonA: { conversion: 1.0e-3, unit: volts }\n");
    let plan = nwb::plan(&mut s, &MetadataFile::parse(&with_conv).unwrap(), || "id".into());
    assert!(!plan.issues.iter().any(|i| i.message.contains("MonA")), "{:?}", plan.issues);
    assert_eq!(s.electrodes.len(), 3, "re-planning does not duplicate electrodes");
}

/// A reader that knows its probe: electrodes with positions, no `type` in the metadata file.
#[test]
fn reader_electrodes_make_electrical_series_with_positions() {
    use nc_core::{ChannelRef, Electrode, ElectrodeGroup};
    let mut s = Session::default();
    s.metadata.start_time = Some("2025-02-26T15:25:56-05:00".into());
    s.recordings.push(Arc::new(MemoryRecording::new("ap", (0..400).map(|v| v as f32).collect(), 2, 1000.0, "V").unwrap()));
    s.electrode_groups.push(ElectrodeGroup { name: "shank0".into(), description: "probe shank".into(), location: "CA1".into(), device: None });
    for c in 0..2 {
        s.electrodes.push(Electrode {
            name: format!("ap {c}"),
            group: "shank0".into(),
            channels: vec![ChannelRef { recording: "ap".into(), channel: c }],
            position_um: Some([0.0, 20.0 * c as f32, 0.0]),
            ..Default::default()
        });
    }
    let meta = MetadataFile::parse("session: { description: probe test }\n").unwrap();
    let plan = nwb::plan(&mut s, &meta, || "probe-id".into());
    assert!(!plan.has_errors(), "{:?}", plan.issues);
    assert_eq!((plan.series[0].electrodes.clone(), plan.series[0].unit.as_str()), (Some(vec![0, 1]), "volts"));

    let dest = out_dir().join("probe.nwb.zarr");
    nwb::write(&s, &plan, &dest, &NwbOptions { overwrite: true, threads: 1, gzip: None, ..Default::default() }, &|_| {}).unwrap();
    let series: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dest.join("acquisition/ap/zarr.json")).unwrap()).unwrap();
    assert_eq!(series["attributes"]["neurodata_type"], "ElectricalSeries");
    let store = Arc::new(zarrs::filesystem::FilesystemStore::new(&dest).unwrap());
    let rel_y = zarrs::array::Array::open(store, "/general/extracellular_ephys/electrodes/rel_y").unwrap();
    assert_eq!(rel_y.retrieve_array_subset::<Vec<f32>>(&rel_y.subset_all()).unwrap(), vec![0.0, 20.0]);
    assert!(nwb::validate::validate(&dest).unwrap().is_empty());

    // Declaring the stream a plain time series overrides the reader's electrodes
    let as_ts = MetadataFile::parse("session: { description: probe test }\nstreams:\n  ap: { type: timeseries }\n").unwrap();
    assert_eq!(nwb::plan(&mut s, &as_ts, || "id".into()).series[0].electrodes, None);
    // And electrical without electrodes is an error
    let mut bare = Session::default();
    bare.metadata.start_time = s.metadata.start_time.clone();
    bare.recordings.push(Arc::new(MemoryRecording::new("ap", vec![0.0; 4], 2, 1000.0, "V").unwrap()));
    let electrical = MetadataFile::parse("session: { description: d }\nstreams:\n  ap: { type: electrical }\n").unwrap();
    let plan = nwb::plan(&mut bare, &electrical, || "id".into());
    assert!(plan.issues.iter().any(|i| i.message.contains("2 of 2 channels have no electrode")), "{:?}", plan.issues);
}

#[test]
fn chunk_policy_parses_and_sizes_auto_chunks_by_bytes() {
    use nwb::ChunkPolicy;
    assert_eq!("1".parse::<ChunkPolicy>().unwrap(), ChunkPolicy::Seconds(1.0));
    assert_eq!("0.5s".parse::<ChunkPolicy>().unwrap(), ChunkPolicy::Seconds(0.5));
    assert_eq!("AUTO".parse::<ChunkPolicy>().unwrap(), ChunkPolicy::Auto);
    assert!("0".parse::<ChunkPolicy>().is_err() && "fast".parse::<ChunkPolicy>().is_err());
    assert_eq!(ChunkPolicy::Seconds(1.0).rows(24_414.0625, 32, 4), 24_414);
    // 32 float32 channels: 10 MB / 128 B per row
    assert_eq!(ChunkPolicy::Auto.rows(24_414.0625, 32, 4), 78_125);
    assert_eq!(ChunkPolicy::Auto.rows(100.0, 1, 2), 5_000_000);

    // An auto-chunked write: 3 × float32 → the whole 1000-sample series fits one chunk
    let mut s = session();
    let plan = nwb::plan(&mut s, &MetadataFile::parse(META).unwrap(), || "auto-id".into());
    let dest = out_dir().join("auto.nwb.zarr");
    nwb::write(&s, &plan, &dest, &NwbOptions { overwrite: true, threads: 2, chunks: ChunkPolicy::Auto, gzip: None, ..Default::default() }, &|_| {}).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dest.join("acquisition/EMG/data/zarr.json")).unwrap()).unwrap();
    assert_eq!(meta["chunk_grid"]["configuration"]["chunk_shape"], serde_json::json!([1000, 3]));
}

/// Content verification: a fresh store matches its source; swapped or damaged chunks do not, and
/// the saved digests re-check a store without the source.
#[test]
fn integrity_detects_swapped_and_damaged_chunks() {
    use nwb::integrity::{recheck, verify};
    use nwb::VerifyLevel;
    let mut s = session();
    // A native (read_stored) float64 stream next to the float32 ones
    let template = MemoryRecording::new("P", vec![0.0; 2 * 500], 2, 1000.0, "a.u.").unwrap();
    let mut info = nc_core::Recording::info(&template).clone();
    info.name = "Precise".into();
    info.stored_as = nc_core::SampleType::F64;
    s.recordings.push(Arc::new(F64Recording { info, data: (0..1000).map(|i| i as f64 * 0.25).collect() }));
    let plan = nwb::plan(&mut s, &MetadataFile::parse(META).unwrap(), || "integrity-id".into());
    let dest = out_dir().join("integrity.nwb.zarr");
    // 0.3 s chunks: EMG (1000 Hz) has 4 chunks
    nwb::write(&s, &plan, &dest, &NwbOptions { overwrite: true, threads: 2, chunks: nwb::ChunkPolicy::Seconds(0.3), gzip: None, ..Default::default() }, &|_| {}).unwrap();

    let (digests, issues) = verify(&s, &plan, &dest, VerifyLevel::Full, 2, None, &|_| {}).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    let paths: Vec<&str> = digests.arrays.iter().map(|a| a.path.as_str()).collect();
    for p in ["/acquisition/EMG/data", "/acquisition/Temp/data", "/acquisition/Precise/data", "/events/MET/timestamp", "/events/MET/duration", "/acquisition/eS1p/data", "/acquisition/eNe1_ch1/data", "/acquisition/eNe1_ch1/timestamps"] {
        assert!(paths.contains(&p), "{p} verified: {paths:?}");
    }
    let precise = digests.arrays.iter().find(|a| a.path == "/acquisition/Precise/data").unwrap();
    assert_eq!((precise.dtype.as_str(), precise.digest.is_some()), ("float64", true));
    assert!(recheck(&dest, &digests, 2).unwrap().1.is_empty(), "an intact store passes the re-check");
    let (none, _) = verify(&s, &plan, &dest, VerifyLevel::Off, 1, None, &|_| {}).unwrap();
    assert!(none.arrays.is_empty());

    // A writer that put two chunks in each other's place: same bytes, wrong rows
    let chunk = |k: usize| dest.join(format!("acquisition/EMG/data/c/{k}/0"));
    let (a, b) = (std::fs::read(chunk(0)).unwrap(), std::fs::read(chunk(1)).unwrap());
    std::fs::write(chunk(0), &b).unwrap();
    std::fs::write(chunk(1), &a).unwrap();
    for level in [VerifyLevel::Full, VerifyLevel::Sampled] {
        let (d, issues) = verify(&s, &plan, &dest, level, 2, None, &|_| {}).unwrap();
        assert_eq!(d.mismatched(), 1, "{level:?}: {issues:?}");
        assert!(issues.iter().any(|i| i.message.starts_with("/acquisition/EMG/data: content differs from the source")), "{issues:?}");
    }
    assert_eq!(recheck(&dest, &digests, 2).unwrap().0.mismatched(), 1, "the saved digests catch it without the source");
    std::fs::write(chunk(0), &a).unwrap();
    std::fs::write(chunk(1), &b).unwrap();

    // One flipped bit in a snippet waveform
    let snip = dest.join("acquisition/eNe1_ch1/data/c/0/0/0");
    let mut bytes = std::fs::read(&snip).unwrap();
    bytes[5] ^= 0x10;
    std::fs::write(&snip, bytes).unwrap();
    let (_, issues) = verify(&s, &plan, &dest, VerifyLevel::Full, 1, None, &|_| {}).unwrap();
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(issues[0].message.starts_with("/acquisition/eNe1_ch1/data"));
}
