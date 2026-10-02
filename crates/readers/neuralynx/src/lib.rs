//! nc-neuralynx: Neuralynx recordings (Cheetah, Pegasus, BML, Neuraview): one file per entity in
//! a session folder.
//!
//! - `.ncs` continuous channels: grouped into streams as neo does (same stated rate, input range
//!   and DSP filters), named `ncs_<rate>Hz` (`_2`, … when several share a rate). Gaps split a
//!   stream into parts `….p1`, `….p2`, … each at its own start. int16 with each channel's
//!   `ADBitVolts` (negative for inverted inputs) as gain; electrical, one electrode per
//!   AD channel.
//! - `.nse` / `.nst` / `.ntt` spikes (single electrode, stereotrode, tetrode): one snippet store
//!   per file (its entity name), one snippet per wire of each spike (channel = AD channel), sort
//!   code = cell number; waveforms in volts.
//! - `.nev` events: one series per (event id, TTL value) with the event strings as labels (neo's
//!   event channels).
//!
//! Times: µs timestamps, seconds from the earliest timestamp of the folder. Start time: the
//! earliest `Time Opened` / `TimeCreated` (local time of the recording computer). `.nvt` / `.nrd`
//! are not read. A folder holding several sessions in sub-folders lists them as containers.

/// This crate's version (`nc-neuralynx`, from its `Cargo.toml`): recorded in every conversion's
/// provenance and report, so a problem in a file can be traced to the code that wrote it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// [`VERSION`].
pub fn version() -> &'static str {
    VERSION
}

pub mod header;
pub mod ncs;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use header::Header;
use nc_base::mapped::MappedFile;
use nc_core::{
    Calibration, ChannelInfo, ChannelRef, Detection, Device, Electrode, ElectrodeGroup, Error, EventSeries, MemoryOrder, OpenOptions, Provenance,
    Reader, RecordingInfo, Result, SampleType, Session, SignalKind, SnippetSeries, Waveforms,
};
use ncs::{NcsFile, NcsRecording};

pub struct Neuralynx;

const DATA: [&str; 5] = ["ncs", "nse", "nst", "ntt", "nev"];

fn ext(p: &Path) -> Option<String> {
    p.extension().map(|e| e.to_string_lossy().to_lowercase())
}

fn is_nlx(p: &Path) -> bool {
    use std::io::Read;
    if !ext(p).is_some_and(|e| DATA.contains(&e.as_str())) {
        return false;
    }
    // The header's first line (`######## Neuralynx Data File Header`) may be missing; its
    // properties are not
    let mut b = [0u8; 1024];
    let Ok(n) = std::fs::File::open(p).and_then(|mut f| f.read(&mut b)) else { return false };
    let text = String::from_utf8_lossy(&b[..n]);
    b.starts_with(b"########") || text.contains("Neuralynx") || text.contains("-FileType") || text.contains("-RecordSize")
}

/// Folders holding Neuralynx files at or below `path` (two levels), sorted.
fn folders(path: &Path) -> Vec<PathBuf> {
    fn has(dir: &Path) -> bool {
        std::fs::read_dir(dir).is_ok_and(|rd| rd.flatten().any(|e| is_nlx(&e.path())))
    }
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if has(dir) {
            out.push(dir.to_path_buf());
            return;
        }
        if depth == 0 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut subs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        subs.sort();
        for s in subs {
            walk(&s, depth - 1, out);
        }
    }
    let start = if path.is_file() { path.parent().unwrap_or(path) } else { path };
    let mut out = Vec::new();
    walk(start, 2, &mut out);
    out
}

fn container_names(path: &Path) -> Vec<(String, PathBuf)> {
    let root = if path.is_file() { path.parent().unwrap_or(path) } else { path };
    folders(path)
        .into_iter()
        .map(|d| {
            let rel = d.strip_prefix(root).ok().map(|r| r.to_string_lossy().into_owned()).filter(|r| !r.is_empty());
            (rel.unwrap_or_else(|| d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), d)
        })
        .collect()
}

impl Reader for Neuralynx {
    fn name(&self) -> &'static str {
        "neuralynx"
    }
    fn version(&self) -> &'static str {
        crate::VERSION
    }

    /// Compared with its reference reader on real data (docs/formats).
    fn maturity(&self) -> nc_core::Maturity {
        nc_core::Maturity::Verified
    }

    fn description(&self) -> &'static str {
        "Neuralynx session folder (Cheetah / Pegasus / BML / Neuraview): .ncs, .nse / .nst / .ntt, .nev"
    }

    fn opens(&self) -> &'static str {
        "A Neuralynx session folder, or any file in it (the whole folder opens)"
    }

    fn versions(&self) -> &'static [&'static str] {
        &[
            "Cheetah 1.x – 6.x, Pegasus 2.x, BML, Neuraview: 16 KiB text headers",
            "CSC channels in streams (rate / input range / filters), gaps as parts, measured sample rate (as neo); spikes; events",
        ]
    }

    fn detect(&self, path: &Path) -> Option<Detection> {
        let dirs = folders(path);
        let first = dirs.first()?;
        let any = std::fs::read_dir(first).ok()?.flatten().map(|e| e.path()).find(|p| is_nlx(p))?;
        let bytes = std::fs::read(&any).ok()?;
        let h = Header::parse(&bytes[..bytes.len().min(header::SIZE)]);
        let version = if dirs.len() > 1 { format!("{}, {} sessions", h.application(), dirs.len()) } else { h.application() };
        Some(Detection { format: "neuralynx", version: Some(version), confidence: 0.9 })
    }

    fn containers(&self, path: &Path) -> Vec<String> {
        let all = container_names(path);
        if all.len() > 1 { all.into_iter().map(|(n, _)| n).collect() } else { Vec::new() }
    }

    fn open(&self, path: &Path, options: &OpenOptions) -> Result<Session> {
        let all = container_names(path);
        let dir = match (&options.block, all.len()) {
            (Some(name), _) => all.iter().find(|(n, _)| n == name).map(|(_, d)| d.clone()).ok_or_else(|| Error::Unsupported(format!("{name}: no such session in {}", path.display())))?,
            (None, 1) => all[0].1.clone(),
            (None, 0) => return Err(Error::format("neuralynx", format!("no Neuralynx files in {}", path.display()))),
            (None, n) => {
                return Err(Error::Unsupported(format!(
                    "{} holds {n} Neuralynx sessions; choose one with --block <name>: {}",
                    path.display(),
                    all.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")
                )))
            }
        };
        open_folder(&dir, options)
    }
}

/// Spike waveforms of a `.nse` / `.nst` / `.ntt` file: snippet = (record, wire).
struct SpikeFile {
    file: Arc<MappedFile>,
    record: usize,
    wires: usize,
    samples: usize,
    /// Volts per bit per wire.
    gains: Vec<f64>,
    snippets: Vec<(u32, u8)>,
}

impl std::fmt::Debug for SpikeFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpikeFile").field("wires", &self.wires).field("samples", &self.samples).field("snippets", &self.snippets.len()).finish()
    }
}

impl Waveforms for SpikeFile {
    fn count(&self) -> usize {
        self.snippets.len()
    }

    fn read(&self, snippets: &[usize], out: &mut [f32]) -> Result<()> {
        let b = self.file.bytes();
        for (o, &i) in out.chunks_exact_mut(self.samples.max(1)).zip(snippets) {
            let &(r, w) = self.snippets.get(i).ok_or_else(|| Error::format("neuralynx", format!("spike {i} out of range")))?;
            // Samples are time-major: [sample][wire]
            let base = header::SIZE + r as usize * self.record + 48;
            for (k, v) in o.iter_mut().enumerate() {
                let at = base + (k * self.wires + w as usize) * 2;
                *v = (i16::from_le_bytes([b[at], b[at + 1]]) as f64 * self.gains[w as usize]) as f32;
            }
        }
        Ok(())
    }
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn i16_at(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([b[at], b[at + 1]])
}

fn open_folder(dir: &Path, options: &OpenOptions) -> Result<Session> {
    let mut s = Session::default();
    let mut warnings = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?.flatten().map(|e| e.path()).filter(|p| is_nlx(p)).collect();
    files.sort();
    let skipped: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| matches!(ext(p).as_deref(), Some("nvt" | "nrd"))).filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect())
        .unwrap_or_default();
    if !skipped.is_empty() {
        warnings.push(format!("not read (video tracking / raw data): {}", skipped.join(", ")));
    }

    // CSC files by stream key (neo); files with only a header are skipped
    let mut csc: BTreeMap<String, Vec<(PathBuf, NcsFile)>> = BTreeMap::new();
    let mut apps = Vec::new();
    let mut opened_at: Vec<String> = Vec::new();
    for p in files.iter().filter(|p| ext(p).as_deref() == Some("ncs")) {
        let f = NcsFile::open(p)?;
        apps.push(f.header.application());
        opened_at.extend(f.header.opened.clone());
        if f.records == 0 {
            warnings.push(format!("{}: no records", p.display()));
            continue;
        }
        csc.entry(f.header.stream_key()).or_default().push((p.clone(), f));
    }

    // Sections per stream: from its first file; other files must match
    struct Stream {
        name: String,
        files: Vec<(PathBuf, NcsFile)>,
        sections: Vec<ncs::Section>,
        rate: f64,
    }
    let mut streams: Vec<Stream> = Vec::new();
    let mut keys: Vec<(f64, String)> = csc.keys().map(|k| (csc[k][0].1.header.sample_rate().unwrap_or(0.0), k.clone())).collect();
    keys.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut per_rate: BTreeMap<i64, usize> = BTreeMap::new();
    for (rate_stated, key) in keys {
        let mut list = csc.remove(&key).unwrap_or_default();
        list.sort_by_key(|(_, f)| f.header.channel_ids().first().copied().unwrap_or(i64::MAX));
        let (sections, rate) = list[0].1.sections();
        let mut kept = vec![list.remove(0)];
        for (p, f) in list {
            if f.records != kept[0].1.records || f.sections().0 != sections {
                warnings.push(format!("{}: its records do not line up with {} (same stream); skipped", p.display(), kept[0].0.display()));
                continue;
            }
            kept.push((p, f));
        }
        let n = per_rate.entry(rate_stated.round() as i64).or_default();
        *n += 1;
        let name = if *n == 1 { format!("ncs_{}Hz", rate_stated.round() as i64) } else { format!("ncs_{}Hz_{}", rate_stated.round() as i64, n) };
        streams.push(Stream { name, files: kept, sections, rate });
    }

    // Spike files and event files
    struct Spikes {
        path: PathBuf,
        file: Arc<MappedFile>,
        header: Header,
    }
    let mut spikes = Vec::new();
    let mut events = Vec::new();
    for p in &files {
        match ext(p).as_deref() {
            Some("nse" | "nst" | "ntt") => {
                let file = Arc::new(MappedFile::open(p)?);
                let header = Header::parse(file.bytes());
                opened_at.extend(header.opened.clone());
                apps.push(header.application());
                spikes.push(Spikes { path: p.clone(), file, header });
            }
            Some("nev") => {
                let file = Arc::new(MappedFile::open(p)?);
                let header = Header::parse(file.bytes());
                opened_at.extend(header.opened.clone());
                apps.push(header.application());
                events.push((p.clone(), file, header));
            }
            _ => {}
        }
    }

    // Time zero: the earliest timestamp anywhere (µs)
    let mut t0 = u64::MAX;
    for st in &streams {
        if let Some(sec) = st.sections.first() {
            t0 = t0.min(sec.start);
        }
    }
    let spike_record = |h: &Header, wires: usize| h.get("RecordSize").and_then(|v| v.parse::<usize>().ok()).unwrap_or(48 + 2 * wires * h.get("WaveformLength").and_then(|v| v.parse().ok()).unwrap_or(32));
    let spike_wires = |h: &Header, p: &Path| h.get("NumADChannels").and_then(|v| v.parse::<usize>().ok()).unwrap_or(match ext(p).as_deref() {
        Some("ntt") => 4,
        Some("nst") => 2,
        _ => 1,
    });
    for sp in &spikes {
        let wires = spike_wires(&sp.header, &sp.path);
        let rec = spike_record(&sp.header, wires);
        if sp.file.bytes().len() >= header::SIZE + rec {
            t0 = t0.min(u64_at(sp.file.bytes(), header::SIZE));
        }
    }
    for (_, file, _) in &events {
        let b = file.bytes();
        let mut at = header::SIZE;
        while at + 184 <= b.len() {
            t0 = t0.min(u64_at(b, at + 6));
            at += 184;
        }
    }
    if t0 == u64::MAX {
        t0 = 0;
    }
    let seconds = |us: u64| (us as i128 - t0 as i128) as f64 / 1e6;

    // Recordings and electrodes (one per AD channel)
    let mut electrode_of: BTreeMap<i64, usize> = BTreeMap::new();
    let group = "Neuralynx".to_string();
    let mut electrode = |s: &mut Session, ad: i64, label: &str| -> usize {
        *electrode_of.entry(ad).or_insert_with(|| {
            if s.electrode_group(&group).is_none() {
                s.electrode_groups.push(ElectrodeGroup { name: group.clone(), description: "Neuralynx AD channels".into(), location: "unknown".into(), device: Some("Neuralynx".into()) });
            }
            s.electrodes.push(Electrode { name: label.to_string(), group: group.clone(), channels: Vec::new(), ..Default::default() });
            s.electrodes.len() - 1
        })
    };
    for st in &streams {
        let several = st.sections.len() > 1;
        if several {
            warnings.push(format!("{}: {} gaps in the records (recording paused?): one part per gap-free section", st.name, st.sections.len() - 1));
        }
        for (k, sec) in st.sections.iter().enumerate() {
            let name = if several { format!("{}.p{}", st.name, k + 1) } else { st.name.clone() };
            if !options.wants(&name) {
                continue;
            }
            let mut unknown = false;
            let channels: Vec<ChannelInfo> = st
                .files
                .iter()
                .map(|(p, f)| {
                    let gain = f.header.volts_per_bit(1)[0];
                    unknown |= gain.is_none();
                    let label = f.header.name().map(str::to_string).unwrap_or_else(|| p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    ChannelInfo { name: label, gain: gain.unwrap_or(1.0), offset: 0.0 }
                })
                .collect();
            for (c, (_, f)) in st.files.iter().enumerate() {
                let ad = f.header.channel_ids().first().copied().unwrap_or(-(c as i64) - 1);
                let e = electrode(&mut s, ad, &channels[c].name);
                s.electrodes[e].channels.push(ChannelRef { recording: name.clone(), channel: c });
            }
            let h = &st.files[0].1.header;
            let mut metadata = BTreeMap::new();
            metadata.insert("neuralynx_first_timestamp_us".to_string(), sec.start.to_string());
            metadata.insert("neuralynx_stated_rate".to_string(), h.get("SamplingFrequency").unwrap_or("").to_string());
            for key in ["InputRange", "DspLowCutFrequency", "DspHighCutFrequency", "ReferenceChannel"] {
                if let Some(v) = h.get(key) {
                    metadata.insert(format!("neuralynx_{key}"), v.to_string());
                }
            }
            let info = RecordingInfo {
                name: name.clone(),
                description: format!(
                    "Neuralynx CSC channels, {} Hz stated ({:.4} Hz measured), filters {}–{} Hz",
                    h.get("SamplingFrequency").unwrap_or("?"),
                    st.rate,
                    h.get("DspLowCutFrequency").unwrap_or("?"),
                    h.get("DspHighCutFrequency").unwrap_or("?")
                ),
                channels,
                samples: sec.samples,
                sample_rate: st.rate,
                start_time: seconds(sec.start),
                unit: if unknown { "a.u." } else { "V" }.into(),
                calibration: if unknown { Calibration::Unknown { note: "no ADBitVolts in the header".into() } } else { Calibration::Known },
                kind: SignalKind::Electrical,
                stored_as: SampleType::I16,
                order: MemoryOrder::ChannelMajor,
                storage: "ncs".into(),
                metadata,
            };
            let mapped = st.files.iter().map(|(_, f)| f.file.clone()).collect();
            s.recordings.push(Arc::new(NcsRecording::new(info, mapped, &st.files[0].1, *sec)));
        }
    }

    // Spikes: one store per file, one snippet per wire
    for sp in spikes {
        let wires = spike_wires(&sp.header, &sp.path);
        let record = spike_record(&sp.header, wires);
        let samples = (record.saturating_sub(48)) / (2 * wires.max(1));
        let name = sp.header.name().map(str::to_string).unwrap_or_else(|| sp.path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        if !options.wants(&name) || samples == 0 {
            continue;
        }
        let b = sp.file.bytes();
        let count = b.len().saturating_sub(header::SIZE) / record;
        let gains: Vec<f64> = sp.header.volts_per_bit(wires).into_iter().map(|g| g.unwrap_or(1.0)).collect();
        let ads = sp.header.channel_ids();
        let rate = sp.header.sample_rate().unwrap_or(32_000.0);
        let mut sn = SnippetSeries {
            name: name.clone(),
            description: format!("Neuralynx spikes of {name} ({wires} wire{}; sort code = cell number)", if wires == 1 { "" } else { "s" }),
            sample_rate: rate,
            samples_per_snippet: samples,
            unit: "V".into(),
            ..Default::default()
        };
        let mut index = Vec::with_capacity(count * wires);
        for r in 0..count {
            let at = header::SIZE + r * record;
            let t = seconds(u64_at(b, at));
            let cell = u32_at(b, at + 12) as u16;
            for w in 0..wires {
                sn.timestamps.push(t);
                sn.channels.push(ads.get(w).copied().unwrap_or(w as i64) as u16);
                sn.sort_codes.push(cell);
                index.push((r as u32, w as u8));
            }
        }
        for (w, &ad) in ads.iter().enumerate().take(wires) {
            let e = electrode(&mut s, ad, &format!("{name} wire {w}"));
            sn.electrodes.insert(ad as u16, e);
        }
        sn.waveforms = Arc::new(SpikeFile { file: sp.file.clone(), record, wires, samples, gains, snippets: index });
        s.snippets.push(sn);
    }

    // Events: one series per (event id, TTL value)
    for (p, file, h) in &events {
        let b = file.bytes();
        let entity = h.name().unwrap_or("Events").to_string();
        let mut series: BTreeMap<(i16, i16), (Vec<f64>, Vec<String>)> = BTreeMap::new();
        let mut at = header::SIZE;
        while at + 184 <= b.len() {
            let (ts, id, ttl) = (u64_at(b, at + 6), i16_at(b, at + 14), i16_at(b, at + 16));
            let text = &b[at + 56..at + 184];
            let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
            let label: String = text[..end].iter().map(|&c| c as char).collect::<String>().trim().to_string();
            let e = series.entry((id, ttl)).or_default();
            e.0.push(seconds(ts));
            e.1.push(label);
            at += 184;
        }
        for ((id, ttl), (onsets, labels)) in series {
            let name = format!("{entity} id{id} ttl{ttl}");
            if !options.wants(&name) {
                continue;
            }
            let n = onsets.len();
            let mut order: Vec<usize> = (0..n).collect();
            order.sort_by(|&a, &b| onsets[a].total_cmp(&onsets[b]));
            s.events.push(EventSeries {
                name,
                description: format!("Neuralynx events of {} with event id {id} and TTL value {ttl}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                onsets: order.iter().map(|&i| onsets[i]).collect(),
                offsets: None,
                values: vec![ttl as f64; n],
                channels: 1,
                labels: order.iter().map(|&i| labels[i].clone()).collect(),
            });
        }
    }

    apps.dedup();
    s.metadata.devices.push(Device { name: "Neuralynx".into(), description: format!("Neuralynx acquisition ({})", apps.first().cloned().unwrap_or_default()), manufacturer: Some("Neuralynx".into()), model: None });
    opened_at.sort();
    let m = &mut s.metadata;
    m.start_time = opened_at.first().cloned();
    m.experiment = dir.file_name().map(|n| n.to_string_lossy().into_owned());
    m.extra.insert("neuralynx_first_timestamp_us".into(), t0.to_string());
    let mut prov = Provenance::new("neuralynx");
    prov.version = apps.first().cloned();
    for p in &files {
        prov.add_file(p);
    }
    prov.warnings = warnings;
    s.provenance = prov;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(lines: &str) -> Vec<u8> {
        let mut h = format!("######## Neuralynx Data File Header\n{lines}").into_bytes();
        h.resize(header::SIZE, 0);
        h
    }

    /// A CSC file at 1 kHz (Digital Lynx: rate measured) with records at the given µs times;
    /// sample k of a record = record index × 1000 + k.
    fn csc(name: &str, ad: u32, starts: &[u64]) -> Vec<u8> {
        let mut b = head(&format!(
            "## Time Opened (m/d/y): 11/28/2016  (h:m:s.ms) 21:50:33.322\n-HardwareSubSystemType DigitalLynxSX\n-SamplingFrequency 1000\n\
-ADBitVolts 0.000001\n-AcqEntName {name}\n-ADChannel {ad}\n-InputRange 1000\n-InputInverted True\n-DspLowCutFrequency 0.1\n"
        ));
        for (r, &t) in starts.iter().enumerate() {
            b.extend(t.to_le_bytes());
            b.extend(ad.to_le_bytes());
            b.extend(1000u32.to_le_bytes());
            b.extend(512u32.to_le_bytes());
            for k in 0..512 {
                b.extend(((r * 1000 + k) as i16).to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn test_session_folder() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/neuralynx-fixture");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Two records back to back (512 ms each), then a gap of 1 s
        let starts = [1_000_000, 1_512_000, 3_024_000];
        std::fs::write(dir.join("CSC1.ncs"), csc("CSC1", 7, &starts)).unwrap();
        std::fs::write(dir.join("CSC2.ncs"), csc("CSC2", 8, &starts)).unwrap();
        // Tetrode, wires listed out of order; 2 samples per wire; one spike at 1.1 s, cell 3
        let mut tt = head("-SamplingFrequency 32000\n-ADBitVolts 0.000002\n-AcqEntName TT1\n-NumADChannels 4\n-ADChannel 9 8 7 6\n-RecordSize 64\n-WaveformLength 2\n");
        tt.extend(1_100_000u64.to_le_bytes());
        tt.extend(0u32.to_le_bytes());
        tt.extend(3u32.to_le_bytes());
        tt.extend([0u8; 32]);
        for v in [10i16, 20, 30, 40, 11, 21, 31, 41] {
            tt.extend(v.to_le_bytes());
        }
        std::fs::write(dir.join("TT1.ntt"), tt).unwrap();
        // Events: TTL 2 at 1.2 s and 0 at 1.3 s (event id 11)
        let mut ev = head("-AcqEntName Events\n");
        for (t, ttl, text) in [(1_200_000u64, 2i16, "TTL on"), (1_300_000, 0, "TTL off"), (900_000, 0, "Starting Recording")] {
            let mut r = vec![0u8; 184];
            r[6..14].copy_from_slice(&t.to_le_bytes());
            r[14..16].copy_from_slice(&(if text.starts_with("TTL") { 11i16 } else { 19 }).to_le_bytes());
            r[16..18].copy_from_slice(&ttl.to_le_bytes());
            r[56..56 + text.len()].copy_from_slice(text.as_bytes());
            ev.extend(r);
        }
        std::fs::write(dir.join("Events.nev"), ev).unwrap();
        std::fs::write(dir.join("VT1.nvt"), head("")).unwrap();

        let s = nc_core::testkit::check_reader(&Neuralynx, &dir, &OpenOptions::default());
        let names: Vec<&str> = s.recordings.iter().map(|r| r.info().name.as_str()).collect();
        assert_eq!(names, vec!["ncs_1000Hz.p1", "ncs_1000Hz.p2"]);
        let p1 = s.recording("ncs_1000Hz.p1").unwrap().info();
        // t0 = the "Starting Recording" event at 0.9 s
        assert_eq!((p1.samples, p1.start_time, p1.sample_rate, p1.channels[1].gain), (1024, 0.1, 1000.0, -0.000001));
        assert_eq!(s.recording("ncs_1000Hz.p2").unwrap().info().start_time, 2.124);
        let mut v = vec![0.0; 2];
        s.recording("ncs_1000Hz.p1").unwrap().read(&[0], 511..513, &mut v).unwrap();
        assert_eq!(v, vec![(511.0 * -1e-6) as f32, (1000.0 * -1e-6) as f32], "across the record boundary");
        assert_eq!(s.electrodes.len(), 4, "AD channels 6–9: the tetrode shares 7 and 8 with the CSC files");

        let sn = &s.snippets[0];
        assert_eq!((sn.name.as_str(), sn.channels.clone(), sn.sort_codes.clone(), sn.samples_per_snippet), ("TT1", vec![9, 8, 7, 6], vec![3; 4], 2));
        assert!((sn.timestamps[0] - 0.2).abs() < 1e-12);
        // Wire 1 (AD 8): samples 20, 21 (time-major in the file)
        assert_eq!(sn.read(&[1]).unwrap(), vec![(20.0 * 2e-6) as f32, (21.0 * 2e-6) as f32]);

        let on = s.event_series("Events id11 ttl2").unwrap();
        assert_eq!((on.onsets.clone(), on.labels.clone()), (vec![0.3], vec!["TTL on".to_string()]));
        assert_eq!(s.event_series("Events id19 ttl0").unwrap().onsets, vec![0.0]);
        assert_eq!(s.metadata.start_time.as_deref(), Some("2016-11-28T21:50:33.322"));
        assert!(s.provenance.warnings.iter().any(|w| w.contains("VT1.nvt")));
    }
}
