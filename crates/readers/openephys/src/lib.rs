//! nc-openephys: Open Ephys GUI recordings in the binary format (GUI 0.4.4 – 0.6+).
//!
//! A save holds `Record Node N/experimentM/recordingK/` folders (GUI ≥ 0.5; older saves have
//! `experimentM/recordingK/` at the top), each described by `structure.oebin` (JSON) with:
//! - `continuous/<stream>/continuous.dat`: int16, channels interleaved per sample, scaled by each
//!   channel's `bit_volts`, plus `sample_numbers.npy` (≥ 0.6; before: `timestamps.npy` holds
//!   sample numbers) and `timestamps.npy` (≥ 0.6: synchronized seconds);
//! - `events/<stream>/TTL/`: `states.npy` (± line number, 1-based; before 0.6
//!   `channel_states.npy`) with `sample_numbers.npy` / `timestamps.npy`; text messages in
//!   `text.npy`.
//!
//! Every recording folder is one container. Each continuous stream becomes up to three recordings:
//! the electrode channels (`<stream>`, volts, electrical), other analog channels (ADC / AUX / NI-DAQ
//! inputs, `<stream>.analog`, volts) and a Neuropixels sync line (`<stream>.sync`). Electrodes: one
//! per electrode channel, one group per probe or stream; Neuropixels AP and LFP share electrodes and
//! get site positions from `settings.xml`. Streams are placed on one time axis (≥ 0.6 the GUI's
//! synchronized timestamps; before, sample numbers / rate); TTL lines become events (high
//! periods), messages become labelled events.
//!
//! The legacy format (one `.continuous` file per channel, `.events`, `.spikes`) is read by
//! [`legacy`]; each acquisition start is a container.
//!
//! Not yet: binary-format spikes, OneBox ADC streams, Open Ephys's own NWB format (already NWB).

#![warn(missing_docs)]

/// This crate's version (`nc-openephys`, from its `Cargo.toml`): recorded in every conversion's
/// provenance and report, so a problem in a file can be traced to the code that wrote it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// [`VERSION`].
pub fn version() -> &'static str {
    VERSION
}

pub mod legacy;
pub mod npy;
pub mod settings;

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nc_base::mapped::MappedFile;
use nc_base::time::format_iso;
use nc_core::{
    check_read, Calibration, ChannelInfo, ChannelRef, Detection, Device, Electrode, ElectrodeGroup, Error, EventSeries, MemoryOrder, OpenOptions,
    Provenance, Reader, Recording, RecordingInfo, Result, SampleType, Session, SignalKind,
};
use npy::Npy;
use serde_json::Value;

/// The Open Ephys reader (binary and legacy formats).
pub struct OpenEphys;

/// Recording folders (with a `structure.oebin`) at or below `path`, sorted.
fn recordings(path: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if dir.join("structure.oebin").is_file() {
            out.push(dir.to_path_buf());
            return;
        }
        if depth == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                walk(&e.path(), depth - 1, out);
            }
        }
    }
    let start = if path.file_name().is_some_and(|n| n == "structure.oebin") { path.parent().unwrap_or(path) } else { path };
    let mut out = Vec::new();
    walk(start, 4, &mut out);
    out.sort();
    out
}

/// The name of a recording folder relative to what was opened (`Record Node 104/experiment1/recording1`).
fn container_name(root: &Path, rec: &Path) -> String {
    let rel = rec.strip_prefix(root).unwrap_or(rec);
    let name = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
    if name.is_empty() { rec.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { name }
}

impl Reader for OpenEphys {
    fn name(&self) -> &'static str {
        "openephys"
    }
    fn version(&self) -> &'static str {
        crate::VERSION
    }

    /// Compared with its reference reader on real data (docs/formats).
    fn maturity(&self) -> nc_core::Maturity {
        nc_core::Maturity::Verified
    }

    fn description(&self) -> &'static str {
        "Open Ephys GUI recording: binary format (structure.oebin + continuous.dat) or legacy format (.continuous)"
    }

    fn opens(&self) -> &'static str {
        "An Open Ephys save folder (Record Node / experiment / recording folders), or one recording folder or its structure.oebin"
    }

    fn versions(&self) -> &'static [&'static str] {
        &[
            "Binary format, GUI 0.4.4 – 0.6+ (sample_numbers.npy from 0.6, timestamps.npy before)",
            "Continuous streams (headstage / Neuropixels / NI-DAQ), TTL events, text messages",
            "Legacy format (.continuous / .events / .spikes, GUI ≤ 0.4.x): channels, TTL events, messages, spikes; one container per start",
        ]
    }

    fn detect(&self, path: &Path) -> Option<Detection> {
        let recs = recordings(path);
        let Some(first) = recs.first() else {
            let starts: usize = legacy::folders(path).iter().map(|d| legacy::starts(d).len()).sum();
            return (starts > 0).then(|| Detection {
                format: "openephys",
                version: Some(format!("legacy format{}", if starts > 1 { format!(", {starts} recordings") } else { String::new() })),
                confidence: 0.9,
            });
        };
        let oebin: Value = serde_json::from_str(&std::fs::read_to_string(first.join("structure.oebin")).ok()?).ok()?;
        let version = oebin.get("GUI version").and_then(Value::as_str).map(|v| format!("GUI {v}{}", if recs.len() > 1 { format!(", {} recordings", recs.len()) } else { String::new() }));
        Some(Detection { format: "openephys", version, confidence: 0.95 })
    }

    fn containers(&self, path: &Path) -> Vec<String> {
        let recs = recordings(path);
        if recs.is_empty() {
            let names = legacy_containers(path);
            return if names.len() > 1 { names.into_iter().map(|(n, _, _)| n).collect() } else { Vec::new() };
        }
        if recs.len() > 1 { recs.iter().map(|r| container_name(path, r)).collect() } else { Vec::new() }
    }

    fn open(&self, path: &Path, options: &OpenOptions) -> Result<Session> {
        let recs = recordings(path);
        if recs.is_empty() {
            let all = legacy_containers(path);
            let chosen = match (&options.block, all.len()) {
                (Some(name), _) => all.iter().find(|(n, _, _)| n == name).ok_or_else(|| Error::Unsupported(format!("{name}: no such recording in {}", path.display())))?,
                (None, 1) => &all[0],
                (None, 0) => return Err(Error::format("openephys", format!("no structure.oebin or .continuous files in {}", path.display()))),
                (None, n) => {
                    return Err(Error::Unsupported(format!(
                        "{} holds {n} Open Ephys recordings; choose one with --block <name>: {}",
                        path.display(),
                        all.iter().map(|(n, _, _)| n.as_str()).collect::<Vec<_>>().join(", ")
                    )))
                }
            };
            return legacy::open(&chosen.1, chosen.2, options);
        }
        let rec = match (&options.block, recs.len()) {
            (Some(name), _) => recs.iter().find(|r| &container_name(path, r) == name).cloned().ok_or_else(|| Error::Unsupported(format!("{name}: no such recording in {}", path.display())))?,
            (None, 1) => recs[0].clone(),
            (None, 0) => return Err(Error::format("openephys", format!("no structure.oebin in {}", path.display()))),
            (None, n) => {
                return Err(Error::Unsupported(format!(
                    "{} holds {n} Open Ephys recordings; choose one with --block <name>: {}",
                    path.display(),
                    recs.iter().map(|r| container_name(path, r)).collect::<Vec<_>>().join(", ")
                )))
            }
        };
        open_recording(&rec, options)
    }
}

/// Legacy containers: (name, folder, start). `experiment<n>`, prefixed by the folder when the
/// opened path holds several (`Record Node 120/experiment1`).
fn legacy_containers(path: &Path) -> Vec<(String, PathBuf, u32)> {
    let root = if path.is_file() { path.parent().unwrap_or(path) } else { path };
    let dirs = legacy::folders(path);
    let mut out = Vec::new();
    for d in &dirs {
        let rel = d.strip_prefix(root).ok().map(|r| r.to_string_lossy().into_owned()).filter(|r| !r.is_empty());
        for start in legacy::starts(d) {
            let name = rel.as_ref().map_or_else(|| format!("experiment{start}"), |r| format!("{r}/experiment{start}"));
            out.push((name, d.clone(), start));
        }
    }
    out
}

/// int16 samples interleaved per sample in `continuous.dat`; a recording uses some columns.
pub struct DatRecording {
    info: RecordingInfo,
    file: Arc<MappedFile>,
    columns: usize,
    selected: Vec<usize>,
}

impl DatRecording {
    fn each(&self, channels: &[usize], samples: Range<u64>, mut f: impl FnMut(usize, usize, [u8; 2])) {
        let bytes = self.file.bytes();
        let row = self.columns * 2;
        let cols: Vec<usize> = channels.iter().map(|&c| self.selected[c] * 2).collect();
        for (t, s) in (samples.start as usize..samples.end as usize).enumerate() {
            let base = s * row;
            for (i, &c) in cols.iter().enumerate() {
                f(i, t, [bytes[base + c], bytes[base + c + 1]]);
            }
        }
    }
}

impl Recording for DatRecording {
    fn info(&self) -> &RecordingInfo {
        &self.info
    }

    fn read(&self, channels: &[usize], samples: Range<u64>, out: &mut [f32]) -> Result<()> {
        let n = check_read(&self.info, channels, &samples, out.len())?;
        let gains: Vec<f64> = channels.iter().map(|&c| self.info.channels[c].gain).collect();
        self.each(channels, samples, |i, t, b| out[i * n + t] = (i16::from_le_bytes(b) as f64 * gains[i]) as f32);
        Ok(())
    }

    fn read_stored(&self, channels: &[usize], samples: Range<u64>, out: &mut [u8]) -> Result<bool> {
        let n = check_read(&self.info, channels, &samples, out.len() / 2)?;
        if out.len() != channels.len() * n * 2 {
            return Err(Error::BufferSize { expected: channels.len() * n * 2, actual: out.len() });
        }
        self.each(channels, samples, |i, t, b| {
            let at = (i * n + t) * 2;
            out[at..at + 2].copy_from_slice(&b);
        });
        Ok(true)
    }
}

/// A continuous stream of the recording, as far as timing needs it.
struct Clock {
    folder: String,
    name: String,
    rate: f64,
    first_sample: i64,
    /// Seconds of the first sample on the common axis (before subtracting the session start).
    first_time: f64,
}

/// Where a channel goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Electrode,
    Analog,
    Sync,
}

fn role(ch: &Value) -> Role {
    let name = ch.get("channel_name").and_then(Value::as_str).unwrap_or("");
    let desc = ch.get("description").and_then(Value::as_str).unwrap_or("").to_lowercase();
    if name.ends_with("_SYNC") || desc.contains("sync line") {
        Role::Sync
    } else if name.contains("ADC") || name.contains("AUX") || desc.contains("adc") || desc.contains("analog") || desc.contains("aux") {
        Role::Analog
    } else {
        Role::Electrode
    }
}

/// Volts per bit of a channel. `bit_volts` is in µV for headstage / probe channels (unless the
/// channel says otherwise) and in volts for analog inputs (Open Ephys's documentation).
fn volts_per_bit(ch: &Value, role: Role) -> f64 {
    let bit = ch.get("bit_volts").and_then(Value::as_f64).unwrap_or(1.0);
    match role {
        Role::Sync => 1.0,
        Role::Analog => bit,
        Role::Electrode => match ch.get("units").and_then(Value::as_str).unwrap_or("") {
            "V" => bit,
            "mV" => bit * 1e-3,
            _ => bit * 1e-6,
        },
    }
}

fn ints(path: &Path) -> Result<Vec<i64>> {
    Npy::load(path)?.ints().map_err(|e| Error::format("openephys", format!("{}: {e}", path.display())))
}

fn floats(path: &Path) -> Result<Vec<f64>> {
    Npy::load(path)?.floats().map_err(|e| Error::format("openephys", format!("{}: {e}", path.display())))
}

/// The first existing file among `names` in `dir`.
fn first_of(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names.iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// `settings.xml` for a recording: in its experiment's parent (`settings_<N>.xml` for experiment N
/// when present).
fn settings_for(rec: &Path) -> Option<PathBuf> {
    let experiment = rec.parent()?;
    let n = experiment.file_name()?.to_string_lossy().strip_prefix("experiment").map(str::to_string);
    let mut dir = experiment.parent();
    for _ in 0..2 {
        let d = dir?;
        if let Some(p) = n.as_ref().map(|n| d.join(format!("settings_{n}.xml"))).filter(|p| p.is_file() && n.as_deref() != Some("1")) {
            return Some(p);
        }
        if d.join("settings.xml").is_file() {
            return Some(d.join("settings.xml"));
        }
        dir = d.parent();
    }
    None
}

/// `Software Time (milliseconds since midnight Jan 1st 1970 UTC): 1693464096435` → seconds.
fn software_time(sync: &str) -> Option<f64> {
    sync.lines().find(|l| l.contains("milliseconds since midnight Jan 1st 1970 UTC")).and_then(|l| l.rsplit(':').next()?.trim().parse::<f64>().ok()).map(|ms| ms / 1000.0)
}

fn open_recording(rec: &Path, options: &OpenOptions) -> Result<Session> {
    let oebin_path = rec.join("structure.oebin");
    let oebin: Value = serde_json::from_str(&std::fs::read_to_string(&oebin_path).map_err(|e| Error::io(&oebin_path, e))?)
        .map_err(|e| Error::format("openephys", format!("{}: {e}", oebin_path.display())))?;
    let gui = oebin.get("GUI version").and_then(Value::as_str).unwrap_or("?").to_string();
    let mut s = Session::default();
    let mut warnings = Vec::new();
    let mut files = vec![oebin_path.clone()];
    let settings_path = settings_for(rec);
    let settings = settings_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| settings::parse(&t)).unwrap_or_default();
    if let Some(p) = &settings_path {
        files.push(p.clone());
    }

    // Continuous streams
    let mut clocks: Vec<Clock> = Vec::new();
    // (info, file, columns per sample, columns used, electrode group of electrode channels)
    type Pending = (RecordingInfo, Arc<MappedFile>, usize, Vec<usize>, Option<String>);
    let mut pending: Vec<Pending> = Vec::new();
    for c in oebin.get("continuous").and_then(Value::as_array).cloned().unwrap_or_default() {
        let folder = c.get("folder_name").and_then(Value::as_str).unwrap_or("").trim_end_matches('/').to_string();
        let base = c.get("stream_name").and_then(Value::as_str).filter(|s| !s.is_empty()).map_or_else(|| folder.clone(), str::to_string);
        let rate = c.get("sample_rate").and_then(Value::as_f64).unwrap_or(0.0);
        let chans = c.get("channels").and_then(Value::as_array).cloned().unwrap_or_default();
        let dir = rec.join("continuous").join(&folder);
        let dat = dir.join("continuous.dat");
        let file = match MappedFile::open(&dat) {
            Ok(f) => Arc::new(f),
            Err(e) => {
                warnings.push(format!("{base}: skipped ({e})"));
                continue;
            }
        };
        files.push(dat.clone());
        let columns = chans.len().max(1);
        let samples = file.len() / (2 * columns as u64);
        if file.len() % (2 * columns as u64) != 0 {
            warnings.push(format!("{base}: continuous.dat ends with a partial sample"));
        }
        // Timing: sample numbers (≥ 0.6) or integer timestamps (before); synchronized seconds (≥ 0.6)
        let numbers = first_of(&dir, &["sample_numbers.npy"]).map(|p| ints(&p)).or_else(|| first_of(&dir, &["timestamps.npy"]).filter(|_| !dir.join("sample_numbers.npy").exists()).map(|p| ints(&p))).transpose()?;
        let seconds = dir.join("sample_numbers.npy").is_file().then(|| first_of(&dir, &["timestamps.npy"]).map(|p| floats(&p))).flatten().transpose()?;
        let first_sample = numbers.as_ref().and_then(|n| n.first().copied()).unwrap_or(0);
        if let Some(n) = &numbers
            && let (Some(a), Some(b)) = (n.first(), n.last())
            && ((b - a + 1) as u64 != samples || n.len() as u64 != samples)
        {
            warnings.push(format!("{base}: sample numbers run {a}–{b} for {samples} samples (gaps or dropped samples)"));
        }
        let first_time = seconds.as_ref().and_then(|t| t.first().copied()).unwrap_or(first_sample as f64 / rate.max(1e-9));
        clocks.push(Clock { folder: folder.clone(), name: base.clone(), rate, first_sample, first_time });

        let probe = base.strip_suffix("-AP").or_else(|| base.strip_suffix("-LFP")).map(str::to_string);
        for r in [Role::Electrode, Role::Analog, Role::Sync] {
            let cols: Vec<usize> = (0..chans.len()).filter(|&i| role(&chans[i]) == r).collect();
            if cols.is_empty() {
                continue;
            }
            let (name, kind, unit, description) = match r {
                Role::Electrode => (base.clone(), SignalKind::Electrical, "V", format!("Electrode channels of {base}")),
                Role::Analog => (format!("{base}.analog"), SignalKind::Other, "V", format!("Analog inputs of {base}")),
                Role::Sync => (format!("{base}.sync"), SignalKind::Other, "a.u.", format!("Sync line of {base}")),
            };
            if !options.wants(&name) {
                continue;
            }
            let channels = cols
                .iter()
                .map(|&i| ChannelInfo { name: chans[i].get("channel_name").and_then(Value::as_str).unwrap_or("?").to_string(), gain: volts_per_bit(&chans[i], r), offset: 0.0 })
                .collect();
            let mut metadata = BTreeMap::new();
            metadata.insert("openephys_folder".to_string(), folder.clone());
            metadata.insert("openephys_first_sample".to_string(), first_sample.to_string());
            let info = RecordingInfo {
                name,
                description,
                channels,
                samples,
                sample_rate: rate,
                start_time: first_time, // made relative below
                unit: unit.into(),
                calibration: Calibration::Known,
                kind,
                stored_as: SampleType::I16,
                order: MemoryOrder::TimeMajor,
                storage: "dat".into(),
                metadata,
            };
            pending.push((info, file.clone(), columns, cols, (r == Role::Electrode).then(|| probe.clone().unwrap_or_else(|| base.clone()))));
        }
    }
    let t0 = clocks.iter().map(|c| c.first_time).fold(f64::INFINITY, f64::min);
    let t0 = if t0.is_finite() { t0 } else { 0.0 };

    // Electrodes: one per electrode channel, keyed by probe (AP and LFP share)
    let mut electrode_of: BTreeMap<(String, usize), usize> = BTreeMap::new();
    for (mut info, file, columns, cols, group) in pending {
        info.start_time -= t0;
        if let Some(group) = group {
            // ProbeA → the first NP_PROBE of settings.xml, ProbeB → the second, …
            let probe = group.strip_prefix("Probe").filter(|l| l.len() == 1).and_then(|l| l.bytes().next()).filter(u8::is_ascii_uppercase).map(|b| (b - b'A') as usize).and_then(|i| settings.probes.get(i));
            if s.electrode_group(&group).is_none() {
                let description = probe.map_or_else(|| format!("Electrodes of {group}"), |p| format!("{} {}", p.model, p.serial).trim().to_string());
                s.electrode_groups.push(ElectrodeGroup { name: group.clone(), description, location: "unknown".into(), device: Some(group.clone()) });
                s.metadata.devices.push(Device {
                    name: group.clone(),
                    description: probe.map_or_else(|| format!("Acquisition source of {group}"), |p| format!("Neuropixels probe {} (serial {})", p.model, p.serial)),
                    manufacturer: probe.map(|_| "IMEC".to_string()),
                    model: probe.map(|p| p.model.clone()),
                });
            }
            for (k, _) in cols.iter().enumerate() {
                let link = ChannelRef { recording: info.name.clone(), channel: k };
                match electrode_of.get(&(group.clone(), k)) {
                    Some(&e) => s.electrodes[e].channels.push(link),
                    None => {
                        let position = probe.and_then(|p| p.positions.get(k).copied().flatten()).map(|[x, y]| [x, y, 0.0]);
                        s.electrodes.push(Electrode { name: format!("{group} ch{}", k + 1), group: group.clone(), channels: vec![link], position_um: position, ..Default::default() });
                        electrode_of.insert((group.clone(), k), s.electrodes.len() - 1);
                    }
                }
            }
        }
        s.recordings.push(Arc::new(DatRecording { info, file, columns, selected: cols }));
    }

    // Events
    for e in oebin.get("events").and_then(Value::as_array).cloned().unwrap_or_default() {
        let folder = e.get("folder_name").and_then(Value::as_str).unwrap_or("").trim_end_matches('/').to_string();
        let dir = rec.join("events").join(&folder);
        let stream = e.get("stream_name").and_then(Value::as_str).unwrap_or("");
        let clock = clocks.iter().find(|c| !stream.is_empty() && c.name == stream).or_else(|| clocks.iter().find(|c| folder.starts_with(&c.folder)));
        let rate = e.get("sample_rate").and_then(Value::as_f64).or(clock.map(|c| c.rate)).unwrap_or(1.0);
        // Seconds on the common axis for every event
        let numbers = first_of(&dir, &["sample_numbers.npy"]).or_else(|| first_of(&dir, &["timestamps.npy"]).filter(|_| !dir.join("sample_numbers.npy").exists()));
        let synced = dir.join("sample_numbers.npy").is_file().then(|| dir.join("timestamps.npy")).filter(|p| p.is_file());
        let times: Vec<f64> = match (&synced, &numbers) {
            (Some(p), _) => floats(p)?.into_iter().map(|t| t - t0).collect(),
            (None, Some(p)) => ints(p)?.into_iter().map(|n| match clock {
                Some(c) => (n - c.first_sample) as f64 / c.rate + (c.first_time - t0),
                None => n as f64 / rate - t0,
            }).collect(),
            (None, None) => {
                warnings.push(format!("events {folder}: no timestamps"));
                continue;
            }
        };
        files.extend(numbers.iter().chain(synced.iter()).cloned());
        let label = if stream.is_empty() { folder.split('/').next().unwrap_or(&folder).to_string() } else { stream.to_string() };
        let ty = e.get("type").and_then(Value::as_str).unwrap_or("");
        if ty == "string" {
            let Some(p) = first_of(&dir, &["text.npy"]) else { continue };
            let text = Npy::load(&p)?.strings().map_err(|m| Error::format("openephys", m))?;
            if text.is_empty() {
                continue;
            }
            let name = "messages".to_string();
            if options.wants(&name) && s.event_series(&name).is_none() {
                s.events.push(EventSeries { name, description: "Messages from the GUI's Message Center".into(), onsets: times.clone(), offsets: None, values: vec![0.0; text.len()], channels: 1, labels: text });
            }
            continue;
        }
        let Some(states_path) = first_of(&dir, &["states.npy", "channel_states.npy"]) else { continue };
        let states = ints(&states_path)?;
        files.push(states_path);
        let mut lines: Vec<i64> = states.iter().map(|s| s.abs()).filter(|&l| l > 0).collect();
        lines.sort_unstable();
        lines.dedup();
        for line in lines {
            let name = format!("{label} TTL {line}");
            if !options.wants(&name) {
                continue;
            }
            let (mut onsets, mut offsets) = (Vec::new(), Vec::new());
            let mut high: Option<f64> = None;
            for (&st, &t) in states.iter().zip(&times) {
                if st == line && high.is_none() {
                    high = Some(t);
                } else if st == -line
                    && let Some(on) = high.take()
                {
                    onsets.push(on);
                    offsets.push(t);
                }
            }
            if let Some(on) = high {
                onsets.push(on);
                offsets.push(f64::NAN);
            }
            if onsets.is_empty() {
                continue;
            }
            // A line still high at the end: offset at the end of the recording
            let end = s.duration();
            for o in offsets.iter_mut().filter(|o| o.is_nan()) {
                *o = end;
            }
            let n = onsets.len();
            let mut order: Vec<usize> = (0..n).collect();
            order.sort_by(|&a, &b| onsets[a].total_cmp(&onsets[b]));
            s.events.push(EventSeries {
                name,
                description: format!("TTL line {line} of {label} (high periods)"),
                onsets: order.iter().map(|&i| onsets[i]).collect(),
                offsets: Some(order.iter().map(|&i| offsets[i]).collect()),
                values: vec![1.0; n],
                channels: 1,
                labels: Vec::new(),
            });
        }
    }

    // Start time: the GUI's wall clock (UTC) when recorded (≥ 0.6), else the settings' date
    let sync_text = std::fs::read_to_string(rec.join("sync_messages.txt")).ok();
    let m = &mut s.metadata;
    m.start_time = match sync_text.as_deref().and_then(software_time) {
        Some(secs) => Some(format!("{}Z", format_iso(secs))),
        None => settings.date.as_deref().and_then(settings::iso_date),
    };
    if m.start_time.is_none() {
        warnings.push("no start time in sync_messages.txt or settings.xml; set session.start_time".into());
    }
    let parts: Vec<String> = rec.components().rev().take(3).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    m.experiment = Some(parts.into_iter().rev().collect::<Vec<_>>().join("/"));
    m.extra.insert("openephys_gui_version".into(), gui.clone());
    for c in &clocks {
        m.extra.insert(format!("openephys_{}_first_sample", c.name), c.first_sample.to_string());
    }
    let mut prov = Provenance::new("openephys");
    prov.version = Some(format!("Open Ephys GUI {gui}"));
    files.sort();
    files.dedup();
    for f in &files {
        prov.add_file(f);
    }
    prov.warnings = warnings;
    s.provenance = prov;
    s.recordings.sort_by(|a, b| a.info().name.cmp(&b.info().name));
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roles_and_scales() {
        let ch = |name: &str, desc: &str, bit: f64, units: &str| serde_json::json!({ "channel_name": name, "description": desc, "bit_volts": bit, "units": units });
        let ap = ch("AP1", "Neuropixels electrode", 0.195, "");
        assert_eq!(role(&ap), Role::Electrode);
        assert!((volts_per_bit(&ap, Role::Electrode) - 0.195e-6).abs() < 1e-15);
        let adc = ch("ADC1", "ADC data channel", 0.000152588, "uV");
        assert_eq!((role(&adc), volts_per_bit(&adc, Role::Analog)), (Role::Analog, 0.000152588), "ADC bit_volts are volts whatever `units` says");
        assert_eq!(role(&ch("AI0", "Analog Input channel from a NIDAQ device", 3e-4, "")), Role::Analog);
        assert_eq!(role(&ch("AP_SYNC", "Neuropixels sync line (continuously sampled)", 1.0, "")), Role::Sync);
        assert_eq!(software_time("Software Time (milliseconds since midnight Jan 1st 1970 UTC): 1693464096435\nStart Time for X: 1"), Some(1_693_464_096.435));
        assert_eq!(container_name(Path::new("/a"), Path::new("/a/Record Node 104/experiment1/recording1")), "Record Node 104/experiment1/recording1");
    }
}
