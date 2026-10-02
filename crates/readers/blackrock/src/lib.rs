//! nc-blackrock: Blackrock Microsystems recordings (Cerebus / NeuroPort / Gemini): `.ns1`–`.ns6`
//! continuous files and the `.nev` event file of one recording (same base name).
//!
//! - Each NSx file is one stream `ns<N>` (its own sample rate): front-end channels (units µV) are
//!   electrical and get electrodes (grouped by connector bank, shared across NSx files by electrode
//!   id); other channels (analog inputs) become `ns<N>.analog`. Samples stay int16 with each
//!   channel's gain and offset from its digital / analog ranges (2.1: from the NEV's
//!   digitization factor).
//! - Recording pauses (several data blocks) and PTP gaps make several parts: `ns5.p1`, `ns5.p2`, …
//!   each starting at its own timestamp.
//! - The NEV gives the spikes (one snippet store `spikes`: channel = electrode id, sort code =
//!   unit class, volts from the digitization factor), digital and serial input words (events with
//!   the word as value) and comments (labelled events).
//! - A clock reset (timestamps starting over; 2.3+) splits the recording into containers
//!   `segment1`, `segment2`, … (neo's segments), each on its own time axis.
//!
//! Times are seconds from the earliest timestamp of the container; the start time is the NSx
//! (else NEV) time origin, in UTC.

/// This crate's version (`nc-blackrock`, from its `Cargo.toml`): recorded in every conversion's
/// provenance and report, so a problem in a file can be traced to the code that wrote it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// [`VERSION`].
pub fn version() -> &'static str {
    VERSION
}

pub mod nev;
pub mod nsx;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nc_core::{
    Calibration, ChannelInfo, ChannelRef, Detection, Device, Electrode, ElectrodeGroup, Error, EventSeries, MemoryOrder, OpenOptions, Provenance,
    Reader, RecordingInfo, Result, SampleType, Session, SignalKind, SnippetSeries,
};
use nev::{Nev, NevWaveforms};
use nsx::{Nsx, NsxRecording};

pub struct Blackrock;

/// The files of one recording: `<base>.nev` and `<base>.ns<N>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileSet {
    pub base: PathBuf,
    pub nev: Option<PathBuf>,
    /// (N, path), ascending.
    pub nsx: Vec<(u8, PathBuf)>,
}

fn signature(path: &Path) -> Option<[u8; 8]> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut b = [0u8; 8];
    f.read_exact(&mut b).ok()?;
    Some(b)
}

/// Recordings (file sets) at `path`: a file selects its own base; a folder lists every base in it.
pub fn file_sets(path: &Path) -> Vec<FileSet> {
    let (dir, only) = if path.is_file() { (path.parent().unwrap_or(Path::new(".")), path.file_stem().map(|s| s.to_string_lossy().into_owned())) } else { (path, None) };
    let mut sets: BTreeMap<String, FileSet> = BTreeMap::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    paths.sort();
    for p in paths {
        let (Some(stem), Some(ext)) = (p.file_stem().map(|s| s.to_string_lossy().into_owned()), p.extension().map(|e| e.to_string_lossy().to_lowercase())) else { continue };
        if only.as_ref().is_some_and(|o| *o != stem) {
            continue;
        }
        let sig = signature(&p);
        let new = || FileSet { base: dir.join(&stem), ..Default::default() };
        if ext == "nev" && sig.is_some_and(|s| &s == b"NEURALEV" || &s == b"BREVENTS") {
            sets.entry(stem.clone()).or_insert_with(new).nev = Some(p.clone());
        } else if let Some(n) = ext.strip_prefix("ns").and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=9).contains(n))
            && sig.is_some_and(|s| &s == b"NEURALCD" || &s == b"BRSMPGRP" || &s == b"NEURALSG")
        {
            sets.entry(stem.clone()).or_insert_with(new).nsx.push((n, p.clone()));
        }
    }
    sets.into_values().collect()
}

/// Containers: (name, file set, epoch). One per file set and clock epoch; names are the base
/// (when the folder holds several) and `segment<k>` (when the clock was reset).
fn containers(path: &Path) -> Result<Vec<(String, FileSet, usize)>> {
    let sets = file_sets(path);
    let mut out = Vec::new();
    for set in &sets {
        let epochs = Opened::open(set)?.epochs();
        let base = set.base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        for e in 0..epochs {
            let name = match (sets.len() > 1, epochs > 1) {
                (true, true) => format!("{base}/segment{}", e + 1),
                (true, false) => base.clone(),
                (false, true) => format!("segment{}", e + 1),
                (false, false) => base.clone(),
            };
            out.push((name, set.clone(), e));
        }
    }
    Ok(out)
}

/// The parsed files of a set.
struct Opened {
    nev: Option<Arc<Nev>>,
    nsx: Vec<(u8, Nsx)>,
    /// Per NSx file: the epoch of each part.
    part_epochs: Vec<Vec<usize>>,
}

impl Opened {
    fn open(set: &FileSet) -> Result<Self> {
        let nev = set.nev.as_ref().map(|p| Nev::open(p)).transpose()?.map(Arc::new);
        let mut nsx = Vec::new();
        for (n, p) in &set.nsx {
            nsx.push((*n, Nsx::open(p)?));
        }
        // A part starting more than a second before the previous part's end means the clock was
        // reset (smaller steps back are PTP jitter)
        let part_epochs = nsx
            .iter()
            .map(|(_, f)| {
                let mut epoch = 0;
                let mut end = 0u64;
                let ticks_per_sample = f.timestamp_resolution as f64 * f.period as f64 / 30_000.0;
                f.parts
                    .iter()
                    .enumerate()
                    .map(|(k, p)| {
                        if k > 0 && p.timestamp + f.timestamp_resolution < end {
                            epoch += 1;
                        }
                        end = p.timestamp + (p.samples as f64 * ticks_per_sample) as u64;
                        epoch
                    })
                    .collect()
            })
            .collect();
        Ok(Self { nev, nsx, part_epochs })
    }

    fn epochs(&self) -> usize {
        let nsx = self.part_epochs.iter().filter_map(|e| e.last().map(|l| l + 1)).max().unwrap_or(0);
        nsx.max(self.nev.as_ref().map_or(0, |n| n.epochs)).max(1)
    }
}

impl Reader for Blackrock {
    fn name(&self) -> &'static str {
        "blackrock"
    }
    fn version(&self) -> &'static str {
        crate::VERSION
    }

    /// Compared with its reference reader on real data (docs/formats).
    fn maturity(&self) -> nc_core::Maturity {
        nc_core::Maturity::Verified
    }

    fn description(&self) -> &'static str {
        "Blackrock Microsystems recording (Cerebus / NeuroPort / Gemini): .ns1–.ns6 + .nev"
    }

    fn opens(&self) -> &'static str {
        "A folder with .nsX / .nev files, or one of them (the files sharing its base name open together)"
    }

    fn versions(&self) -> &'static [&'static str] {
        &[
            "File spec 2.1 (NEURALSG / NEURALEV), 2.2, 2.3 (NEURALCD), 3.0 (BRSMPGRP / BREVENTS), 3.0 with PTP timestamps",
            "Continuous channels with pauses, NEV spikes (waveforms, unit classes), digital / serial input, comments; clock resets as segments",
        ]
    }

    fn detect(&self, path: &Path) -> Option<Detection> {
        let sets = file_sets(path);
        let first = sets.first()?;
        let any = first.nev.as_ref().or(first.nsx.first().map(|(_, p)| p))?;
        let sig = signature(any)?;
        let version = match &sig {
            b"NEURALSG" => "file spec 2.1".to_string(),
            _ => {
                let b = std::fs::read(any).ok()?;
                format!("file spec {}.{}", b.get(8)?, b.get(9)?)
            }
        };
        let version = if sets.len() > 1 { format!("{version}, {} recordings", sets.len()) } else { version };
        Some(Detection { format: "blackrock", version: Some(version), confidence: 0.95 })
    }

    fn containers(&self, path: &Path) -> Vec<String> {
        let all = containers(path).unwrap_or_default();
        if all.len() > 1 { all.into_iter().map(|(n, _, _)| n).collect() } else { Vec::new() }
    }

    fn open(&self, path: &Path, options: &OpenOptions) -> Result<Session> {
        let all = containers(path)?;
        let chosen = match (&options.block, all.len()) {
            (Some(name), _) => all.iter().find(|(n, _, _)| n == name).ok_or_else(|| Error::Unsupported(format!("{name}: no such recording in {}", path.display())))?,
            (None, 1) => &all[0],
            (None, 0) => return Err(Error::format("blackrock", format!("no .nsX / .nev files in {}", path.display()))),
            (None, n) => {
                return Err(Error::Unsupported(format!(
                    "{} holds {n} Blackrock recordings; choose one with --block <name>: {}",
                    path.display(),
                    all.iter().map(|(n, _, _)| n.as_str()).collect::<Vec<_>>().join(", ")
                )))
            }
        };
        open_set(&chosen.1, chosen.2, options)
    }
}

fn bank(connector: u8) -> String {
    match connector {
        1..=26 => format!("bank {}", (b'A' + connector - 1) as char),
        _ => "bank ?".into(),
    }
}

/// `[year, month, weekday, day, hour, minute, second, ms]` (UTC) → ISO 8601 with `Z`.
fn origin_iso(o: &[u16; 8]) -> Option<String> {
    if o[0] < 1970 || o[1] == 0 || o[3] == 0 {
        return None;
    }
    Some(format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z", o[0], o[1], o[3], o[4], o[5], o[6], o[7]))
}

fn open_set(set: &FileSet, epoch: usize, options: &OpenOptions) -> Result<Session> {
    let opened = Opened::open(set)?;
    let mut s = Session::default();
    let mut warnings: Vec<String> = Vec::new();
    let mut files: Vec<PathBuf> = set.nsx.iter().map(|(_, p)| p.clone()).chain(set.nev.clone()).collect();
    files.sort();
    if let Some(nev) = &opened.nev {
        warnings.extend(nev.warnings.iter().cloned());
    }

    // Time zero: the earliest timestamp of this epoch (NSx parts and NEV packets), kept in ticks
    // so times on the same clock subtract exactly
    let mut zero: Option<(u64, u64)> = None;
    let mut consider = |ticks: u64, res: u64| {
        if zero.is_none_or(|(t, r)| (ticks as f64 / res as f64) < (t as f64 / r as f64)) {
            zero = Some((ticks, res));
        }
    };
    for ((_, f), epochs) in opened.nsx.iter().zip(&opened.part_epochs) {
        for (p, _) in f.parts.iter().zip(epochs).filter(|(p, e)| **e == epoch && p.samples >= 2) {
            consider(p.timestamp, f.timestamp_resolution);
        }
    }
    if let Some(nev) = &opened.nev
        && let Some(p) = nev.packets.iter().filter(|p| p.epoch == epoch).min_by_key(|p| p.timestamp)
    {
        consider(p.timestamp, nev.timestamp_resolution);
    }
    let (z_ticks, z_res) = zero.unwrap_or((0, 1));
    let since = move |ticks: u64, res: u64| if res == z_res { (ticks as i128 - z_ticks as i128) as f64 / res as f64 } else { ticks as f64 / res as f64 - z_ticks as f64 / z_res as f64 };

    // Electrodes: one per electrode id, grouped by connector bank
    let mut electrode_of: BTreeMap<u32, usize> = BTreeMap::new();
    let nev_scale = |id: u32| opened.nev.as_ref().and_then(|n| n.waves.get(&(id as u16))).map(|w| if w.digitization == 21516 { 152_592.547 } else { w.digitization as f64 } * 1e-9);
    for ((n, f), epochs) in opened.nsx.iter().zip(&opened.part_epochs) {
        warnings.extend(f.warnings.iter().map(|w| format!("ns{n}: {w}")));
        let rate = f.sample_rate();
        let electrical: Vec<usize> = (0..f.channels.len()).filter(|&i| if f.spec == "2.1" { f.channels[i].id < 129 } else { f.channels[i].units.trim() == "uV" }).collect();
        let analog: Vec<usize> = (0..f.channels.len()).filter(|i| !electrical.contains(i)).collect();
        let parts: Vec<(usize, &nsx::Part)> = f.parts.iter().enumerate().filter(|(k, p)| epochs[*k] == epoch && p.samples >= 2).collect();
        let skipped = f.parts.iter().enumerate().filter(|(k, p)| epochs[*k] == epoch && p.samples < 2).count();
        if skipped > 0 {
            warnings.push(format!("ns{n}: {skipped} data blocks with fewer than 2 samples skipped (as neo)"));
        }
        for (pi, (_, part)) in parts.iter().enumerate() {
            let suffix = if parts.len() > 1 { format!(".p{}", pi + 1) } else { String::new() };
            for (cols, is_analog) in [(&electrical, false), (&analog, true)] {
                if cols.is_empty() {
                    continue;
                }
                let name = if is_analog { format!("ns{n}.analog{suffix}") } else { format!("ns{n}{suffix}") };
                if !options.wants(&name) {
                    continue;
                }
                let mut unknown = false;
                let channels: Vec<ChannelInfo> = cols
                    .iter()
                    .map(|&i| {
                        let c = &f.channels[i];
                        let scale = if f.spec == "2.1" { nev_scale(c.id).map(|g| (g, 0.0)) } else { c.scale() };
                        unknown |= scale.is_none();
                        let (gain, offset) = scale.unwrap_or((1.0, 0.0));
                        ChannelInfo { name: if c.label.is_empty() { format!("elec{}", c.id) } else { c.label.clone() }, gain, offset }
                    })
                    .collect();
                let (unit, calibration) = if unknown {
                    ("a.u.", Calibration::Unknown { note: if f.spec == "2.1" { "file spec 2.1 without a .nev: no digitization factors".into() } else { "unknown units".into() } })
                } else {
                    ("V", Calibration::Known)
                };
                let mut metadata = BTreeMap::new();
                metadata.insert("blackrock_nsx".to_string(), format!("ns{n}"));
                metadata.insert("blackrock_first_timestamp".to_string(), part.timestamp.to_string());
                if !f.label.is_empty() {
                    metadata.insert("blackrock_label".to_string(), f.label.clone());
                }
                let info = RecordingInfo {
                    name: name.clone(),
                    description: format!("{} of ns{n} ({}{})", if is_analog { "Analog inputs" } else { "Front-end channels" }, f.label, if f.ptp { ", PTP timestamps" } else { "" }).replace(" ()", ""),
                    channels,
                    samples: part.samples,
                    sample_rate: rate,
                    start_time: since(part.timestamp, f.timestamp_resolution),
                    unit: unit.into(),
                    calibration,
                    kind: if is_analog { SignalKind::Other } else { SignalKind::Electrical },
                    stored_as: SampleType::I16,
                    order: MemoryOrder::TimeMajor,
                    storage: format!("ns{n}"),
                    metadata,
                };
                if !is_analog {
                    for (k, &i) in cols.iter().enumerate() {
                        let c = &f.channels[i];
                        let link = ChannelRef { recording: name.clone(), channel: k };
                        match electrode_of.get(&c.id) {
                            Some(&e) => s.electrodes[e].channels.push(link),
                            None => {
                                let group = bank(c.connector);
                                if s.electrode_group(&group).is_none() {
                                    s.electrode_groups.push(ElectrodeGroup { name: group.clone(), description: format!("Electrodes on front-end {group}"), location: "unknown".into(), device: Some("Blackrock".into()) });
                                }
                                s.electrodes.push(Electrode { name: format!("elec{} ({})", c.id, c.label), group, channels: vec![link], ..Default::default() });
                                electrode_of.insert(c.id, s.electrodes.len() - 1);
                            }
                        }
                    }
                }
                s.recordings.push(Arc::new(NsxRecording::new(info, f, part, cols.clone())));
            }
        }
    }

    // NEV: spikes, digital / serial input, comments of this epoch
    if let Some(nev) = &opened.nev {
        let res = nev.timestamp_resolution as f64;
        let time = |ts: u64| since(ts, nev.timestamp_resolution);
        let packets: Vec<&nev::Packet> = nev.packets.iter().filter(|p| p.epoch == epoch).collect();
        // Spikes, one store per waveform length (normally one)
        let mut stores: BTreeMap<usize, SnippetSeries> = BTreeMap::new();
        let mut entries: BTreeMap<usize, Vec<(usize, u8, f64)>> = BTreeMap::new();
        let rate = opened.nsx.iter().map(|(_, f)| f.sample_rate()).fold(0.0, f64::max).max(30_000.0f64.min(res));
        for p in packets.iter().filter(|p| (1..=2048).contains(&p.id)) {
            let (bytes, width) = nev.waveform_shape(p.id);
            let scale = nev.waves.get(&p.id).map_or(1.0, |w| w.digitization as f64 * 1e-9);
            let sn = stores.entry(width).or_insert_with(|| SnippetSeries {
                name: "spikes".into(),
                description: "Spike waveforms detected online by the Blackrock NSP (channel = electrode id, sort code = unit class: 0 unsorted, 1–16 sorted, 255 noise)".into(),
                sample_rate: rate,
                samples_per_snippet: width,
                unit: "V".into(),
                ..Default::default()
            });
            sn.timestamps.push(time(p.timestamp));
            sn.channels.push(p.id);
            sn.sort_codes.push(nev.unit(p.offset) as u16);
            entries.entry(width).or_default().push((p.offset, bytes as u8, scale));
        }
        let several = stores.len() > 1;
        for (width, mut sn) in stores {
            if several {
                sn.name = format!("spikes_{width}");
            }
            if !options.wants(&sn.name) {
                continue;
            }
            // Spike channels on the electrodes of the same id (created when only the NEV has them)
            for c in sn.channel_set() {
                let e = *electrode_of.entry(c as u32).or_insert_with(|| {
                    let group = bank(nev.waves.get(&c).map_or(0, |w| w.connector));
                    if s.electrode_group(&group).is_none() {
                        s.electrode_groups.push(ElectrodeGroup { name: group.clone(), description: format!("Electrodes on front-end {group}"), location: "unknown".into(), device: Some("Blackrock".into()) });
                    }
                    let label = nev.labels.get(&c).cloned().unwrap_or_default();
                    s.electrodes.push(Electrode { name: format!("elec{c} ({label})"), group, channels: Vec::new(), ..Default::default() });
                    s.electrodes.len() - 1
                });
                sn.electrodes.insert(c, e);
            }
            sn.waveforms = Arc::new(NevWaveforms { nev: nev.clone(), spikes: entries.remove(&width).unwrap_or_default(), samples: width });
            s.snippets.push(sn);
        }

        // Digital and serial input words (2.3+: bit 0 = digital port, bit 7 = serial; 2.1 / 2.2:
        // reason 1 / 129)
        let new_flags = nev.spec.as_str() >= "2.3";
        let mut words: BTreeMap<&str, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
        for p in packets.iter().filter(|p| p.id == 0) {
            let (reason, word) = nev.digital(p.offset);
            let kind = if new_flags {
                match (reason & 1 == 1, reason & 0x80 != 0) {
                    (true, false) => Some("digital_input"),
                    (true, true) => Some("serial_input"),
                    _ => None,
                }
            } else {
                match reason {
                    1 => Some("digital_input"),
                    129 => Some("serial_input"),
                    _ => None,
                }
            };
            if let Some(k) = kind {
                let e = words.entry(k).or_default();
                e.0.push(time(p.timestamp));
                e.1.push(word as f64);
            }
        }
        for (name, (onsets, values)) in words {
            if options.wants(name) {
                let description = if name == "digital_input" { "Digital input port words (value = the 16-bit word)" } else { "Serial input port words" };
                s.events.push(EventSeries { name: name.into(), description: description.into(), onsets, offsets: None, values, channels: 1, labels: Vec::new() });
            }
        }
        // Comments
        let comments: Vec<(f64, String)> = packets.iter().filter(|p| p.id == 0xFFFF).map(|p| (time(p.timestamp), nev.comment_text(p.offset))).collect();
        if !comments.is_empty() && options.wants("comments") {
            let n = comments.len();
            s.events.push(EventSeries {
                name: "comments".into(),
                description: "Comments entered during the recording".into(),
                onsets: comments.iter().map(|c| c.0).collect(),
                offsets: None,
                values: vec![0.0; n],
                channels: 1,
                labels: comments.into_iter().map(|c| c.1).collect(),
            });
        }
        if opened.epochs() > 1 {
            warnings.push(format!("the clock was reset during the recording: this is segment {} of {}", epoch + 1, opened.epochs()));
        }
    }
    for e in &mut s.events {
        let mut order: Vec<usize> = (0..e.onsets.len()).collect();
        order.sort_by(|&a, &b| e.onsets[a].total_cmp(&e.onsets[b]));
        if order.windows(2).any(|w| w[0] > w[1]) {
            e.onsets = order.iter().map(|&i| e.onsets[i]).collect();
            e.values = order.iter().map(|&i| e.values[i]).collect();
            if !e.labels.is_empty() {
                e.labels = order.iter().map(|&i| e.labels[i].clone()).collect();
            }
        }
    }

    s.metadata.devices.push(Device {
        name: "Blackrock".into(),
        description: opened.nev.as_ref().map_or_else(|| "Blackrock Microsystems neural signal processor".into(), |n| format!("Blackrock Microsystems neural signal processor ({})", n.application)),
        manufacturer: Some("Blackrock Microsystems".into()),
        model: None,
    });
    let m = &mut s.metadata;
    let origin = opened.nsx.iter().find_map(|(_, f)| f.origin).or(opened.nev.as_ref().map(|n| n.origin));
    m.start_time = origin.as_ref().and_then(origin_iso);
    let base = set.base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    m.experiment = Some(base);
    if let Some(c) = opened.nev.as_ref().map(|n| n.comment.clone()).filter(|c| !c.is_empty()) {
        m.notes.push(c);
    }
    let spec = opened.nev.as_ref().map(|n| n.spec.clone()).or(opened.nsx.first().map(|(_, f)| f.spec.clone())).unwrap_or_default();
    m.extra.insert("blackrock_file_spec".into(), spec.clone());
    let mut prov = Provenance::new("blackrock");
    prov.version = Some(format!("file spec {spec}{}", if opened.nsx.iter().any(|(_, f)| f.ptp) { " (PTP)" } else { "" }));
    for f in &files {
        prov.add_file(f);
    }
    prov.warnings = warnings;
    s.provenance = prov;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir(name: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(out: &mut Vec<u8>, at: usize, bytes: &[u8]) {
        if out.len() < at + bytes.len() {
            out.resize(at + bytes.len(), 0);
        }
        out[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// NSx 2.3, 1 kHz: an electrode (id 1, bank A, ±8191 µV over ±32764) and an analog input (id
    /// 129, ±5000 mV); blocks at the given timestamps (30 kHz ticks) with `n` samples each.
    fn nsx(blocks: &[(u32, u32)]) -> Vec<u8> {
        let mut b = Vec::new();
        put(&mut b, 0, b"NEURALCD");
        put(&mut b, 8, &[2, 3]);
        put(&mut b, 10, &(314u32 + 2 * 66).to_le_bytes());
        put(&mut b, 14, b"1 kS/s");
        put(&mut b, 286, &30u32.to_le_bytes());
        put(&mut b, 290, &30_000u32.to_le_bytes());
        for (k, v) in [2024u16, 3, 0, 15, 12, 30, 5, 250].iter().enumerate() {
            put(&mut b, 294 + 2 * k, &v.to_le_bytes());
        }
        put(&mut b, 310, &2u32.to_le_bytes());
        for (i, (id, label, units, max_a)) in [(1u16, "elec1", "uV", 8191i16), (129, "ainp1", "mV", 5000)].iter().enumerate() {
            let c = 314 + 66 * i;
            put(&mut b, c, b"CC");
            put(&mut b, c + 2, &id.to_le_bytes());
            put(&mut b, c + 4, label.as_bytes());
            put(&mut b, c + 20, &[1, (i + 1) as u8]);
            put(&mut b, c + 22, &(-32764i16).to_le_bytes());
            put(&mut b, c + 24, &32764i16.to_le_bytes());
            put(&mut b, c + 26, &(-max_a).to_le_bytes());
            put(&mut b, c + 28, &max_a.to_le_bytes());
            put(&mut b, c + 30, units.as_bytes());
        }
        b.resize(314 + 2 * 66, 0);
        for &(ts, n) in blocks {
            b.push(1);
            b.extend(ts.to_le_bytes());
            b.extend(n.to_le_bytes());
            for t in 0..n {
                b.extend((t as i16 * 10).to_le_bytes());
                b.extend((-(t as i16)).to_le_bytes());
            }
        }
        b
    }

    /// NEV 2.3: 30 kHz timestamps, 40-byte packets (4-sample int16 waveforms), electrode 1 at
    /// 250 nV per bit; packets (timestamp, id, payload).
    fn nev(packets: &[(u32, u16, Vec<u8>)]) -> Vec<u8> {
        let mut b = Vec::new();
        put(&mut b, 0, b"NEURALEV");
        put(&mut b, 8, &[2, 3]);
        put(&mut b, 12, &(336u32 + 32).to_le_bytes());
        put(&mut b, 16, &40u32.to_le_bytes());
        put(&mut b, 20, &30_000u32.to_le_bytes());
        put(&mut b, 44, b"Central");
        put(&mut b, 332, &1u32.to_le_bytes());
        put(&mut b, 336, b"NEUEVWAV");
        put(&mut b, 344, &1u16.to_le_bytes());
        put(&mut b, 346, &[1, 1]);
        put(&mut b, 348, &250u16.to_le_bytes());
        put(&mut b, 357, &[2]);
        put(&mut b, 358, &4u16.to_le_bytes());
        b.resize(336 + 32, 0);
        for (ts, id, payload) in packets {
            let at = b.len();
            put(&mut b, at, &ts.to_le_bytes());
            put(&mut b, at + 4, &id.to_le_bytes());
            put(&mut b, at + 6, payload);
            b.resize(at + 40, 0);
        }
        b
    }

    #[test]
    fn test_pause_reset_spikes_events() {
        let dir = fixture_dir("blackrock-fixture");
        // Block 1 at 1 s (30 000 ticks), 10 samples; a pause; block 2 at 2 s; then a clock reset
        std::fs::write(dir.join("rec.ns2"), nsx(&[(30_000, 10), (60_000, 10), (300, 5)])).unwrap();
        let spike = |unit: u8| [vec![unit, 0], [100i16, -100, 40, 0].iter().flat_map(|v| v.to_le_bytes()).collect()].concat();
        let comment = |text: &str| [vec![0u8, 0, 0, 0, 0, 0], text.as_bytes().to_vec()].concat();
        std::fs::write(
            dir.join("rec.nev"),
            nev(&[
                (30_300, 1, spike(1)),
                (30_600, 0, vec![1, 0, 5, 0]),
                (60_150, 1, spike(255)),
                (61_000, 0xFFFF, comment("hello")),
                (0, 0xFFFF, comment("critical load restart")),
                (450, 1, spike(2)),
            ]),
        )
        .unwrap();

        assert_eq!(Blackrock.containers(&dir), vec!["segment1", "segment2"]);
        let s = nc_core::testkit::check_reader(&Blackrock, &dir, &OpenOptions { block: Some("segment1".into()), ..Default::default() });
        let names: Vec<&str> = s.recordings.iter().map(|r| r.info().name.as_str()).collect();
        assert_eq!(names, vec!["ns2.p1", "ns2.analog.p1", "ns2.p2", "ns2.analog.p2"]);
        let p2 = s.recording("ns2.p2").unwrap().info();
        assert_eq!((p2.start_time, p2.sample_rate, p2.samples), (1.0, 1000.0, 10));
        let mut v = vec![0.0; 2];
        s.recording("ns2.p1").unwrap().read(&[0], 3..4, &mut v[..1]).unwrap();
        s.recording("ns2.analog.p1").unwrap().read(&[0], 3..4, &mut v[1..]).unwrap();
        let (g_uv, g_mv) = (2.0 * 8191.0 / 65528.0 * 1e-6, 2.0 * 5000.0 / 65528.0 * 1e-3);
        assert!((v[0] as f64 - 30.0 * g_uv).abs() < 1e-12 && (v[1] as f64 + 3.0 * g_mv).abs() < 1e-9, "{v:?}");
        assert_eq!(s.electrodes.len(), 1);
        assert_eq!((s.electrodes[0].group.as_str(), s.electrodes[0].channels.len()), ("bank A", 2), "one electrode for both parts");

        let sn = &s.snippets[0];
        assert_eq!((sn.len(), sn.channels.clone(), sn.sort_codes.clone(), sn.samples_per_snippet), (2, vec![1, 1], vec![1, 255], 4));
        assert_eq!(sn.timestamps, vec![0.01, 1.005]);
        assert_eq!(sn.read(&[0]).unwrap(), vec![(100.0 * 250e-9) as f32, (-100.0 * 250e-9) as f32, (40.0 * 250e-9) as f32, 0.0]);
        assert_eq!(sn.electrodes[&1], 0);
        let dig = s.event_series("digital_input").unwrap();
        assert_eq!((dig.onsets.clone(), dig.values.clone()), (vec![0.02], vec![5.0]));
        assert_eq!(s.event_series("comments").unwrap().labels, vec!["hello"]);
        assert_eq!(s.metadata.start_time.as_deref(), Some("2024-03-15T12:30:05.250Z"));

        let second = Blackrock.open(&dir, &OpenOptions { block: Some("segment2".into()), ..Default::default() }).unwrap();
        assert_eq!(second.recordings[0].info().samples, 5);
        assert_eq!(second.snippets[0].timestamps, vec![0.005]);
        assert!(second.provenance.warnings.iter().any(|w| w.contains("segment 2 of 2")));
    }
}
