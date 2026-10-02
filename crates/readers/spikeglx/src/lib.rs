//! nc-spikeglx: SpikeGLX recordings (Neuropixels imec probes and the NI-DAQ).
//!
//! Every `.bin` / `.meta` pair of a run becomes recordings of one [`Session`]:
//! - imec: `<probe>.ap` and/or `<probe>.lf` (neural channels, volts, `Electrical`) and
//!   `<probe>.<band>.sync` (the sync word, raw);
//! - nidq: `nidq` (MN / MA / XA analog channels, volts) and `nidq.digital` (DW words, raw).
//!
//! Probes also bring their electrodes: one per probe channel, shared by the AP and LF bands,
//! placed from the geometry map, grouped per probe (and shank). Samples stay int16; the
//! volts-per-bit factor of each channel is its gain (`range / maxInt / channel gain`).
//!
//! Not yet: OneBox (`obx`) streams, TTL events from digital lines, aligning streams with the
//! sync channel (every stream starts at 0 s), split files (`_t0`, `_t1`, … triggers).

pub mod bin;
pub mod files;
pub mod meta;
pub mod probe;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use bin::BinRecording;
use files::MetaFile;
use meta::{Meta, StreamType};
use nc_base::mapped::MappedFile;
use nc_base::time::{format_iso, parse_iso};
use nc_core::{
    Calibration, ChannelInfo, ChannelRef, Detection, Device, Electrode, ElectrodeGroup, Error, MemoryOrder, OpenOptions, Provenance, Reader,
    RecordingInfo, Result, SampleType, Session, SignalKind,
};

pub struct SpikeGlx;

impl Reader for SpikeGlx {
    fn name(&self) -> &'static str {
        "spikeglx"
    }

    fn description(&self) -> &'static str {
        "SpikeGLX run (Neuropixels imec probes, NI-DAQ): .bin + .meta"
    }

    fn opens(&self) -> &'static str {
        "A run folder (with *.ap.bin / *.lf.bin / *.nidq.bin and their .meta files), or one .bin / .meta file"
    }

    fn versions(&self) -> &'static [&'static str] {
        &[
            "imec AP / LF / sync: Neuropixels 3A, 1.0 family, 2.0 (gains and site positions from the metadata)",
            "nidq: MN / MA / XA analog, DW digital words",
        ]
    }

    fn detect(&self, path: &Path) -> Option<Detection> {
        let runs = files::runs(path);
        let first = runs.values().flatten().next()?;
        Meta::load(&first.path).ok()?;
        let confidence = if path.is_file() { 0.95 } else { 0.9 };
        let version = (runs.len() > 1).then(|| format!("{} runs", runs.len()));
        Some(Detection { format: "spikeglx", version, confidence })
    }

    /// The runs reachable from `path` when there are several.
    fn containers(&self, path: &Path) -> Vec<String> {
        let runs = files::runs(path);
        if runs.len() > 1 { runs.into_keys().collect() } else { Vec::new() }
    }

    fn open(&self, path: &Path, options: &OpenOptions) -> Result<Session> {
        let runs = files::runs(path);
        let (run, metas) = match (&options.block, runs.len()) {
            (Some(name), _) => runs.into_iter().find(|(r, _)| r == name),
            (None, 1) => runs.into_iter().next(),
            (None, 0) => return Err(Error::format("spikeglx", format!("no .meta file in {}", path.display()))),
            (None, n) => {
                let names: Vec<&String> = runs.keys().collect();
                return Err(Error::Unsupported(format!(
                    "{} holds {n} SpikeGLX runs; choose one with --block <name>: {}",
                    path.display(),
                    names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                )));
            }
        }
        .ok_or_else(|| Error::Unsupported(format!("run {:?} not found in {}", options.block.as_deref().unwrap_or(""), path.display())))?;
        open_run(&run, metas, options)
    }
}

/// Builds the session of one run from its `.meta` files.
fn open_run(run: &str, metas: Vec<MetaFile>, options: &OpenOptions) -> Result<Session> {
    let mut b = Builder::default();
    let mut loaded = Vec::new();
    for f in metas {
        let meta = Meta::load(&f.path)?;
        let label = match (&f.stream, meta.stream_type()?) {
            (Some(s), _) => s.clone(),
            (None, StreamType::Imec) => "imec0".into(),
            (None, StreamType::Nidq) => "nidq".into(),
            (None, StreamType::Obx) => "obx0".into(),
        };
        loaded.push((f, meta, label));
    }
    // AP before LF, so a probe's electrodes follow AP channel order
    loaded.sort_by(|a, b| (&a.2, &a.0.band).cmp(&(&b.2, &b.0.band)));
    for (f, meta, label) in &loaded {
        if let Err(e) = b.add_stream(f, meta, label, options) {
            b.warnings.push(format!("{}: skipped ({e})", f.path.display()));
        }
    }
    b.finish(run, &loaded)
}

#[derive(Default)]
struct Builder {
    session: Session,
    warnings: Vec<String>,
    /// (probe label, probe channel) → index into `session.electrodes`.
    electrodes: BTreeMap<(String, usize), usize>,
    files: Vec<std::path::PathBuf>,
}

impl Builder {
    fn add_stream(&mut self, f: &MetaFile, meta: &Meta, label: &str, options: &OpenOptions) -> Result<()> {
        let kind = meta.stream_type()?;
        if kind == StreamType::Obx {
            return Err(Error::Unsupported("OneBox (obx) streams are not read yet".into()));
        }
        let bin_path = f.bin();
        let file = Arc::new(MappedFile::open(&bin_path)?);
        self.files.extend([f.path.clone(), bin_path.clone()]);
        let columns = meta.saved_channels()?;
        let samples = file.len() / (2 * columns as u64);
        if file.len() % (2 * columns as u64) != 0 {
            self.warnings.push(format!("{label}: {} ends with a partial sample", bin_path.display()));
        }
        if let Some(expected) = meta.f64("fileSizeBytes").filter(|e| *e as u64 != file.len()) {
            self.warnings.push(format!("{label}: {} has {} bytes, the metadata says {expected} (truncated or still recording?)", bin_path.display(), file.len()));
        }
        let original = meta.original_channels()?;
        let rate = meta.sample_rate()?;
        let vpb = meta.volts_per_bit()?;
        let mut extras = BTreeMap::new();
        if let Some(v) = meta.get("firstSample") {
            extras.insert("spikeglx_first_sample".to_string(), v.to_string());
        }
        let base = |name: String, channels: Vec<ChannelInfo>, kind: SignalKind, unit: &str, calibration: Calibration, description: String| RecordingInfo {
            name,
            description,
            channels,
            samples,
            sample_rate: rate,
            start_time: 0.0,
            unit: unit.into(),
            calibration,
            kind,
            stored_as: SampleType::I16,
            order: MemoryOrder::TimeMajor,
            storage: "bin".into(),
            metadata: extras.clone(),
        };

        match kind {
            StreamType::Imec => {
                let counts = meta.counts("snsApLfSy")?;
                let (ap, lf) = (counts[0], counts.get(1).copied().unwrap_or(0));
                // Original index of the first LF channel = number of acquired AP channels
                let acquired_ap = meta.counts("acqApLfSy").ok().and_then(|c| c.first().copied()).unwrap_or(ap);
                let gains = probe::Gains::from_meta(meta);
                let sites = probe::sites(meta);
                if sites.is_none() {
                    self.warnings.push(format!("{label}: probe type {} has no known site layout; electrodes have no positions", probe::model(meta)));
                }
                self.add_probe_device(label, meta);
                for (band, cols, is_lf) in [("ap", 0..ap, false), ("lf", ap..ap + lf, true)] {
                    if cols.is_empty() {
                        continue;
                    }
                    let name = format!("{label}.{band}");
                    let mut channels = Vec::new();
                    let mut unknown = false;
                    for &orig in &original[cols.clone()] {
                        let chan = if is_lf { orig - acquired_ap } else { orig };
                        let gain = gains.get(chan, is_lf);
                        unknown |= gain.is_none();
                        let prefix = if is_lf { "LF" } else { "AP" };
                        channels.push(ChannelInfo { name: format!("{prefix}{chan}"), gain: gain.map_or(1.0, |g| vpb / g), offset: 0.0 });
                    }
                    let (unit, calibration) = if unknown {
                        ("a.u.", Calibration::Unknown { note: format!("probe {}: channel gains unknown", probe::model(meta)) })
                    } else {
                        ("V", Calibration::Known)
                    };
                    let description = format!("Neuropixels {} band, probe {} ({label})", band.to_uppercase(), probe::model(meta));
                    let info = base(name.clone(), channels, SignalKind::Electrical, unit, calibration, description);
                    if options.wants(&name) {
                        // Electrodes: one per probe channel, shared between bands
                        for (k, &orig) in original[cols.clone()].iter().enumerate() {
                            let chan = if is_lf { orig - acquired_ap } else { orig };
                            let site = sites.as_ref().and_then(|s| s.get(cols.start + k));
                            self.link_electrode(label, chan, site, ChannelRef { recording: name.clone(), channel: k });
                        }
                        self.session.recordings.push(Arc::new(BinRecording::new(info, file.clone(), columns, cols.collect())));
                    }
                }
                let sync: Vec<usize> = (ap + lf..columns).collect();
                let band = f.band.as_deref().unwrap_or(if ap > 0 { "ap" } else { "lf" });
                let name = format!("{label}.{band}.sync");
                if !sync.is_empty() && options.wants(&name) {
                    let channels = sync.iter().enumerate().map(|(i, _)| ChannelInfo::unity(format!("SY{i}"))).collect();
                    let info = base(name, channels, SignalKind::Other, "a.u.", Calibration::Known, format!("Sync word of probe {label} (16 digital bits)"));
                    self.session.recordings.push(Arc::new(BinRecording::new(info, file, columns, sync)));
                }
            }
            StreamType::Nidq => {
                let counts = meta.counts("snsMnMaXaDw")?;
                let (mn, ma, xa) = (counts[0], counts[1], counts[2]);
                let gain = |key: &str| meta.f64(key).filter(|g| *g > 0.0).unwrap_or(1.0);
                let (mn_gain, ma_gain) = (gain("niMNGain"), gain("niMAGain"));
                let analog: Vec<ChannelInfo> = (0..mn + ma + xa)
                    .map(|j| {
                        let (prefix, i, g) = if j < mn { ("MN", j, mn_gain) } else if j < mn + ma { ("MA", j - mn, ma_gain) } else { ("XA", j - mn - ma, 1.0) };
                        ChannelInfo { name: format!("{prefix}{i}"), gain: vpb / g, offset: 0.0 }
                    })
                    .collect();
                let device = meta.get("niDev1ProductName").map(str::to_string);
                self.session.metadata.devices.push(Device {
                    name: label.into(),
                    description: "National Instruments DAQ recorded by SpikeGLX".into(),
                    manufacturer: Some("National Instruments".into()),
                    model: device,
                });
                if !analog.is_empty() && options.wants(label) {
                    let info = base(label.into(), analog, SignalKind::Other, "V", Calibration::Known, "NI-DAQ analog channels (MN, MA, XA)".into());
                    self.session.recordings.push(Arc::new(BinRecording::new(info, file.clone(), columns, (0..mn + ma + xa).collect())));
                }
                let digital: Vec<usize> = (mn + ma + xa..columns).collect();
                let name = format!("{label}.digital");
                if !digital.is_empty() && options.wants(&name) {
                    let channels = digital.iter().enumerate().map(|(i, _)| ChannelInfo::unity(format!("DW{i}"))).collect();
                    let info = base(name, channels, SignalKind::Other, "a.u.", Calibration::Known, "NI-DAQ digital words (16 lines each)".into());
                    self.session.recordings.push(Arc::new(BinRecording::new(info, file, columns, digital)));
                }
            }
            StreamType::Obx => unreachable!(),
        }
        Ok(())
    }

    fn add_probe_device(&mut self, label: &str, meta: &Meta) {
        if self.session.metadata.devices.iter().any(|d| d.name == label) {
            return;
        }
        let model = probe::model(meta);
        let serial = probe::serial(meta).map_or_else(String::new, |s| format!(", serial {s}"));
        self.session.metadata.devices.push(Device {
            name: label.into(),
            description: format!("Neuropixels probe {model}{serial}"),
            manufacturer: Some("IMEC".into()),
            model: Some(model),
        });
    }

    /// Adds `channel` to the electrode of probe channel `chan`, creating it (and its group) once.
    fn link_electrode(&mut self, label: &str, chan: usize, site: Option<&probe::Site>, channel: ChannelRef) {
        if let Some(&i) = self.electrodes.get(&(label.to_string(), chan)) {
            self.session.electrodes[i].channels.push(channel);
            return;
        }
        let shank = site.map_or(0, |s| s.shank);
        let shanked = site.is_some_and(|s| s.shank > 0) || self.session.electrode_groups.iter().any(|g| g.name.starts_with(&format!("{label}_shank")));
        let group = if shanked { format!("{label}_shank{shank}") } else { label.to_string() };
        if self.session.electrode_group(&group).is_none() {
            let description = if shanked { format!("Neuropixels probe {label}, shank {shank}") } else { format!("Neuropixels probe {label}") };
            self.session.electrode_groups.push(ElectrodeGroup { name: group.clone(), description, location: "unknown".into(), device: Some(label.into()) });
        }
        self.session.electrodes.push(Electrode {
            name: format!("{label} ch{chan}"),
            group,
            channels: vec![channel],
            position_um: site.map(|s| [s.x, s.z, 0.0]),
            ..Default::default()
        });
        self.electrodes.insert((label.to_string(), chan), self.session.electrodes.len() - 1);
    }

    fn finish(mut self, run: &str, loaded: &[(MetaFile, Meta, String)]) -> Result<Session> {
        let s = &mut self.session;
        s.recordings.sort_by(|a, b| a.info().name.cmp(&b.info().name));
        if loaded.len() > 1 {
            self.warnings.push("streams are not aligned with the sync channel yet: every stream starts at 0 s".into());
        }
        let m = &mut s.metadata;
        m.experiment = Some(run.to_string());
        // Earliest file creation time (local time, no zone in the metadata)
        let start = loaded.iter().filter_map(|(_, meta, _)| meta.get("fileCreateTime").and_then(parse_iso)).reduce(f64::min);
        m.start_time = start.map(format_iso);
        let longest = loaded.iter().filter_map(|(_, meta, _)| meta.f64("fileTimeSecs")).reduce(f64::max);
        if let (Some(t0), Some(d)) = (start, longest) {
            m.stop_time = Some(format_iso(t0 + d));
        }
        for (f, meta, label) in loaded {
            let key = format!("spikeglx_{label}{}", f.band.as_ref().map_or_else(String::new, |b| format!(".{b}")));
            if let Some(v) = meta.get("firstSample") {
                m.extra.insert(format!("{key}_first_sample"), v.to_string());
            }
            if let Some(n) = meta.get("userNotes").filter(|n| !n.is_empty()) {
                m.notes.push(format!("{label}: {n}"));
            }
        }
        let app = loaded.iter().find_map(|(_, meta, _)| meta.get("appVersion")).unwrap_or("?");
        m.extra.insert("spikeglx_app_version".into(), app.to_string());
        m.extra.insert("spikeglx_run".into(), run.to_string());

        let mut prov = Provenance::new("spikeglx");
        prov.version = Some(format!("SpikeGLX {app}"));
        for f in &self.files {
            prov.add_file(f);
        }
        // SpikeGLX records the SHA-1 of every finished .bin
        for (f, meta, _) in loaded {
            if let Some(sha1) = meta.get("fileSHA1").filter(|v| !v.is_empty()) {
                prov.set_checksum(&f.bin(), "sha1", sha1);
            }
        }
        prov.warnings = self.warnings;
        s.provenance = prov;
        Ok(self.session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::tests::NP1_3A;

    /// `<workspace>/target/<name>`.
    fn fixture_dir(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target").join(name)
    }

    /// A 3A-style AP file (4 AP + 1 sync, 10 samples) with an LF sibling, plus a NI-DAQ file.
    fn write_run(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let write = |stem: &str, meta: &str, columns: usize, samples: usize| {
            std::fs::write(dir.join(format!("{stem}.meta")), meta).unwrap();
            // value = column * 100 + sample
            let bin: Vec<u8> = (0..samples).flat_map(|t| (0..columns).flat_map(move |c| ((c * 100 + t) as i16).to_le_bytes())).collect();
            std::fs::write(dir.join(format!("{stem}.bin")), bin).unwrap();
        };
        let ap = NP1_3A.replace("fileSizeBytes=50", "fileSizeBytes=100");
        write("run_g0_t0.imec.ap", &ap, 5, 10);
        let lf = NP1_3A
            .replace("snsApLfSy=4,0,1", "snsApLfSy=0,4,1")
            .replace("snsSaveChanSubset=0:3,8", "snsSaveChanSubset=4:7,8")
            .replace("imSampRate=30000", "imSampRate=2500")
            .replace("fileSizeBytes=50", "fileSizeBytes=20");
        write("run_g0_t0.imec.lf", &lf, 5, 2);
        let ni = "typeThis=nidq\nnSavedChans=3\nniSampRate=1000\nniAiRangeMax=5\nniMaxInt=32768\nniMNGain=200\nniMAGain=1\n\
snsMnMaXaDw=1,0,1,1\nsnsSaveChanSubset=all\nfileCreateTime=2019-05-07T17:24:01\nfileTimeSecs=0.004\nfileSizeBytes=24\n";
        write("run_g0_t0.nidq", ni, 3, 4);
    }

    #[test]
    fn test_run_with_probe_and_nidq() {
        let dir = fixture_dir("spikeglx-fixture");
        write_run(&dir);
        let s = nc_core::testkit::check_reader(&SpikeGlx, &dir, &OpenOptions::default());
        let names: Vec<String> = s.recordings.iter().map(|r| r.info().name.clone()).collect();
        assert_eq!(names, vec!["imec0.ap", "imec0.ap.sync", "imec0.lf", "imec0.lf.sync", "nidq", "nidq.digital"]);

        let ap = s.recording("imec0.ap").unwrap();
        assert_eq!((ap.info().channel_count(), ap.info().samples, ap.info().sample_rate), (4, 10, 30000.0));
        let mut out = vec![0.0; 2];
        ap.read(&[2], 3..5, &mut out).unwrap();
        // column 2 = AP2 with gain 250: value × 0.6 / 512 / 250
        let v = 0.6 / 512.0 / 250.0;
        assert_eq!(out, vec![(203.0 * v) as f32, (204.0 * v) as f32]);
        let mut raw = vec![0u8; 2];
        s.recording("imec0.ap.sync").unwrap().read_stored(&[0], 1..2, &mut raw).unwrap();
        assert_eq!(i16::from_le_bytes([raw[0], raw[1]]), 401, "sync is column 4");

        // AP and LF share the probe's four electrodes
        assert_eq!(s.electrodes.len(), 4);
        assert_eq!(s.channel_electrodes("imec0.ap"), s.channel_electrodes("imec0.lf"));
        assert_eq!(s.electrodes[1].channels.len(), 2);
        assert_eq!(s.electrodes[1].position_um, Some([59.0, 0.0, 0.0]));
        assert_eq!(s.electrode_groups[0].device.as_deref(), Some("imec0"));

        let ni = s.recording("nidq").unwrap();
        assert_eq!(ni.info().channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["MN0", "XA0"]);
        assert_eq!(ni.info().channels[0].gain, 5.0 / 32768.0 / 200.0);
        assert_eq!(s.metadata.start_time.as_deref(), Some("2019-05-07T17:24:01"));
        assert!(s.provenance.warnings.iter().any(|w| w.contains("not aligned")));
    }

    #[test]
    fn test_file_path_and_run_choice() {
        let dir = fixture_dir("spikeglx-runs");
        write_run(&dir);
        // A second run in the same folder
        for ext in ["meta", "bin"] {
            std::fs::copy(dir.join(format!("run_g0_t0.nidq.{ext}")), dir.join(format!("run_g1_t0.nidq.{ext}"))).unwrap();
        }
        assert_eq!(SpikeGlx.containers(&dir), vec!["run_g0_t0", "run_g1_t0"]);
        assert!(SpikeGlx.containers(&dir.join("run_g0_t0.nidq.bin")).is_empty(), "a file selects one run");
        let err = SpikeGlx.open(&dir, &OpenOptions::default()).err().unwrap().to_string();
        assert!(err.contains("--block") && err.contains("run_g0_t0, run_g1_t0"), "{err}");
        let one = SpikeGlx.open(&dir, &OpenOptions { block: Some("run_g1_t0".into()), ..Default::default() }).unwrap();
        assert_eq!(one.recordings.len(), 2);
        // A file selects its own run
        let s = SpikeGlx.open(&dir.join("run_g0_t0.imec.lf.bin"), &OpenOptions::default()).unwrap();
        assert_eq!(s.recordings.len(), 6);
        let only = SpikeGlx.open(&dir.join("run_g0_t0.imec.ap.meta"), &OpenOptions { only: Some(vec!["imec0.ap".into()]), ..Default::default() }).unwrap();
        assert_eq!(only.recordings.len(), 1);
        assert!(only.validate().is_empty(), "{:?}", only.validate());
    }
}
