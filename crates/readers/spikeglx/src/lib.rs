//! nc-spikeglx: SpikeGLX recordings (Neuropixels imec probes, the NI-DAQ and OneBox).
//!
//! Every `.bin` / `.meta` pair of a run (one gate) becomes recordings of one [`Session`]:
//! - imec: `<probe>.ap` and/or `<probe>.lf` (neural channels, volts, `Electrical`) and
//!   `<probe>.<band>.sync` (the sync word, raw);
//! - nidq: `nidq` (MN / MA / XA analog channels, volts) and `nidq.digital` (DW words, raw);
//! - OneBox: `obx0` (XA analog, volts), `obx0.digital` (XD word) and `obx0.sync` (SY word).
//!
//! Probes also bring their electrodes: one per probe channel, shared by the AP and LF bands,
//! placed from the geometry map, grouped per probe (and shank). Samples stay int16; the
//! volts-per-bit factor of each channel is its gain (`range / maxInt / channel gain`).
//!
//! Triggers (`_t0`, `_t1`, …) of a gate open together: with several, every recording is named
//! with its trigger (`imec0.ap.t1`). Every recording starts at its `firstSample` / rate (time
//! since acquisition start) relative to the run's earliest stream. Digital lines that change (NI DW, OneBox XD) become TTL events
//! `<stream> TTL <line>` (high periods). When streams carry the sync pulse, each is fitted to
//! the reference stream's clock (the first probe, else NI, else OneBox): its start time and
//! sample rate are corrected and events follow ([`sync`]).

/// This crate's version (`nc-spikeglx`, from its `Cargo.toml`): recorded in every conversion's
/// provenance and report, so a problem in a file can be traced to the code that wrote it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// [`VERSION`].
pub fn version() -> &'static str {
    VERSION
}

pub mod bin;
pub mod files;
pub mod meta;
pub mod probe;
pub mod sync;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use bin::BinRecording;
use files::MetaFile;
use meta::{Meta, StreamType};
use nc_base::mapped::MappedFile;
use nc_base::time::{format_iso, parse_iso};
use nc_core::{
    Calibration, ChannelInfo, ChannelRef, Detection, Device, Electrode, ElectrodeGroup, Error, EventSeries, MemoryOrder, OpenOptions, Provenance,
    Reader, RecordingInfo, Result, SampleType, Session, SignalKind,
};
use sync::{Columns, Level};

pub struct SpikeGlx;

impl Reader for SpikeGlx {
    fn name(&self) -> &'static str {
        "spikeglx"
    }
    fn version(&self) -> &'static str {
        crate::VERSION
    }

    /// Compared with its reference reader on real data (docs/formats).
    fn maturity(&self) -> nc_core::Maturity {
        nc_core::Maturity::Verified
    }

    fn description(&self) -> &'static str {
        "SpikeGLX run (Neuropixels imec probes, NI-DAQ, OneBox): .bin + .meta"
    }

    fn opens(&self) -> &'static str {
        "A run folder (with *.ap.bin / *.lf.bin / *.nidq.bin / *.obx.bin and their .meta files), or one .bin / .meta file"
    }

    fn versions(&self) -> &'static [&'static str] {
        &[
            "imec AP / LF / sync: Neuropixels 3A, 1.0 family, 2.0 (gains and site positions from the metadata)",
            "nidq: MN / MA / XA analog, DW digital words (TTL events)",
            "OneBox: XA analog, XD digital word (TTL events), SY sync",
            "multi-trigger gates (_t0, _t1, …), CatGT output (_tcat), sync-pulse alignment",
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

/// One loaded `.meta` / `.bin` pair.
struct Loaded {
    file: MetaFile,
    meta: Meta,
    /// `imec0`, `nidq`, `obx0`, …
    label: String,
}

/// Builds the session of one run (gate) from its `.meta` files.
fn open_run(run: &str, metas: Vec<MetaFile>, options: &OpenOptions) -> Result<Session> {
    let mut loaded = Vec::new();
    for f in metas {
        let meta = Meta::load(&f.path)?;
        let label = match (&f.stream, meta.stream_type()?) {
            (Some(s), _) => s.clone(),
            (None, StreamType::Imec) => "imec0".into(),
            (None, StreamType::Nidq) => "nidq".into(),
            (None, StreamType::Obx) => "obx0".into(),
        };
        loaded.push(Loaded { file: f, meta, label });
    }
    // Triggers in order; within one, streams by label with AP before LF (a probe's electrodes
    // follow AP channel order)
    let trig_key = |t: &Option<String>| t.as_deref().map_or((0, 0), |t| t.parse::<u64>().map_or((1, 0), |n| (0, n)));
    loaded.sort_by(|a, b| (trig_key(&a.file.trigger), &a.label, &a.file.band).cmp(&(trig_key(&b.file.trigger), &b.label, &b.file.band)));
    let mut triggers: Vec<Option<String>> = loaded.iter().map(|l| l.file.trigger.clone()).collect();
    triggers.dedup();
    let mut b = Builder { multi_trigger: triggers.len() > 1, ..Default::default() };
    for l in &loaded {
        if let Err(e) = b.add_file(l, &loaded, options) {
            b.warnings.push(format!("{}: skipped ({e})", l.file.path.display()));
        }
    }
    b.finish(run, &loaded)
}

/// Whether another file of probe `label` has a site layout (then a file without one is fine).
fn loaded_sites_later(all: &[Loaded], label: &str) -> bool {
    all.iter().any(|o| o.label == label && probe::sites(&o.meta).is_some())
}

/// A file's clock: where its samples are in time, and its sync edges (samples).
struct Clock {
    /// `<label>[.<band>]`, for messages.
    name: String,
    trigger: Option<String>,
    kind: StreamType,
    rate: f64,
    /// Seconds from the run's start (trigger offset).
    start: f64,
    edges: Vec<u64>,
    /// Sync period (s).
    period: f64,
    /// Filled by the fit: reference time = scale × own time + offset.
    fit: Option<sync::Fit>,
}

impl Clock {
    /// Seconds of sample `t` on the reference clock (the own clock without a fit).
    fn time(&self, t: u64) -> f64 {
        let own = self.start + t as f64 / self.rate;
        self.fit.map_or(own, |f| f.scale * own + f.offset)
    }
}

/// A recording waiting for its clock.
struct Pending {
    info: RecordingInfo,
    file: Arc<MappedFile>,
    columns: usize,
    selected: Vec<usize>,
    clock: usize,
}

/// TTL lines of one file: line → (rising, falling) samples.
struct Ttl {
    stream: String,
    clock: usize,
    samples: u64,
    lines: BTreeMap<u32, (Vec<u64>, Vec<u64>)>,
}

#[derive(Default)]
struct Builder {
    session: Session,
    warnings: Vec<String>,
    /// (probe label, probe channel) → index into `session.electrodes`.
    electrodes: BTreeMap<(String, usize), usize>,
    /// Site layout per probe (an LF `.meta` may lack the maps its AP sibling has).
    sites: BTreeMap<String, Vec<probe::Site>>,
    files: Vec<std::path::PathBuf>,
    multi_trigger: bool,
    clocks: Vec<Clock>,
    pending: Vec<Pending>,
    ttl: Vec<Ttl>,
}

impl Builder {
    /// `name` with the trigger when the gate has several.
    fn named(&self, name: String, trigger: &Option<String>) -> String {
        match (self.multi_trigger, trigger) {
            (true, Some(t)) => format!("{name}.t{t}"),
            _ => name,
        }
    }

    fn add_file(&mut self, l: &Loaded, all: &[Loaded], options: &OpenOptions) -> Result<()> {
        let (f, meta, label) = (&l.file, &l.meta, l.label.as_str());
        let kind = meta.stream_type()?;
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

        // Start: seconds since acquisition start (`firstSample` / rate, each stream on its own
        // clock) relative to the run's earliest stream, so triggers and streams keep their offsets
        let since = |m: &Meta| Some(m.f64("firstSample")? / m.sample_rate().ok()?);
        let t0 = all.iter().filter_map(|o| since(&o.meta)).reduce(f64::min);
        let start = match (since(meta), t0) {
            (Some(t), Some(t0)) => t - t0,
            _ => 0.0,
        };
        let clock = self.clocks.len();
        let band_name = f.band.as_ref().map_or_else(|| label.to_string(), |b| format!("{label}.{b}"));
        self.clocks.push(Clock {
            name: self.named(band_name, &f.trigger),
            trigger: f.trigger.clone(),
            kind,
            rate,
            start,
            edges: Vec::new(),
            period: meta.f64("syncSourcePeriod").filter(|p| *p > 0.0).unwrap_or(1.0),
            fit: None,
        });

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
            start_time: start,
            unit: unit.into(),
            calibration,
            kind,
            stored_as: SampleType::I16,
            order: MemoryOrder::TimeMajor,
            storage: "bin".into(),
            metadata: extras.clone(),
        };
        let push = |b: &mut Self, info: RecordingInfo, selected: Vec<usize>| b.pending.push(Pending { info, file: file.clone(), columns, selected, clock });
        let cols = Columns::new(&file, columns);
        // Sync edges, sampled every 10 ms (or a quarter period)
        let step = |b: &Self| ((rate * 0.01).min(rate * b.clocks[clock].period / 4.0)).max(1.0) as u64;

        match kind {
            StreamType::Imec => {
                let counts = meta.counts("snsApLfSy")?;
                let (ap, lf) = (counts[0], counts.get(1).copied().unwrap_or(0));
                // Original index of the first LF channel = number of acquired AP channels
                let acquired_ap = meta.counts("acqApLfSy").ok().and_then(|c| c.first().copied()).unwrap_or(ap);
                let gains = probe::Gains::from_meta(meta);
                if let Some(found) = probe::sites(meta) {
                    self.sites.entry(label.to_string()).or_insert(found);
                }
                let sites = self.sites.get(label).cloned();
                // Shanks: from the whole layout, so shank 0 is named like the others
                let multi_shank = sites.as_ref().is_some_and(|s| s.iter().any(|x| x.shank > 0));
                if sites.is_none() && !loaded_sites_later(all, label) {
                    self.warnings.push(format!("{label}: probe type {} has no known site layout; electrodes have no positions", probe::model(meta)));
                }
                self.add_probe_device(label, meta);
                for (band, cols, is_lf) in [("ap", 0..ap, false), ("lf", ap..ap + lf, true)] {
                    if cols.is_empty() {
                        continue;
                    }
                    let name = self.named(format!("{label}.{band}"), &f.trigger);
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
                        // Electrodes: one per probe channel, shared between bands and triggers
                        for (k, &orig) in original[cols.clone()].iter().enumerate() {
                            let chan = if is_lf { orig - acquired_ap } else { orig };
                            let site = sites.as_ref().and_then(|s| s.get(cols.start + k));
                            self.link_electrode(label, chan, site, multi_shank, ChannelRef { recording: name.clone(), channel: k });
                        }
                        push(self, info, cols.collect());
                    }
                }
                let sync: Vec<usize> = (ap + lf..columns).collect();
                if let Some(&column) = sync.first() {
                    let step = step(self);
                    self.clocks[clock].edges = sync::square_edges(&cols, Level::Bit { column, bit: sync::SYNC_BIT }, step);
                }
                let band = f.band.as_deref().unwrap_or(if ap > 0 { "ap" } else { "lf" });
                let name = self.named(format!("{label}.{band}.sync"), &f.trigger);
                if !sync.is_empty() && options.wants(&name) {
                    let channels = sync.iter().enumerate().map(|(i, _)| ChannelInfo::unity(format!("SY{i}"))).collect();
                    let info = base(name, channels, SignalKind::Other, "a.u.", Calibration::Known, format!("Sync word of probe {label} (16 digital bits)"));
                    push(self, info, sync);
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
                if !self.session.metadata.devices.iter().any(|d| d.name == label) {
                    self.session.metadata.devices.push(Device {
                        name: label.into(),
                        description: "National Instruments DAQ recorded by SpikeGLX".into(),
                        manufacturer: Some("National Instruments".into()),
                        model: meta.get("niDev1ProductName").map(str::to_string),
                    });
                }
                // Sync: a digital line, or an analog channel above a threshold
                let sync_level = match (meta.f64("syncNiChanType").map(|v| v as u32), meta.f64("syncNiChan").map(|v| v as usize)) {
                    (Some(0), Some(line)) if mn + ma + xa + line / 16 < columns => Some(Level::Bit { column: mn + ma + xa + line / 16, bit: (line % 16) as u32 }),
                    (Some(1), Some(chan)) => original.iter().position(|&o| o == chan).filter(|&c| c < mn + ma + xa).map(|c| {
                        let volts = meta.f64("syncNiThresh").unwrap_or(1.1);
                        Level::Above { column: c, threshold: volts / analog[c].gain }
                    }),
                    _ => None,
                };
                let analog_name = self.named(label.to_string(), &f.trigger);
                if !analog.is_empty() && options.wants(&analog_name) {
                    let info = base(analog_name, analog, SignalKind::Other, "V", Calibration::Known, "NI-DAQ analog channels (MN, MA, XA)".into());
                    push(self, info, (0..mn + ma + xa).collect());
                }
                let digital: Vec<usize> = (mn + ma + xa..columns).collect();
                self.add_digital(label, &cols, &digital, clock, samples, options);
                if let Some(level) = sync_level {
                    let step = step(self);
                    self.clocks[clock].edges = sync::square_edges(&cols, level, step);
                }
                let name = self.named(format!("{label}.digital"), &f.trigger);
                if !digital.is_empty() && options.wants(&name) {
                    let channels = digital.iter().enumerate().map(|(i, _)| ChannelInfo::unity(format!("DW{i}"))).collect();
                    let info = base(name, channels, SignalKind::Other, "a.u.", Calibration::Known, "NI-DAQ digital words (16 lines each)".into());
                    push(self, info, digital);
                }
            }
            StreamType::Obx => {
                let counts = meta.counts("snsXaDwSy")?;
                let (xa, dw) = (counts[0], counts.get(1).copied().unwrap_or(0));
                if !self.session.metadata.devices.iter().any(|d| d.name == label) {
                    let serial = meta.get("imDatBsc_sn").map_or_else(String::new, |s| format!(", serial {s}"));
                    self.session.metadata.devices.push(Device {
                        name: label.into(),
                        description: format!("OneBox recorded by SpikeGLX{serial}"),
                        manufacturer: Some("IMEC".into()),
                        model: meta.get("imDatBsc_pn").map(str::to_string),
                    });
                }
                let analog_name = self.named(label.to_string(), &f.trigger);
                if xa > 0 && options.wants(&analog_name) {
                    let channels = (0..xa).map(|i| ChannelInfo { name: format!("XA{}", original[i]), gain: vpb, offset: 0.0 }).collect();
                    let info = base(analog_name, channels, SignalKind::Other, "V", Calibration::Known, "OneBox analog inputs (XA)".into());
                    push(self, info, (0..xa).collect());
                }
                let digital: Vec<usize> = (xa..xa + dw).collect();
                self.add_digital(label, &cols, &digital, clock, samples, options);
                let name = self.named(format!("{label}.digital"), &f.trigger);
                if !digital.is_empty() && options.wants(&name) {
                    let channels = digital.iter().enumerate().map(|(i, _)| ChannelInfo::unity(format!("XD{i}"))).collect();
                    let info = base(name, channels, SignalKind::Other, "a.u.", Calibration::Known, "OneBox digital word (16 lines)".into());
                    push(self, info, digital);
                }
                let sync: Vec<usize> = (xa + dw..columns).collect();
                if let Some(&column) = sync.first() {
                    let step = step(self);
                    self.clocks[clock].edges = sync::square_edges(&cols, Level::Bit { column, bit: sync::SYNC_BIT }, step);
                }
                let name = self.named(format!("{label}.sync"), &f.trigger);
                if !sync.is_empty() && options.wants(&name) {
                    let channels = sync.iter().enumerate().map(|(i, _)| ChannelInfo::unity(format!("SY{i}"))).collect();
                    let info = base(name, channels, SignalKind::Other, "a.u.", Calibration::Known, format!("Sync word of OneBox {label} (16 digital bits)"));
                    push(self, info, sync);
                }
            }
        }
        Ok(())
    }

    /// TTL lines of the digital words in `digital` (line = word × 16 + bit).
    fn add_digital(&mut self, label: &str, cols: &Columns, digital: &[usize], clock: usize, samples: u64, options: &OpenOptions) {
        let mut lines = BTreeMap::new();
        for (w, &column) in digital.iter().enumerate() {
            for (bit, edges) in sync::transitions(cols, column) {
                let line = w as u32 * 16 + bit;
                if options.wants(&format!("{label} TTL {line}")) {
                    lines.insert(line, edges);
                }
            }
        }
        if !lines.is_empty() {
            self.ttl.push(Ttl { stream: label.to_string(), clock, samples, lines });
        }
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
    fn link_electrode(&mut self, label: &str, chan: usize, site: Option<&probe::Site>, shanked: bool, channel: ChannelRef) {
        if let Some(&i) = self.electrodes.get(&(label.to_string(), chan)) {
            self.session.electrodes[i].channels.push(channel);
            return;
        }
        let shank = site.map_or(0, |s| s.shank);
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

    /// Fits every clock with sync edges to its trigger's reference clock (the first probe, else
    /// NI, else OneBox, that has edges). Returns notes for the metadata and unaligned names.
    fn align(&mut self) -> (Vec<String>, Vec<String>) {
        let (mut notes, mut unaligned) = (Vec::new(), Vec::new());
        let rank = |k: StreamType| match k {
            StreamType::Imec => 0,
            StreamType::Nidq => 1,
            StreamType::Obx => 2,
        };
        let mut triggers: Vec<Option<String>> = self.clocks.iter().map(|c| c.trigger.clone()).collect();
        triggers.dedup();
        for trig in triggers {
            let members: Vec<usize> = (0..self.clocks.len()).filter(|&i| self.clocks[i].trigger == trig).collect();
            if members.len() < 2 {
                continue;
            }
            let reference = members.iter().copied().filter(|&i| self.clocks[i].edges.len() >= 2).min_by_key(|&i| (rank(self.clocks[i].kind), i));
            let Some(r) = reference else {
                unaligned.extend(members.iter().map(|&i| self.clocks[i].name.clone()));
                continue;
            };
            let ref_times: Vec<f64> = self.clocks[r].edges.iter().map(|&t| self.clocks[r].time(t)).collect();
            for &i in members.iter().filter(|&&i| i != r) {
                let c = &self.clocks[i];
                let times: Vec<f64> = c.edges.iter().map(|&t| c.time(t)).collect();
                match sync::fit(&times, &ref_times, c.period / 4.0) {
                    Some(fit) => {
                        notes.push(format!(
                            "{} aligned to {} with {} sync edges: offset {:.3} ms, drift {:.2} ppm, largest residual {:.3} ms",
                            c.name,
                            self.clocks[r].name,
                            fit.edges,
                            (fit.scale * c.start + fit.offset - c.start) * 1e3,
                            (fit.scale - 1.0) * 1e6,
                            fit.residual * 1e3
                        ));
                        self.clocks[i].fit = Some(fit);
                    }
                    None => unaligned.push(c.name.clone()),
                }
            }
        }
        (notes, unaligned)
    }

    fn finish(mut self, run: &str, loaded: &[Loaded]) -> Result<Session> {
        let (align_notes, unaligned) = self.align();
        // Recordings on the reference clock: start and rate from the fit
        for p in std::mem::take(&mut self.pending) {
            let mut info = p.info;
            let c = &self.clocks[p.clock];
            if let Some(fit) = c.fit {
                info.start_time = fit.scale * c.start + fit.offset;
                info.sample_rate = c.rate / fit.scale;
            }
            self.session.recordings.push(Arc::new(BinRecording::new(info, p.file, p.columns, p.selected)));
        }
        // TTL events, joined across triggers
        let mut events: BTreeMap<(String, u32), Vec<(f64, f64)>> = BTreeMap::new();
        for t in &self.ttl {
            let c = &self.clocks[t.clock];
            for (line, (rise, fall)) in &t.lines {
                let e = events.entry((t.stream.clone(), *line)).or_default();
                e.extend(sync::periods(rise, fall, t.samples).into_iter().map(|(a, b)| (c.time(a), c.time(b))));
            }
        }
        for ((stream, line), mut periods) in events {
            periods.sort_by(|a, b| a.0.total_cmp(&b.0));
            let n = periods.len();
            self.session.events.push(EventSeries {
                name: format!("{stream} TTL {line}"),
                description: format!("TTL line {line} of {stream} (high periods)"),
                onsets: periods.iter().map(|p| p.0).collect(),
                offsets: Some(periods.iter().map(|p| p.1).collect()),
                values: vec![1.0; n],
                channels: 1,
                labels: Vec::new(),
            });
        }
        let s = &mut self.session;
        s.recordings.sort_by(|a, b| a.info().name.cmp(&b.info().name));
        if !unaligned.is_empty() && self.clocks.len() > 1 {
            self.warnings.push(format!("no sync pulse to align {} (they keep their own clock, starting at the trigger's first sample)", unaligned.join(", ")));
        }
        let m = &mut s.metadata;
        m.experiment = Some(run.to_string());
        m.notes.extend(align_notes);
        // Earliest file creation time (local time, no zone in the metadata)
        let start = loaded.iter().filter_map(|l| l.meta.get("fileCreateTime").and_then(parse_iso)).reduce(f64::min);
        m.start_time = start.map(format_iso);
        let longest = self.clocks.iter().zip(loaded).filter_map(|(c, l)| Some(c.start + l.meta.f64("fileTimeSecs")?)).reduce(f64::max);
        if let (Some(t0), Some(d)) = (start, longest) {
            m.stop_time = Some(format_iso(t0 + d));
        }
        for l in loaded {
            let band = l.file.band.as_ref().map_or_else(String::new, |b| format!(".{b}"));
            let trig = if self.multi_trigger { l.file.trigger.as_ref().map_or_else(String::new, |t| format!(".t{t}")) } else { String::new() };
            let key = format!("spikeglx_{}{band}{trig}", l.label);
            if let Some(v) = l.meta.get("firstSample") {
                m.extra.insert(format!("{key}_first_sample"), v.to_string());
            }
            // Notes repeat across bands and triggers: once per stream
            if let Some(n) = l.meta.get("userNotes").filter(|n| !n.is_empty())
                && !m.notes.iter().any(|x| x.ends_with(n))
            {
                m.notes.push(format!("{}: {n}", l.label));
            }
        }
        let app = loaded.iter().find_map(|l| l.meta.get("appVersion")).unwrap_or("?");
        m.extra.insert("spikeglx_app_version".into(), app.to_string());
        m.extra.insert("spikeglx_run".into(), run.to_string());
        if self.multi_trigger {
            let mut t: Vec<String> = loaded.iter().filter_map(|l| l.file.trigger.clone()).collect();
            t.dedup();
            m.extra.insert("spikeglx_triggers".into(), t.join(","));
        }

        let mut prov = Provenance::new("spikeglx");
        prov.version = Some(format!("SpikeGLX {app}"));
        for f in &self.files {
            prov.add_file(f);
        }
        // SpikeGLX records the SHA-1 of every finished .bin
        for l in loaded {
            if let Some(sha1) = l.meta.get("fileSHA1").filter(|v| !v.is_empty()) {
                prov.set_checksum(&l.file.bin(), "sha1", sha1);
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
        assert!(s.provenance.warnings.iter().any(|w| w.contains("no sync pulse")), "{:?}", s.provenance.warnings);
    }

    #[test]
    fn test_file_path_and_run_choice() {
        let dir = fixture_dir("spikeglx-runs");
        write_run(&dir);
        // A second run in the same folder
        for ext in ["meta", "bin"] {
            std::fs::copy(dir.join(format!("run_g0_t0.nidq.{ext}")), dir.join(format!("run_g1_t0.nidq.{ext}"))).unwrap();
        }
        assert_eq!(SpikeGlx.containers(&dir), vec!["run_g0", "run_g1"]);
        assert!(SpikeGlx.containers(&dir.join("run_g0_t0.nidq.bin")).is_empty(), "a file selects one run");
        let err = SpikeGlx.open(&dir, &OpenOptions::default()).err().unwrap().to_string();
        assert!(err.contains("--block") && err.contains("run_g0, run_g1"), "{err}");
        let one = SpikeGlx.open(&dir, &OpenOptions { block: Some("run_g1".into()), ..Default::default() }).unwrap();
        assert_eq!(one.recordings.len(), 2);
        // A file selects its own run
        let s = SpikeGlx.open(&dir.join("run_g0_t0.imec.lf.bin"), &OpenOptions::default()).unwrap();
        assert_eq!(s.recordings.len(), 6);
        let only = SpikeGlx.open(&dir.join("run_g0_t0.imec.ap.meta"), &OpenOptions { only: Some(vec!["imec0.ap".into()]), ..Default::default() }).unwrap();
        assert_eq!(only.recordings.len(), 1);
        assert!(only.validate().is_empty(), "{:?}", only.validate());
    }

    /// Writes `<stem>.meta` and `<stem>.bin` (int16 rows from `row(t)`).
    fn write_file(dir: &Path, stem: &str, meta: &str, samples: usize, row: impl Fn(usize) -> Vec<i16>) {
        std::fs::write(dir.join(format!("{stem}.meta")), meta).unwrap();
        let bin: Vec<u8> = (0..samples).flat_map(|t| row(t).into_iter().flat_map(i16::to_le_bytes)).collect();
        std::fs::write(dir.join(format!("{stem}.bin")), bin).unwrap();
    }

    #[test]
    fn test_onebox_ttl_and_sync_alignment() {
        let dir = fixture_dir("spikeglx-sync");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // True time: the probe clock (30 kHz). Sync: high from k + 0.5 s to k + 1 s.
        let high = |t: f64| t.fract() >= 0.5;
        let ap = NP1_3A.replace("fileSizeBytes=50", "fileSizeBytes=0");
        write_file(&dir, "run_g0_t0.imec.ap", &ap, 90_000, |t| {
            let sync = if high(t as f64 / 30_000.0) { 1 << 6 } else { 0 };
            vec![0, 0, 0, 0, sync]
        });
        // NI at a nominal 10 kHz that really runs 50 ppm fast and started 3 ms late; line 0 is
        // the sync, line 3 a TTL high 0.2–0.25 s and 1.7–1.8 s (true time)
        let ni = "typeThis=nidq\nnSavedChans=2\nniSampRate=10000\nniAiRangeMax=5\nniMaxInt=32768\nsnsMnMaXaDw=0,0,1,1\n\
snsSaveChanSubset=all\nsyncNiChanType=0\nsyncNiChan=0\nsyncSourcePeriod=1\nfileTimeSecs=2.9\n";
        let true_time = |t: usize| 0.003 + t as f64 / 10_000.5;
        write_file(&dir, "run_g0_t0.nidq", ni, 29_000, |t| {
            let tt = true_time(t);
            let ttl = (0.2..0.25).contains(&tt) || (1.7..1.8).contains(&tt);
            vec![0, i16::from(high(tt)) | (i16::from(ttl) << 3)]
        });
        // OneBox: 2 analog inputs, digital word, sync word (no pulse connected)
        let obx = "typeThis=obx\nnSavedChans=4\nobSampRate=30303\nobAiRangeMax=5\nobMaxInt=32768\nsnsXaDwSy=2,1,1\nsnsSaveChanSubset=all\nimDatBsc_sn=42\n";
        write_file(&dir, "run_g0_t0.obx0.obx", obx, 100, |t| vec![1000, -1000, i16::from((10..20).contains(&t)) << 2, 0]);

        let s = nc_core::testkit::check_reader(&SpikeGlx, &dir, &OpenOptions::default());
        let names: Vec<&str> = s.recordings.iter().map(|r| r.info().name.as_str()).collect();
        assert_eq!(names, vec!["imec0.ap", "imec0.ap.sync", "nidq", "nidq.digital", "obx0", "obx0.digital", "obx0.sync"]);
        let obx = s.recording("obx0").unwrap();
        let mut v = vec![0.0; 1];
        obx.read(&[1], 0..1, &mut v).unwrap();
        assert_eq!(v[0], (-1000.0 * 5.0 / 32768.0) as f32);
        assert_eq!(s.metadata.devices.iter().find(|d| d.name == "obx0").unwrap().description, "OneBox recorded by SpikeGLX, serial 42");

        // NI on the probe clock: starts 3 ms late, 50 ppm fast
        let nidq = s.recording("nidq").unwrap().info();
        assert!((nidq.start_time - 0.003).abs() < 2e-4, "start {}", nidq.start_time);
        assert!((nidq.sample_rate - 10_000.5).abs() < 0.05, "rate {}", nidq.sample_rate);
        assert!(s.metadata.notes.iter().any(|n| n.starts_with("nidq aligned to imec0.ap")), "{:?}", s.metadata.notes);
        assert!(s.provenance.warnings.iter().any(|w| w.contains("no sync pulse") && w.contains("obx0")), "{:?}", s.provenance.warnings);

        // TTL lines (the NI sync line is one too); times on the probe clock
        let names: Vec<&str> = s.events.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["nidq TTL 0", "nidq TTL 3", "obx0 TTL 2"]);
        let ttl = s.event_series("nidq TTL 3").unwrap();
        let (on, off) = (&ttl.onsets, ttl.offsets.as_ref().unwrap());
        assert_eq!(on.len(), 2);
        for (got, want) in [(on[0], 0.2), (off[0], 0.25), (on[1], 1.7), (off[1], 1.8)] {
            assert!((got - want).abs() < 2e-4, "{got} vs {want}");
        }
        let obx_ttl = s.event_series("obx0 TTL 2").unwrap();
        assert_eq!((obx_ttl.onsets[0], obx_ttl.offsets.as_ref().unwrap()[0]), (10.0 / 30_303.0, 20.0 / 30_303.0));
    }

    #[test]
    fn test_triggers_open_together() {
        let dir = fixture_dir("spikeglx-triggers");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ni = |first: u64| format!("typeThis=nidq\nnSavedChans=2\nniSampRate=1000\nniAiRangeMax=5\nniMaxInt=32768\nsnsMnMaXaDw=0,0,1,1\nsnsSaveChanSubset=all\nfirstSample={first}\n");
        write_file(&dir, "run_g0_t0.nidq", &ni(5_000), 100, |t| vec![t as i16, i16::from(t >= 50)]);
        write_file(&dir, "run_g0_t1.nidq", &ni(35_000), 100, |t| vec![t as i16, i16::from(t < 10)]);
        assert!(SpikeGlx.containers(&dir).is_empty(), "one gate");
        let s = nc_core::testkit::check_reader(&SpikeGlx, &dir, &OpenOptions::default());
        let names: Vec<&str> = s.recordings.iter().map(|r| r.info().name.as_str()).collect();
        assert_eq!(names, vec!["nidq.digital.t0", "nidq.digital.t1", "nidq.t0", "nidq.t1"]);
        assert_eq!(s.recording("nidq.t1").unwrap().info().start_time, 30.0);
        // One TTL series over both triggers: 0.05–0.1 s, then 30.0–30.01 s
        let e = s.event_series("nidq TTL 0").unwrap();
        assert_eq!(e.onsets, vec![0.05, 30.0]);
        assert_eq!(e.offsets.as_ref().unwrap(), &vec![0.1, 30.01]);
        assert_eq!(s.metadata.extra["spikeglx_triggers"], "0,1");
        // A file opens its whole gate
        assert_eq!(SpikeGlx.open(&dir.join("run_g0_t1.nidq.bin"), &OpenOptions::default()).unwrap().recordings.len(), 4);
    }
}
