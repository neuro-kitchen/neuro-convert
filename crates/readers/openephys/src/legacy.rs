//! The legacy Open Ephys format (GUI before 0.4.4 and later as an option): one file per channel.
//!
//! Every file starts with a 1024-byte text header (`header.sampleRate = 30000;`, `bitVolts`, …).
//! - `<proc>_[<source>_]<CH|AUX|ADC><n>[_<start>].continuous`: records of an int64 first-sample
//!   number, uint16 sample count (1024), uint16 recording number, 1024 **big-endian** int16
//!   samples and a 10-byte marker (2070 bytes). `CH` channels are in µV per bit, `AUX` / `ADC` in
//!   volts per bit.
//! - `all_channels[_<start>].events`: 16-byte records (int64 sample number, int16 position,
//!   uint8 type, processor, id (1 rising / 0 falling), channel, uint16 recording number); type 3
//!   is a TTL change.
//! - `messages[_<start>].events`: text lines `<sample number> <message>`.
//! - `<electrode>[_<start>].spikes`: records of uint8 type, int64 sample number, int64 software
//!   time, uint16 source, channels (n), samples (m), sorted id, electrode id, channel, 3 colour
//!   bytes, 2 float32 projections, uint16 sample rate, n × m uint16 samples (offset 32768,
//!   channel-major), n float32 gains, n uint16 thresholds, uint16 recording number. Volts =
//!   (raw − 32768) / gain / 1000 (as neo).
//!
//! Each acquisition start (`_2`, `_3`, … suffix; `settings_<n>.xml`) is one container
//! (`experiment<n>`), like neo's segments. Gaps in the record numbers read as zeros (as neo).

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nc_base::mapped::MappedFile;
use nc_core::{
    check_read, Calibration, ChannelInfo, Electrode, ElectrodeGroup, Error, EventSeries, MemoryOrder, OpenOptions, Provenance, Recording, RecordingInfo,
    Result, SampleType, Session, SignalKind, SnippetSeries, Waveforms, Device,
};

const HEADER: usize = 1024;
const BLOCK: usize = 1024;
const RECORD: usize = 8 + 2 + 2 + BLOCK * 2 + 10;

/// `header.key = value;` pairs of a 1024-byte header (quotes removed).
pub fn header(bytes: &[u8]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(HEADER)]);
    text.split(';')
        .filter_map(|part| {
            let (k, v) = part.split_once('=')?;
            let k = k.trim().strip_prefix("header.")?;
            Some((k.to_string(), v.trim().trim_matches('\'').trim().to_string()))
        })
        .collect()
}

/// A legacy file name: processor, optional source name, channel / electrode, start number.
#[derive(Debug, Clone, PartialEq)]
pub struct Name {
    pub processor: String,
    pub source: Option<String>,
    /// `CH3`, `AUX1`, `ADC2` (continuous); the electrode for spikes; `all_channels`, `messages`.
    pub item: String,
    /// 1 for the first start (no suffix), else the `_<n>` suffix.
    pub start: u32,
}

impl Name {
    pub fn parse(path: &Path) -> Option<Self> {
        let stem = path.file_stem()?.to_string_lossy().into_owned();
        let mut parts: Vec<&str> = stem.split('_').collect();
        let start = match parts.last().and_then(|p| p.parse::<u32>().ok()) {
            Some(n) if parts.len() > 1 && !parts.last().is_some_and(|p| p.starts_with('0') && p.len() > 1) => {
                parts.pop();
                n
            }
            _ => 1,
        };
        let ext = path.extension()?.to_string_lossy().into_owned();
        if ext != "continuous" {
            return Some(Self { processor: String::new(), source: None, item: parts.join("_"), start });
        }
        match parts.as_slice() {
            [p, item] => Some(Self { processor: p.to_string(), source: None, item: item.to_string(), start }),
            [p, source @ .., item] if !source.is_empty() => Some(Self { processor: p.to_string(), source: Some(source.join("_")), item: item.to_string(), start }),
            _ => None,
        }
    }

    /// `CH`, `AUX`, `ADC` (the letters before the number).
    pub fn kind(&self) -> &str {
        self.item.trim_end_matches(|c: char| c.is_ascii_digit())
    }

    pub fn number(&self) -> u32 {
        self.item[self.kind().len()..].parse().unwrap_or(0)
    }
}

/// Folders holding `.continuous` files at or below `path` (two levels), sorted.
pub fn folders(path: &Path) -> Vec<PathBuf> {
    fn has(dir: &Path) -> bool {
        std::fs::read_dir(dir).is_ok_and(|rd| rd.flatten().any(|e| e.path().extension().is_some_and(|x| x == "continuous")))
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
        for e in rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())) {
            walk(&e.path(), depth - 1, out);
        }
    }
    let start = if path.is_file() { path.parent().unwrap_or(path) } else { path };
    let mut out = Vec::new();
    walk(start, 2, &mut out);
    out.sort();
    out
}

/// Acquisition starts in `dir` (1, 2, …).
pub fn starts(dir: &Path) -> Vec<u32> {
    let mut out: Vec<u32> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "continuous")).filter_map(|p| Name::parse(&p)).map(|n| n.start).collect())
        .unwrap_or_default();
    out.sort_unstable();
    out.dedup();
    out
}

/// One channel file: its mapped bytes and where each record's samples go.
struct ChannelFile {
    file: Arc<MappedFile>,
    /// Record index of the first record at or after the stream's first sample number.
    first: usize,
}

/// A `.continuous` file with its header and record count.
struct OpenedFile {
    file: Arc<MappedFile>,
    header: BTreeMap<String, String>,
    name: Name,
    records: usize,
}

impl OpenedFile {
    /// Sample number of record `k`.
    fn number(&self, k: usize) -> i64 {
        i64_at(self.file.bytes(), HEADER + k * RECORD)
    }
}

/// Channels of one processor read from their own files, on one sample axis.
pub struct LegacyRecording {
    info: RecordingInfo,
    files: Vec<ChannelFile>,
    /// First sample number (of the first record) and the sample number of every record, from
    /// the first channel's file (records of all channels line up).
    first_number: i64,
    numbers: Vec<i64>,
    /// No gaps: record k holds samples k × 1024 … (fast path).
    contiguous: bool,
}

impl LegacyRecording {
    /// Record index (relative to `first`) and offset holding sample `s`, if any.
    fn locate(&self, s: u64) -> Option<(usize, usize)> {
        if self.contiguous {
            return Some((s as usize / BLOCK, s as usize % BLOCK));
        }
        let n = self.first_number + s as i64;
        let k = self.numbers.partition_point(|&x| x <= n).checked_sub(1)?;
        let off = (n - self.numbers[k]) as usize;
        (off < BLOCK).then_some((k, off))
    }

    fn each(&self, channels: &[usize], samples: Range<u64>, mut f: impl FnMut(usize, usize, i16)) {
        for (i, &c) in channels.iter().enumerate() {
            let cf = &self.files[c];
            let bytes = cf.file.bytes();
            for (t, s) in (samples.start..samples.end).enumerate() {
                let v = self
                    .locate(s)
                    .and_then(|(k, off)| {
                        let at = HEADER + (cf.first + k) * RECORD + 12 + off * 2;
                        bytes.get(at..at + 2).map(|b| i16::from_be_bytes([b[0], b[1]]))
                    })
                    .unwrap_or(0);
                f(i, t, v);
            }
        }
    }
}

impl Recording for LegacyRecording {
    fn info(&self) -> &RecordingInfo {
        &self.info
    }

    fn read(&self, channels: &[usize], samples: Range<u64>, out: &mut [f32]) -> Result<()> {
        let n = check_read(&self.info, channels, &samples, out.len())?;
        let gains: Vec<f64> = channels.iter().map(|&c| self.info.channels[c].gain).collect();
        self.each(channels, samples, |i, t, v| out[i * n + t] = (v as f64 * gains[i]) as f32);
        Ok(())
    }

    /// Stored samples as little-endian int16 (the files hold big-endian).
    fn read_stored(&self, channels: &[usize], samples: Range<u64>, out: &mut [u8]) -> Result<bool> {
        let n = check_read(&self.info, channels, &samples, out.len() / 2)?;
        if out.len() != channels.len() * n * 2 {
            return Err(Error::BufferSize { expected: channels.len() * n * 2, actual: out.len() });
        }
        self.each(channels, samples, |i, t, v| {
            let at = (i * n + t) * 2;
            out[at..at + 2].copy_from_slice(&v.to_le_bytes());
        });
        Ok(true)
    }
}

/// Waveforms of a `.spikes` file: snippet = (record, channel within the electrode).
pub struct SpikeFile {
    file: Arc<MappedFile>,
    record: usize,
    channels: usize,
    samples: usize,
    snippets: Vec<(u32, u16)>,
}

impl std::fmt::Debug for SpikeFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpikeFile").field("channels", &self.channels).field("samples", &self.samples).field("snippets", &self.snippets.len()).finish()
    }
}

impl Waveforms for SpikeFile {
    fn count(&self) -> usize {
        self.snippets.len()
    }

    fn read(&self, snippets: &[usize], out: &mut [f32]) -> Result<()> {
        let bytes = self.file.bytes();
        let (n, m) = (self.channels, self.samples);
        for (o, &i) in out.chunks_exact_mut(m.max(1)).zip(snippets) {
            let &(r, c) = self.snippets.get(i).ok_or_else(|| Error::format("openephys", format!("spike {i} out of range")))?;
            let base = HEADER + r as usize * self.record;
            let wave = base + 42 + c as usize * m * 2;
            let gain_at = base + 42 + n * m * 2 + c as usize * 4;
            let gain = f32::from_le_bytes(bytes[gain_at..gain_at + 4].try_into().expect("4 bytes")) as f64;
            let scale = if gain != 0.0 { 1e-3 / gain } else { 0.0 };
            for (k, v) in o.iter_mut().enumerate() {
                let at = wave + k * 2;
                *v = ((u16::from_le_bytes([bytes[at], bytes[at + 1]]) as f64 - 32768.0) * scale) as f32;
            }
        }
        Ok(())
    }
}

fn i64_at(b: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

/// The files of start `start` in `dir` with `ext`, by parsed name.
fn files_of(dir: &Path, start: u32, ext: &str) -> Vec<(PathBuf, Name)> {
    let mut out: Vec<(PathBuf, Name)> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == ext)).filter_map(|p| Some((p.clone(), Name::parse(&p)?))).filter(|(_, n)| n.start == start).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| (&a.1.processor, &a.1.source, a.1.kind(), a.1.number()).cmp(&(&b.1.processor, &b.1.source, b.1.kind(), b.1.number())));
    out
}

/// Processor names from `settings.xml` (`<PROCESSOR name="Sources/Rhythm FPGA" … NodeId="100">`).
fn processor_names(settings: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for part in settings.split("<PROCESSOR ").skip(1) {
        let attr = |key: &str| {
            let at = part.find(&format!("{key}=\""))? + key.len() + 2;
            Some(part[at..at + part[at..].find('"')?].to_string())
        };
        if let (Some(name), Some(id)) = (attr("name"), attr("NodeId").or_else(|| attr("nodeId"))) {
            out.entry(id).or_insert_with(|| name.rsplit('/').next().unwrap_or(&name).replace(' ', "_"));
        }
    }
    out
}

/// Opens start `start` of the legacy folder `dir`.
pub fn open(dir: &Path, start: u32, options: &OpenOptions) -> Result<Session> {
    let mut s = Session::default();
    let mut warnings = Vec::new();
    let mut paths = Vec::new();
    let settings_path = [dir.join(format!("settings_{start}.xml")), dir.join("settings.xml")].into_iter().find(|p| p.is_file() && (start != 1 || !p.ends_with(format!("settings_{start}.xml"))));
    let settings = settings_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    paths.extend(settings_path.clone());
    let names = processor_names(&settings);
    let parsed = crate::settings::parse(&settings);

    // Continuous files grouped by processor (and source name)
    type Group = (String, Option<String>);
    let mut groups: BTreeMap<Group, Vec<(PathBuf, Name)>> = BTreeMap::new();
    for (p, n) in files_of(dir, start, "continuous") {
        groups.entry((n.processor.clone(), n.source.clone())).or_default().push((p, n));
    }
    if groups.is_empty() {
        return Err(Error::format("openephys", format!("no .continuous files for start {start} in {}", dir.display())));
    }
    let mut clocks: BTreeMap<String, (f64, i64)> = BTreeMap::new();
    let mut pending: Vec<LegacyRecording> = Vec::new();
    for ((proc, source), list) in &groups {
        let label = match source {
            Some(src) => format!("{src}-{proc}"),
            None => names.get(proc).map_or_else(|| format!("Processor-{proc}"), |n| format!("{n}-{proc}")),
        };
        // Open every file; the common sample-number range of all channels
        let mut opened = Vec::new();
        for (p, n) in list {
            let file = Arc::new(MappedFile::open(p)?);
            let h = header(file.bytes());
            let records = (file.bytes().len().saturating_sub(HEADER)) / RECORD;
            if (file.bytes().len().saturating_sub(HEADER)) % RECORD != 0 {
                warnings.push(format!("{}: ends with a partial record", p.display()));
            }
            paths.push(p.clone());
            opened.push(OpenedFile { file, header: h, name: n.clone(), records });
        }
        let first_numbers: Vec<i64> = opened.iter().filter(|o| o.records > 0).map(|o| o.number(0)).collect();
        let Some(&first_number) = first_numbers.iter().max() else {
            warnings.push(format!("{label}: no records"));
            continue;
        };
        let reference = &opened[0];
        let numbers_all: Vec<i64> = (0..reference.records).map(|k| reference.number(k)).collect();
        let last_end = opened.iter().filter(|o| o.records > 0).map(|o| o.number(o.records - 1) + BLOCK as i64).min().unwrap_or(first_number);
        if first_numbers.iter().any(|&f| f != first_number) || opened.iter().any(|o| o.records != reference.records) {
            warnings.push(format!("{label}: channel files do not cover the same samples; clipped to the common range (as neo)"));
        }
        let numbers: Vec<i64> = numbers_all.iter().copied().filter(|&x| x >= first_number && x < last_end).collect();
        let contiguous = numbers.windows(2).all(|w| w[1] - w[0] == BLOCK as i64);
        if !contiguous {
            warnings.push(format!("{label}: the records have gaps (recording paused?); missing samples read as 0 (as neo)"));
        }
        if numbers.windows(2).any(|w| w[1] <= w[0]) {
            warnings.push(format!("{label}: record sample numbers go backwards; the data may be corrupted"));
        }
        let samples = (last_end - first_number).max(0) as u64;
        let rate = reference.header.get("sampleRate").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        clocks.insert(label.clone(), (rate, first_number));

        // Electrode channels (CH) and analog inputs (AUX, ADC)
        for analog in [false, true] {
            let chosen: Vec<&OpenedFile> = opened.iter().filter(|o| (o.name.kind() != "CH") == analog).collect();
            if chosen.is_empty() {
                continue;
            }
            let name = if analog { format!("{label}.analog") } else { label.clone() };
            if !options.wants(&name) {
                continue;
            }
            let channels = chosen
                .iter()
                .map(|o| {
                    let bit = o.header.get("bitVolts").and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
                    ChannelInfo { name: o.name.item.clone(), gain: if analog { bit } else { bit * 1e-6 }, offset: 0.0 }
                })
                .collect();
            let files = chosen
                .iter()
                .map(|o| {
                    let first = (0..o.records).position(|k| o.number(k) >= first_number).unwrap_or(0);
                    ChannelFile { file: o.file.clone(), first }
                })
                .collect();
            let mut metadata = BTreeMap::new();
            metadata.insert("openephys_first_sample".to_string(), first_number.to_string());
            let info = RecordingInfo {
                name,
                description: if analog { format!("Analog inputs of {label} (AUX / ADC)") } else { format!("Electrode channels of {label}") },
                channels,
                samples,
                sample_rate: rate,
                start_time: first_number as f64 / rate.max(1e-9),
                unit: "V".into(),
                calibration: Calibration::Known,
                kind: if analog { SignalKind::Other } else { SignalKind::Electrical },
                stored_as: SampleType::I16,
                order: MemoryOrder::ChannelMajor,
                storage: "continuous".into(),
                metadata,
            };
            pending.push(LegacyRecording { info, files, first_number, numbers: numbers.clone(), contiguous });
        }
    }
    let t0 = pending.iter().map(|p| p.info.start_time).fold(f64::INFINITY, f64::min);
    let t0 = if t0.is_finite() { t0 } else { 0.0 };
    for mut rec in pending {
        rec.info.start_time -= t0;
        let info = &rec.info;
        if info.kind == SignalKind::Electrical {
            let group = info.name.clone();
            s.electrode_groups.push(ElectrodeGroup { name: group.clone(), description: format!("Electrodes of {group}"), location: "unknown".into(), device: Some(group.clone()) });
            s.metadata.devices.push(Device { name: group.clone(), description: format!("Acquisition source of {group}"), manufacturer: None, model: None });
            for (k, ch) in info.channels.iter().enumerate() {
                s.electrodes.push(Electrode {
                    name: format!("{group} {}", ch.name),
                    group: group.clone(),
                    channels: vec![nc_core::ChannelRef { recording: info.name.clone(), channel: k }],
                    ..Default::default()
                });
            }
        }
        s.recordings.push(Arc::new(rec));
    }
    let (rate0, _) = clocks.values().next().copied().unwrap_or((1.0, 0));
    let label_of = |proc: u8| clocks.keys().find(|l| l.ends_with(&format!("-{proc}"))).cloned().unwrap_or_else(|| format!("Processor-{proc}"));

    // TTL events
    let suffix = if start == 1 { String::new() } else { format!("_{start}") };
    let events_path = dir.join(format!("all_channels{suffix}.events"));
    let events_alt: Option<PathBuf> = std::fs::read_dir(dir).ok().and_then(|rd| {
        rd.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "events") && Name::parse(p).is_some_and(|n| n.start == start && n.item != "messages" && n.item != "all_channels"))
    });
    if let Some(p) = Some(events_path).filter(|p| p.is_file()).or(events_alt) {
        let file = MappedFile::open(&p)?;
        let b = file.bytes();
        let rate = header(b).get("sampleRate").and_then(|v| v.parse::<f64>().ok()).unwrap_or(rate0);
        paths.push(p.clone());
        // (label, line) → (time, rising)
        let mut lines: BTreeMap<(String, u32), Vec<(f64, bool)>> = BTreeMap::new();
        for r in (HEADER..b.len().saturating_sub(15)).step_by(16) {
            if b[r + 10] != 3 {
                continue;
            }
            let t = i64_at(b, r) as f64 / rate - t0;
            lines.entry((label_of(b[r + 11]), b[r + 13] as u32 + 1)).or_default().push((t, b[r + 12] == 1));
        }
        for ((label, line), changes) in lines {
            let name = format!("{label} TTL {line}");
            if !options.wants(&name) {
                continue;
            }
            let (mut onsets, mut offsets) = (Vec::new(), Vec::new());
            let mut high: Option<f64> = None;
            for (t, rising) in changes {
                match (rising, high) {
                    (true, None) => high = Some(t),
                    (false, Some(on)) => {
                        onsets.push(on);
                        offsets.push(t);
                        high = None;
                    }
                    _ => {}
                }
            }
            // Still high at the end (lines may even go high after the last sample)
            if let Some(on) = high {
                onsets.push(on);
                offsets.push(s.duration().max(on));
            }
            if onsets.is_empty() {
                continue;
            }
            let n = onsets.len();
            s.events.push(EventSeries { name, description: format!("TTL line {line} of {label} (high periods)"), onsets, offsets: Some(offsets), values: vec![1.0; n], channels: 1, labels: Vec::new() });
        }
    }

    // Messages: `<sample number> <text>`
    let messages = dir.join(format!("messages{suffix}.events"));
    if let Ok(text) = std::fs::read_to_string(&messages) {
        paths.push(messages.clone());
        let (mut onsets, mut labels) = (Vec::new(), Vec::new());
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (num, msg) = line.split_once(' ').unwrap_or((line, ""));
            let Ok(n) = num.parse::<i64>() else { continue };
            onsets.push(n as f64 / rate0 - t0);
            labels.push(msg.trim().to_string());
        }
        if !onsets.is_empty() && options.wants("messages") {
            // Lines are not always in time order
            let mut order: Vec<usize> = (0..onsets.len()).collect();
            order.sort_by(|&a, &b| onsets[a].total_cmp(&onsets[b]));
            let (onsets, labels): (Vec<f64>, Vec<String>) = order.into_iter().map(|i| (onsets[i], std::mem::take(&mut labels[i]))).unzip();
            let n = onsets.len();
            s.events.push(EventSeries { name: "messages".into(), description: "Messages from the GUI".into(), onsets, offsets: None, values: vec![0.0; n], channels: 1, labels });
        }
    }

    // Spikes: one snippet store per electrode file, one snippet per channel of each spike
    for (p, n) in files_of(dir, start, "spikes") {
        let file = Arc::new(MappedFile::open(&p)?);
        let b = file.bytes();
        if b.len() < HEADER + 23 {
            continue;
        }
        let h = header(b);
        let (channels, samples) = (u16_at(b, HEADER + 19) as usize, u16_at(b, HEADER + 21) as usize);
        let record = 42 + channels * samples * 2 + channels * 4 + channels * 2 + 2;
        let count = (b.len() - HEADER) / record;
        let rate = h.get("sampleRate").and_then(|v| v.parse::<f64>().ok()).unwrap_or(rate0);
        if !options.wants(&n.item) {
            continue;
        }
        paths.push(p.clone());
        let mut sn = SnippetSeries {
            name: n.item.clone(),
            description: format!("Open Ephys spikes of electrode {}", h.get("electrode").map_or(n.item.as_str(), |e| e.as_str())),
            sample_rate: rate,
            samples_per_snippet: samples,
            unit: "V".into(),
            ..Default::default()
        };
        let mut index = Vec::with_capacity(count * channels);
        for r in 0..count {
            let at = HEADER + r * record;
            let t = i64_at(b, at + 1) as f64 / rate - t0;
            let sorted = u16_at(b, at + 23);
            for c in 0..channels {
                sn.timestamps.push(t);
                sn.channels.push(c as u16 + 1);
                sn.sort_codes.push(sorted);
                index.push((r as u32, c as u16));
            }
        }
        sn.waveforms = Arc::new(SpikeFile { file: file.clone(), record, channels, samples, snippets: index });
        s.snippets.push(sn);
    }

    let m = &mut s.metadata;
    m.experiment = Some(format!("experiment{start}"));
    m.start_time = parsed.date.as_deref().and_then(crate::settings::iso_date);
    if let Some(v) = &parsed.version {
        m.extra.insert("openephys_gui_version".into(), v.clone());
    }
    m.extra.insert("openephys_format".into(), "legacy (.continuous)".into());
    let mut prov = Provenance::new("openephys");
    prov.version = Some(format!("legacy format{}", parsed.version.as_ref().map_or_else(String::new, |v| format!(", GUI {v}"))));
    for p in &paths {
        prov.add_file(p);
    }
    prov.warnings = warnings;
    s.provenance = prov;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_names_and_header() {
        let n = Name::parse(Path::new("100_CH12_2.continuous")).unwrap();
        assert_eq!((n.processor.as_str(), n.source.as_deref(), n.item.as_str(), n.start, n.kind(), n.number()), ("100", None, "CH12", 2, "CH", 12));
        let n = Name::parse(Path::new("127_RhythmData-A_AUX3.continuous")).unwrap();
        assert_eq!((n.source.as_deref(), n.item.as_str(), n.start, n.kind()), (Some("RhythmData-A"), "AUX3", 1, "AUX"));
        let n = Name::parse(Path::new("STp106.0n0_2.spikes")).unwrap();
        assert_eq!((n.item.as_str(), n.start), ("STp106.0n0", 2));
        assert_eq!(Name::parse(Path::new("all_channels.events")).unwrap().item, "all_channels");
        let mut h = b"header.format = 'Open Ephys Data Format'; \nheader.sampleRate = 40000;\nheader.bitVolts = 0.05;\n".to_vec();
        h.resize(HEADER, b' ');
        let h = header(&h);
        assert_eq!((h["sampleRate"].as_str(), h["bitVolts"].as_str(), h["format"].as_str()), ("40000", "0.05", "Open Ephys Data Format"));
    }

    fn head(lines: &str) -> Vec<u8> {
        let mut h = lines.as_bytes().to_vec();
        h.resize(HEADER, b' ');
        h
    }

    /// A channel file: records starting at the given sample numbers, sample value = number + k.
    fn continuous(numbers: &[i64], rate: u32, bit: f64, value: impl Fn(i64) -> i16) -> Vec<u8> {
        let mut out = head(&format!("header.format = 'Open Ephys Data Format';\nheader.sampleRate = {rate};\nheader.bitVolts = {bit};\n"));
        for &n in numbers {
            out.extend(n.to_le_bytes());
            out.extend((BLOCK as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            for k in 0..BLOCK as i64 {
                out.extend(value(n + k).to_be_bytes());
            }
            out.extend([0, 1, 2, 3, 4, 5, 6, 7, 8, 255]);
        }
        out
    }

    #[test]
    fn test_legacy_folder() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/openephys-legacy-fixture");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Records at 1024, 2048 and (after a gap of one record) 4096; second start: one record
        let numbers = [1024, 2048, 4096];
        std::fs::write(dir.join("100_CH1.continuous"), continuous(&numbers, 1000, 0.195, |n| (n % 1000) as i16)).unwrap();
        std::fs::write(dir.join("100_CH2.continuous"), continuous(&numbers, 1000, 0.195, |n| -((n % 1000) as i16))).unwrap();
        std::fs::write(dir.join("100_ADC1.continuous"), continuous(&numbers, 1000, 0.00015, |_| 1000)).unwrap();
        std::fs::write(dir.join("100_CH1_2.continuous"), continuous(&[0], 1000, 0.195, |_| 7)).unwrap();
        std::fs::write(dir.join("100_CH2_2.continuous"), continuous(&[0], 1000, 0.195, |_| 7)).unwrap();
        std::fs::write(dir.join("settings.xml"), "<SETTINGS><INFO><DATE>3 Oct 2018 13:16:50</DATE></INFO><PROCESSOR name=\"Sources/Rhythm FPGA\" NodeId=\"100\"></PROCESSOR></SETTINGS>").unwrap();
        // TTL line 2 (channel 1): high 1.5–1.6 s; a falling edge first is ignored
        let mut ev = head("header.sampleRate = 1000;\n");
        for (n, id, ch) in [(1100i64, 0u8, 1u8), (1500, 1, 1), (1600, 0, 1)] {
            ev.extend(n.to_le_bytes());
            ev.extend(0i16.to_le_bytes());
            ev.extend([3, 100, id, ch]);
            ev.extend(0u16.to_le_bytes());
        }
        std::fs::write(dir.join("all_channels.events"), ev).unwrap();
        std::fs::write(dir.join("messages.events"), "2000 second\n1030 first\n").unwrap();
        // Two stereotrode spikes (2 channels × 3 samples), gain 10: raw 32768 + 50 → 5 / 1000 V
        let mut sp = head("header.num_channels = 2;\nheader.sampleRate = 1000;\n");
        for (n, sorted) in [(1200i64, 1u16), (1300, 2)] {
            sp.push(4);
            sp.extend(n.to_le_bytes());
            sp.extend(0i64.to_le_bytes());
            for v in [0u16, 2, 3, sorted, 0, 0] {
                sp.extend(v.to_le_bytes());
            }
            sp.extend([0, 0, 0]);
            sp.extend([0u8; 8]);
            sp.extend(1000u16.to_le_bytes());
            for k in 0..6u16 {
                sp.extend((32768 + 50 * (k / 3 + 1)).to_le_bytes());
            }
            sp.extend(10f32.to_le_bytes());
            sp.extend(10f32.to_le_bytes());
            sp.extend([0u8; 4]);
            sp.extend(0u16.to_le_bytes());
        }
        std::fs::write(dir.join("SEp100.0n0.spikes"), sp).unwrap();

        let reader = crate::OpenEphys;
        use nc_core::Reader as _;
        assert_eq!(reader.containers(&dir), vec!["experiment1", "experiment2"]);
        let s = nc_core::testkit::check_reader(&reader, &dir, &OpenOptions { block: Some("experiment1".into()), ..Default::default() });
        let names: Vec<&str> = s.recordings.iter().map(|r| r.info().name.as_str()).collect();
        assert_eq!(names, vec!["Rhythm_FPGA-100", "Rhythm_FPGA-100.analog"]);
        let ch = s.recording("Rhythm_FPGA-100").unwrap();
        assert_eq!((ch.info().samples, ch.info().start_time, ch.info().channels[0].gain), (4096, 0.0, 0.195 * 1e-6));
        let mut out = vec![0.0; 4];
        // Samples 1023..1025 (numbers 2047, 2048) then the gap (3072 → 0)
        ch.read(&[0], 1023..1025, &mut out[..2]).unwrap();
        ch.read(&[1], 2048..2050, &mut out[2..]).unwrap();
        assert_eq!(out, vec![(47.0 * 0.195 * 1e-6) as f32, (48.0 * 0.195 * 1e-6) as f32, 0.0, 0.0]);
        let mut raw = vec![0u8; 2];
        ch.read_stored(&[1], 3072..3073, &mut raw).unwrap();
        assert_eq!(i16::from_le_bytes([raw[0], raw[1]]), -96, "sample 4096 after the gap, little-endian");
        assert!(s.provenance.warnings.iter().any(|w| w.contains("gaps")));
        let adc = s.recording("Rhythm_FPGA-100.analog").unwrap();
        adc.read(&[0], 0..1, &mut out[..1]).unwrap();
        assert_eq!(out[0], (1000.0 * 0.00015) as f32);

        let ttl = s.event_series("Rhythm_FPGA-100 TTL 2").unwrap();
        assert_eq!((ttl.onsets.clone(), ttl.offsets.clone().unwrap()), (vec![1500.0 / 1000.0 - 1.024], vec![1600.0 / 1000.0 - 1.024]));
        let msg = s.event_series("messages").unwrap();
        assert_eq!(msg.labels, vec!["first", "second"]);

        let sn = &s.snippets[0];
        assert_eq!((sn.name.as_str(), sn.len(), sn.channels.clone(), sn.sort_codes.clone()), ("SEp100.0n0", 4, vec![1, 2, 1, 2], vec![1, 1, 2, 2]));
        assert_eq!(sn.read(&[1]).unwrap(), vec![(100.0 / 10.0 * 1e-3) as f32; 3]);
        assert!((sn.timestamps[2] - (1.3 - 1.024)).abs() < 1e-12);

        let second = reader.open(&dir, &OpenOptions { block: Some("experiment2".into()), ..Default::default() }).unwrap();
        assert_eq!(second.recordings[0].info().samples, 1024);
        assert!(second.events.is_empty() && second.snippets.is_empty());
    }
}
