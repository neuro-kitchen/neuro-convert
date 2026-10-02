//! nc-intan: Intan Technologies recordings — RHD (RHD2000 USB interface board / Recording
//! Controller) and RHS (Stimulation / Recording Controller), from the RHX software and the older
//! Intan programs.
//!
//! Three layouts, all opened from what the user selects:
//! - **traditional**: one `.rhd` / `.rhs` file, header then data blocks;
//! - **one file per signal type**: a folder with `info.rhd` / `info.rhs`, `time.dat`,
//!   `amplifier.dat`, `auxiliary.dat`, `analogin.dat`, `digitalin.dat`, … (RHS: `dcamplifier.dat`,
//!   `stim.dat`, `analogout.dat`);
//! - **one file per channel**: a folder with `info.*`, `time.dat`, `amp-A-000.dat`,
//!   `aux-A-AUX1.dat`, `board-ANALOG-IN-1.dat`, `board-DIGITAL-IN-01.dat`, ….
//!
//! Streams: `amplifier` (volts, electrical: one electrode per channel, one electrode group per
//! headstage port, impedances from the header), RHD `aux` / `supply` / `temperature`, `analog_in`,
//! RHS `analog_out`, `dc_amplifier` (shares the amplifier electrodes) and `stim` (amperes).
//! Digital inputs / outputs become one event series per line (high periods). Samples stay 16-bit;
//! scales follow Intan's readers (`importrhdutilities.py` / `importrhsutilities.py`).
//!
//! Not yet: joining the consecutive files of a traditional recording split by time (each file is
//! a container), notch filtering (Intan's readers apply it on request; RHX ≥ 3 saves filtered data).

pub mod data;
pub mod header;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use data::{high_periods, raw32, Column, Decode, IntanRecording, Place};
use header::{Header, Kind, Signal};
use nc_base::mapped::MappedFile;
use nc_core::{
    Calibration, ChannelInfo, ChannelRef, Detection, Device, Electrode, ElectrodeGroup, Error, EventSeries, MemoryOrder, OpenOptions, Provenance,
    Reader, RecordingInfo, Result, SampleType, Session, SignalKind,
};

pub struct Intan;

/// How a recording is laid out on disk.
#[derive(Debug, Clone, PartialEq)]
enum Layout {
    Traditional(PathBuf),
    /// Folder and its `info.*` file.
    PerType(PathBuf, PathBuf),
    PerChannel(PathBuf, PathBuf),
}

fn is_intan_ext(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("rhd") || e.eq_ignore_ascii_case("rhs"))
}

/// Traditional files in a folder, sorted by name.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_intan_ext(p)).collect()).unwrap_or_default();
    v.sort();
    v
}

fn folder_layout(dir: &Path) -> Option<Layout> {
    let info = ["info.rhd", "info.rhs"].iter().map(|n| dir.join(n)).find(|p| p.is_file())?;
    let per_type = ["amplifier.dat", "auxiliary.dat", "analogin.dat", "digitalin.dat", "dcamplifier.dat", "stim.dat"].iter().any(|n| dir.join(n).is_file());
    Some(if per_type { Layout::PerType(dir.to_path_buf(), info) } else { Layout::PerChannel(dir.to_path_buf(), info) })
}

/// The layout `path` points at; `block` picks one of several traditional files in a folder.
fn layout(path: &Path, block: Option<&str>) -> Result<Layout> {
    if path.is_file() {
        if !is_intan_ext(path) {
            return Err(Error::format("intan", format!("{} is not an .rhd / .rhs file", path.display())));
        }
        let info = path.file_stem().is_some_and(|s| s == "info");
        if let (true, Some(dir)) = (info, path.parent())
            && let Some(l) = folder_layout(dir)
        {
            return Ok(l);
        }
        return Ok(Layout::Traditional(path.to_path_buf()));
    }
    if let Some(l) = folder_layout(path) {
        return Ok(l);
    }
    let files = files_in(path);
    let stem = |p: &PathBuf| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match (block, files.len()) {
        (Some(name), _) => files.iter().find(|f| stem(f) == name).cloned().map(Layout::Traditional).ok_or_else(|| Error::Unsupported(format!("{name}: no such .rhd / .rhs file in {}", path.display()))),
        (None, 1) => Ok(Layout::Traditional(files[0].clone())),
        (None, 0) => Err(Error::format("intan", format!("no .rhd / .rhs file or info.rhd in {}", path.display()))),
        (None, n) => Err(Error::Unsupported(format!(
            "{} holds {n} Intan files (a recording split in time, or several recordings); choose one with --block <name>: {}",
            path.display(),
            files.iter().map(stem).collect::<Vec<_>>().join(", ")
        ))),
    }
}

impl Reader for Intan {
    fn name(&self) -> &'static str {
        "intan"
    }

    fn description(&self) -> &'static str {
        "Intan RHD / RHS recording (RHX and older Intan software)"
    }

    fn opens(&self) -> &'static str {
        "An .rhd / .rhs file, or an RHX folder with info.rhd / info.rhs (one file per signal type or per channel)"
    }

    fn versions(&self) -> &'static [&'static str] {
        &[
            "RHD 1.0–3.x: amplifier, auxiliary, supply, temperature, board ADC, digital in / out",
            "RHS 1.0–3.x: amplifier, DC amplifier, stimulation current, board ADC / DAC, digital in / out",
            "Layouts: traditional file, one file per signal type, one file per channel",
        ]
    }

    fn detect(&self, path: &Path) -> Option<Detection> {
        let header_file = match layout(path, None) {
            Ok(Layout::Traditional(f)) => f,
            Ok(Layout::PerType(_, info) | Layout::PerChannel(_, info)) => info,
            Err(_) => files_in(path).into_iter().next()?,
        };
        let bytes = std::fs::read(&header_file).ok()?;
        let kind = header::kind_of(&bytes)?;
        let h = header::parse(&bytes).ok();
        let version = h.map(|h| format!("{} {}.{}", if kind == Kind::Rhd { "RHD" } else { "RHS" }, h.version.0, h.version.1));
        Some(Detection { format: "intan", version, confidence: 0.95 })
    }

    fn containers(&self, path: &Path) -> Vec<String> {
        if !path.is_dir() || folder_layout(path).is_some() {
            return Vec::new();
        }
        let files = files_in(path);
        if files.len() > 1 { files.iter().filter_map(|f| f.file_stem().map(|s| s.to_string_lossy().into_owned())).collect() } else { Vec::new() }
    }

    fn open(&self, path: &Path, options: &OpenOptions) -> Result<Session> {
        let layout = layout(path, options.block.as_deref())?;
        let header_file = match &layout {
            Layout::Traditional(f) => f.clone(),
            Layout::PerType(_, i) | Layout::PerChannel(_, i) => i.clone(),
        };
        let file = Arc::new(MappedFile::open(&header_file)?);
        let h = header::parse(file.bytes())?;
        let mut b = Builder { options, h: &h, session: Session::default(), warnings: Vec::new(), files: vec![header_file.clone()], samples: 0 };
        match &layout {
            Layout::Traditional(_) => b.traditional(file)?,
            Layout::PerType(dir, _) => b.per_type(dir)?,
            Layout::PerChannel(dir, _) => b.per_channel(dir)?,
        }
        let name_source = match &layout {
            Layout::Traditional(f) => f.file_stem().map(|s| s.to_string_lossy().into_owned()),
            Layout::PerType(d, _) | Layout::PerChannel(d, _) => d.file_name().map(|s| s.to_string_lossy().into_owned()),
        };
        b.finish(name_source.unwrap_or_default(), &layout)
    }
}

/// Scale of a signal: decode, gain, offset, unit.
fn scale(h: &Header, signal: Signal, traditional: bool) -> (Decode, f64, f64, &'static str) {
    match signal {
        // µV per bit; traditional files store it offset by 32768
        Signal::Amplifier => (if traditional { Decode::Offset } else { Decode::I16 }, 0.195e-6, 0.0, "V"),
        Signal::Aux => (Decode::U16, 37.4e-6, 0.0, "V"),
        Signal::Supply => (Decode::U16, 74.8e-6, 0.0, "V"),
        Signal::AnalogIn if h.kind == Kind::Rhd => match h.board_mode {
            1 => (Decode::Offset, 152.59e-6, 0.0, "V"),
            13 => (Decode::Offset, 312.5e-6, 0.0, "V"),
            _ => (Decode::U16, 50.354e-6, 0.0, "V"),
        },
        Signal::AnalogIn | Signal::AnalogOut => (Decode::Offset, 312.5e-6, 0.0, "V"),
        Signal::DigitalIn | Signal::DigitalOut => (Decode::U16, 1.0, 0.0, "a.u."),
    }
}

fn stored_as(decode: Decode) -> SampleType {
    if decode == Decode::U16 { SampleType::U16 } else { SampleType::I16 }
}

/// A stream to add: channels with their columns.
struct Stream {
    name: &'static str,
    description: String,
    kind: SignalKind,
    unit: &'static str,
    decode: Decode,
    rate: f64,
    samples: u64,
    channels: Vec<(ChannelInfo, Column)>,
    order: MemoryOrder,
    storage: &'static str,
}

struct Builder<'a> {
    options: &'a OpenOptions,
    h: &'a Header,
    session: Session,
    warnings: Vec<String>,
    files: Vec<PathBuf>,
    /// Samples at the full rate.
    samples: u64,
}

impl Builder<'_> {
    fn add(&mut self, s: Stream) {
        if s.channels.is_empty() || !self.options.wants(s.name) {
            return;
        }
        let (channels, columns): (Vec<ChannelInfo>, Vec<Column>) = s.channels.into_iter().unzip();
        let info = RecordingInfo {
            name: s.name.into(),
            description: s.description,
            channels,
            samples: s.samples,
            sample_rate: s.rate,
            start_time: 0.0,
            unit: s.unit.into(),
            calibration: Calibration::Known,
            kind: s.kind,
            stored_as: stored_as(s.decode),
            order: s.order,
            storage: s.storage.into(),
            metadata: BTreeMap::new(),
        };
        self.session.recordings.push(Arc::new(IntanRecording::new(info, columns, s.decode)));
    }

    fn channel(&self, c: &header::Channel, gain: f64, offset: f64, suffix: &str) -> ChannelInfo {
        let name = if c.custom.is_empty() { c.native.clone() } else { c.custom.clone() };
        ChannelInfo { name: format!("{name}{suffix}"), gain, offset }
    }

    /// The analog streams of one signal type, with `place(k)` the column of its k-th channel.
    fn analog(&mut self, signal: Signal, traditional: bool, rate: f64, samples: u64, mut column: impl FnMut(usize, &header::Channel) -> Option<Column>, storage: &'static str) {
        let h = self.h;
        let (decode, gain, offset, unit) = scale(h, signal, traditional);
        let (name, description, kind) = match signal {
            Signal::Amplifier => ("amplifier", "Amplifier channels (headstage electrodes)".to_string(), SignalKind::Electrical),
            Signal::Aux => ("aux", "Headstage auxiliary inputs (e.g. accelerometer)".to_string(), SignalKind::Other),
            Signal::Supply => ("supply", "Headstage supply voltage".to_string(), SignalKind::Other),
            Signal::AnalogIn => ("analog_in", "Board analog inputs (ADC)".to_string(), SignalKind::Other),
            Signal::AnalogOut => ("analog_out", "Board analog outputs (DAC)".to_string(), SignalKind::Other),
            Signal::DigitalIn | Signal::DigitalOut => return,
        };
        let chans: Vec<header::Channel> = h.of(signal).cloned().collect();
        let mut channels = Vec::new();
        for (k, c) in chans.iter().enumerate() {
            match column(k, c) {
                Some(col) => channels.push((self.channel(c, gain, offset, ""), col)),
                None => self.warnings.push(format!("{name}: no data file for {}", c.native)),
            }
        }
        let order = if storage == "rhx per channel" { MemoryOrder::ChannelMajor } else { MemoryOrder::TimeMajor };
        self.add(Stream { name, description, kind, unit, decode, rate, samples, channels, order, storage });
    }

    /// RHS: DC amplifier and stimulation current of each amplifier channel.
    fn rhs_extras(&mut self, samples: u64, mut dc: impl FnMut(usize, &header::Channel) -> Option<Column>, mut stim: impl FnMut(usize, &header::Channel) -> Option<Column>, storage: &'static str) {
        let h = self.h;
        let amps: Vec<header::Channel> = h.of(Signal::Amplifier).cloned().collect();
        let order = if storage == "rhx per channel" { MemoryOrder::ChannelMajor } else { MemoryOrder::TimeMajor };
        // V = −0.01923 · (raw − 512)
        let dc_cols: Vec<_> = amps.iter().enumerate().filter_map(|(k, c)| dc(k, c).map(|col| (self.channel(c, -0.01923, 512.0 * 0.01923, "_DC"), col))).collect();
        self.add(Stream { name: "dc_amplifier", description: "DC amplifier channels (low gain, electrode voltage)".into(), kind: SignalKind::Electrical, unit: "V", decode: Decode::U16, rate: h.sample_rate, samples, channels: dc_cols, order, storage });
        let stim_cols: Vec<_> = amps.iter().enumerate().filter_map(|(k, c)| stim(k, c).map(|col| (self.channel(c, h.stim_step, 0.0, "_STIM"), col))).collect();
        self.add(Stream { name: "stim", description: format!("Stimulation current per channel ({:.4} µA per step)", h.stim_step * 1e6), kind: SignalKind::Other, unit: "A", decode: Decode::Stim, rate: h.sample_rate, samples, channels: stim_cols, order, storage });
    }

    /// One event series per digital line: high periods.
    fn digital(&mut self, signal: Signal, samples: u64, mut column: impl FnMut(&header::Channel) -> Option<(Column, Option<u16>)>) {
        let rate = self.h.sample_rate;
        let kind = if signal == Signal::DigitalIn { "input" } else { "output" };
        for c in self.h.of(signal).cloned().collect::<Vec<_>>() {
            let name = if c.custom.is_empty() { c.native.clone() } else { c.custom.clone() };
            if !self.options.wants(&name) {
                continue;
            }
            let Some((col, bit)) = column(&c) else {
                self.warnings.push(format!("{name}: no data file"));
                continue;
            };
            let periods = high_periods(&col, samples, bit);
            if periods.is_empty() {
                // A line that never went high carries nothing to convert
                continue;
            }
            let end = samples as f64 / rate;
            self.session.events.push(EventSeries {
                name,
                description: format!("Digital {kind} {} (high periods)", c.native),
                onsets: periods.iter().map(|(on, _)| *on as f64 / rate).collect(),
                offsets: Some(periods.iter().map(|(_, off)| off.map_or(end, |o| o as f64 / rate)).collect()),
                values: vec![1.0; periods.len()],
                channels: 1,
                labels: Vec::new(),
            });
        }
    }

    /// Checks that timestamps count up by one; returns the first.
    fn timestamps(&mut self, file: &MappedFile, place: Place) -> Option<i64> {
        if self.samples == 0 {
            return None;
        }
        let signed = self.h.signed_timestamps();
        let at = |s: u64| {
            let v = raw32(file, place, s);
            if signed { v as i32 as i64 } else { v as i64 }
        };
        let (first, last) = (at(0), at(self.samples - 1));
        if last - first != self.samples as i64 - 1 {
            self.warnings.push(format!(
                "timestamps run from {first} to {last} over {} samples: {} samples missing (gaps are not filled; times after a gap are early)",
                self.samples,
                last - first + 1 - self.samples as i64
            ));
        }
        Some(first)
    }

    fn traditional(&mut self, file: Arc<MappedFile>) -> Result<()> {
        let h = self.h;
        let b = h.block() as u64;
        let n = |s: Signal| h.count(s) as u64;
        let (n_amp, n_aux, n_sup, n_adc, n_dac) = (n(Signal::Amplifier), n(Signal::Aux), n(Signal::Supply), n(Signal::AnalogIn), n(Signal::AnalogOut));
        // Field offsets inside a block
        let mut at = 0u64;
        let mut field = |bytes: u64| {
            let start = at;
            at += bytes;
            start
        };
        let ts_at = field(b * 4);
        let amp_at = field(n_amp * b * 2);
        let (mut aux_at, mut sup_at, mut temp_at, mut dc_at, mut stim_at, mut dac_at) = (0, 0, 0, 0, 0, 0);
        match h.kind {
            Kind::Rhd => {
                aux_at = field(n_aux * (b / 4) * 2);
                sup_at = field(n_sup * 2);
                temp_at = field(h.temp_sensors as u64 * 2);
            }
            Kind::Rhs => {
                if h.dc_saved {
                    dc_at = field(n_amp * b * 2);
                }
                stim_at = field(n_amp * b * 2);
            }
        }
        let adc_at = field(n_adc * b * 2);
        if h.kind == Kind::Rhs {
            dac_at = field(n_dac * b * 2);
        }
        let din_at = field(if n(Signal::DigitalIn) > 0 { b * 2 } else { 0 });
        let dout_at = field(if n(Signal::DigitalOut) > 0 { b * 2 } else { 0 });
        let block_bytes = at;
        let data = file.len().saturating_sub(h.size as u64);
        let blocks = data / block_bytes;
        if !data.is_multiple_of(block_bytes) {
            self.warnings.push(format!("the file ends with a partial data block ({} bytes ignored)", data % block_bytes));
        }
        self.samples = blocks * b;
        let start = h.size as u64;
        let col = |field_at: u64, per_block: u64, k: u64| Column { file: file.clone(), place: Place::Blocks { start, block_bytes, per_block, at: field_at + k * per_block * 2 } };
        let first = self.timestamps(&file, Place::Blocks { start, block_bytes, per_block: b, at: ts_at });
        let (sr, samples) = (h.sample_rate, self.samples);
        self.analog(Signal::Amplifier, true, sr, samples, |k, _| Some(col(amp_at, b, k as u64)), "traditional");
        if h.kind == Kind::Rhd {
            self.analog(Signal::Aux, true, sr / 4.0, blocks * (b / 4), |k, _| Some(col(aux_at, b / 4, k as u64)), "traditional");
            self.analog(Signal::Supply, true, sr / b as f64, blocks, |k, _| Some(col(sup_at, 1, k as u64)), "traditional");
            if h.temp_sensors > 0 && self.options.wants("temperature") {
                let channels = (0..h.temp_sensors as u64).map(|k| (ChannelInfo { name: format!("temperature {}", k + 1), gain: 0.01, offset: 0.0 }, col(temp_at, 1, k))).collect();
                self.add(Stream { name: "temperature", description: "Headstage temperature sensors".into(), kind: SignalKind::Other, unit: "degrees C", decode: Decode::I16, rate: sr / b as f64, samples: blocks, channels, order: MemoryOrder::TimeMajor, storage: "traditional" });
            }
        } else {
            let dc_saved = h.dc_saved;
            self.rhs_extras(samples, |k, _| dc_saved.then(|| col(dc_at, b, k as u64)), |k, _| Some(col(stim_at, b, k as u64)), "traditional");
            self.analog(Signal::AnalogOut, true, sr, samples, |k, _| Some(col(dac_at, b, k as u64)), "traditional");
        }
        self.analog(Signal::AnalogIn, true, sr, samples, |k, _| Some(col(adc_at, b, k as u64)), "traditional");
        self.digital(Signal::DigitalIn, samples, |c| Some((col(din_at, b, 0), Some(c.native_order))));
        self.digital(Signal::DigitalOut, samples, |c| Some((col(dout_at, b, 0), Some(c.native_order))));
        if let Some(first) = first {
            self.session.metadata.extra.insert("intan_first_timestamp".into(), first.to_string());
        }
        Ok(())
    }

    fn map(&mut self, path: &Path) -> Option<Arc<MappedFile>> {
        if !path.is_file() {
            return None;
        }
        match MappedFile::open(path) {
            Ok(f) => {
                self.files.push(path.to_path_buf());
                Some(Arc::new(f))
            }
            Err(e) => {
                self.warnings.push(format!("{}: {e}", path.display()));
                None
            }
        }
    }

    /// `time.dat`: the sample count and the first timestamp.
    fn time_file(&mut self, dir: &Path) -> Result<()> {
        let time = self.map(&dir.join("time.dat")).ok_or_else(|| Error::format("intan", format!("{}: time.dat is missing", dir.display())))?;
        self.samples = time.len() / 4;
        if let Some(first) = self.timestamps(&time, Place::Whole) {
            self.session.metadata.extra.insert("intan_first_timestamp".into(), first.to_string());
        }
        Ok(())
    }

    fn per_type(&mut self, dir: &Path) -> Result<()> {
        self.time_file(dir)?;
        let h = self.h;
        let full = self.samples;
        // A file of `columns` interleaved channels: its columns and its rate (aux may be decimated)
        let open = |b: &mut Self, name: &str, columns: usize| -> Option<(Arc<MappedFile>, f64, u64)> {
            if columns == 0 {
                return None;
            }
            let f = b.map(&dir.join(name))?;
            let samples = f.len() / (2 * columns as u64);
            let rate = if full > 0 { h.sample_rate * samples as f64 / full as f64 } else { h.sample_rate };
            Some((f, rate, samples))
        };
        let inter = |f: &Arc<MappedFile>, columns: usize, k: usize| Column { file: f.clone(), place: Place::Interleaved { columns: columns as u64, column: k as u64 } };
        let files: &[(Signal, &str)] = match h.kind {
            Kind::Rhd => &[(Signal::Amplifier, "amplifier.dat"), (Signal::Aux, "auxiliary.dat"), (Signal::Supply, "supply.dat"), (Signal::AnalogIn, "analogin.dat")],
            Kind::Rhs => &[(Signal::Amplifier, "amplifier.dat"), (Signal::AnalogIn, "analogin.dat"), (Signal::AnalogOut, "analogout.dat")],
        };
        for &(signal, name) in files {
            let n = h.count(signal);
            if let Some((f, rate, samples)) = open(self, name, n) {
                self.analog(signal, false, rate, samples, |k, _| Some(inter(&f, n, k)), "rhx per type");
            }
        }
        if h.kind == Kind::Rhs {
            let n = h.count(Signal::Amplifier);
            let dc = open(self, "dcamplifier.dat", n).map(|(f, _, _)| f);
            let stim = open(self, "stim.dat", n).map(|(f, _, _)| f);
            self.rhs_extras(full, |k, _| dc.as_ref().map(|f| inter(f, n, k)), |k, _| stim.as_ref().map(|f| inter(f, n, k)), "rhx per type");
        }
        for (signal, name) in [(Signal::DigitalIn, "digitalin.dat"), (Signal::DigitalOut, "digitalout.dat")] {
            if h.count(signal) == 0 {
                continue;
            }
            let words = open(self, name, 1).map(|(f, _, _)| f);
            self.digital(signal, full, |c| words.as_ref().map(|f| (inter(f, 1, 0), Some(c.native_order))));
        }
        Ok(())
    }

    fn per_channel(&mut self, dir: &Path) -> Result<()> {
        self.time_file(dir)?;
        let h = self.h;
        let full = self.samples;
        let whole = |b: &mut Self, prefix: &str, c: &header::Channel| b.map(&dir.join(format!("{prefix}-{}.dat", c.native))).map(|file| Column { file, place: Place::Whole });
        for (signal, prefix) in [(Signal::Amplifier, "amp"), (Signal::Aux, "aux"), (Signal::Supply, "vdd"), (Signal::AnalogIn, "board"), (Signal::AnalogOut, "board")] {
            let chans: Vec<header::Channel> = h.of(signal).cloned().collect();
            if chans.is_empty() {
                continue;
            }
            let cols: Vec<Option<Column>> = chans.iter().map(|c| whole(self, prefix, c)).collect();
            let samples = cols.iter().flatten().map(|c| c.file.len() / 2).min().unwrap_or(0);
            let rate = if full > 0 { h.sample_rate * samples as f64 / full as f64 } else { h.sample_rate };
            self.analog(signal, false, rate, samples, |k, _| cols[k].clone(), "rhx per channel");
        }
        if h.kind == Kind::Rhs {
            let amps: Vec<header::Channel> = h.of(Signal::Amplifier).cloned().collect();
            let dc: Vec<Option<Column>> = amps.iter().map(|c| whole(self, "dc", c)).collect();
            let stim: Vec<Option<Column>> = amps.iter().map(|c| whole(self, "stim", c)).collect();
            self.rhs_extras(full, |k, _| dc[k].clone(), |k, _| stim[k].clone(), "rhx per channel");
        }
        for signal in [Signal::DigitalIn, Signal::DigitalOut] {
            let chans: Vec<header::Channel> = h.of(signal).cloned().collect();
            let cols: Vec<Option<Column>> = chans.iter().map(|c| whole(self, "board", c)).collect();
            self.digital(signal, full, |c| chans.iter().position(|x| x.native == c.native).and_then(|i| cols[i].clone()).map(|col| (col, None)));
        }
        Ok(())
    }

    fn finish(mut self, name: String, layout: &Layout) -> Result<Session> {
        let h = self.h;
        let model = if h.kind == Kind::Rhd { "RHD" } else { "RHS" };
        // Electrodes: one per amplifier channel, one group per port
        for c in h.of(Signal::Amplifier) {
            let group = format!("port {}", c.group);
            if self.session.electrode_group(&group).is_none() {
                let device = format!("headstage {}", c.group);
                self.session.metadata.devices.push(Device {
                    name: device.clone(),
                    description: format!("Intan {model}2000 headstage on port {}", c.group),
                    manufacturer: Some("Intan Technologies".into()),
                    model: None,
                });
                self.session.electrode_groups.push(ElectrodeGroup { name: group.clone(), description: format!("Electrodes on headstage port {}", c.group), location: "unknown".into(), device: Some(device) });
            }
            let mut channels = Vec::new();
            for rec in ["amplifier", "dc_amplifier"] {
                if let Some(r) = self.session.recording(rec) {
                    // Channels missing a file were skipped: match by name
                    let wanted = self.channel(c, 0.0, 0.0, if rec == "dc_amplifier" { "_DC" } else { "" }).name;
                    if let Some(i) = r.info().channels.iter().position(|x| x.name == wanted) {
                        channels.push(ChannelRef { recording: rec.into(), channel: i });
                    }
                }
            }
            self.session.electrodes.push(Electrode {
                name: if c.custom.is_empty() { c.native.clone() } else { c.custom.clone() },
                group,
                channels,
                impedance_ohms: (c.impedance_ohms > 0.0).then_some(c.impedance_ohms),
                ..Default::default()
            });
        }
        let m = &mut self.session.metadata;
        m.start_time = start_from_name(&name);
        if m.start_time.is_none() {
            self.warnings.push(format!("no start time: {name:?} does not end in _YYMMDD_HHMMSS; set session.start_time"));
        }
        m.experiment = Some(name);
        m.notes = h.notes.clone();
        m.devices.insert(0, Device {
            name: "controller".into(),
            description: format!("Intan {model} acquisition controller"),
            manufacturer: Some("Intan Technologies".into()),
            model: None,
        });
        m.extra.insert("intan_version".into(), format!("{}.{}", h.version.0, h.version.1));
        m.extra.insert("intan_sample_rate".into(), h.sample_rate.to_string());
        m.extra.insert("intan_bandwidth_hz".into(), format!("{}–{}", h.bandwidth.0, h.bandwidth.1));
        if let Some(c) = h.dsp_cutoff {
            m.extra.insert("intan_dsp_cutoff_hz".into(), c.to_string());
        }
        if !h.reference.is_empty() {
            m.extra.insert("intan_reference".into(), h.reference.clone());
        }
        m.extra.insert(
            "intan_layout".into(),
            match layout {
                Layout::Traditional(_) => "traditional",
                Layout::PerType(..) => "one file per signal type",
                Layout::PerChannel(..) => "one file per channel",
            }
            .into(),
        );
        let mut prov = Provenance::new("intan");
        prov.version = Some(format!("{model} {}.{}", h.version.0, h.version.1));
        for f in &self.files {
            prov.add_file(f);
        }
        prov.warnings = self.warnings;
        self.session.provenance = prov;
        Ok(self.session)
    }
}

/// `…_231117_052500` → `2023-11-17T05:25:00` (RHX names files and folders this way).
pub fn start_from_name(name: &str) -> Option<String> {
    let parts: Vec<&str> = name.rsplitn(3, '_').collect();
    let [time, date, ..] = parts.as_slice() else { return None };
    let digits = |s: &str| s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(date) || !digits(time) {
        return None;
    }
    let n = |s: &str, i: usize| s[i..i + 2].parse::<u32>().unwrap_or(0);
    let (mo, d, hh, mm, ss) = (n(date, 2), n(date, 4), n(time, 0), n(time, 2), n(time, 4));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    Some(format!("20{}-{mo:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}", &date[..2]))
}

#[cfg(test)]
mod tests {
    use super::start_from_name;

    #[test]
    fn test_start_from_name() {
        assert_eq!(start_from_name("intan_fps_test_231117_052500").as_deref(), Some("2023-11-17T05:25:00"));
        assert_eq!(start_from_name("intan_rhd_test_1"), None);
        assert_eq!(start_from_name("x_231399_052500"), None);
    }
}
