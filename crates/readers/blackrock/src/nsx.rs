//! `.ns1` … `.ns6`: continuous data at one sample rate per file.
//!
//! - 2.1 (`NEURALSG`): 32-byte header (label, period, channel count), one uint32 electrode id
//!   per channel, then int16 samples (channels interleaved) to the end; no timestamps, no scaling
//!   (the NEV's digitization factors give it).
//! - 2.2 / 2.3 (`NEURALCD`) and 3.0 (`BRSMPGRP`): 314-byte header (spec, header size, label,
//!   comment, period in 1/30 000 s, timestamp resolution, UTC time origin, channel count), a
//!   66-byte `CC` header per channel (electrode id, label, connector bank and pin, digital and
//!   analog ranges, units, filters), then data blocks: uint8 flag (1), timestamp (uint32; 3.0:
//!   uint64), uint32 sample count, samples. A recording pause starts a new block.
//! - 3.0 with PTP (timestamp resolution 1 ns): every block holds one sample with its own
//!   timestamp; runs of samples whose timestamps advance by about one period form the parts.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use nc_base::mapped::MappedFile;
use nc_core::{check_read, Error, Recording, RecordingInfo, Result};

/// A channel of an NSx file.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    /// Electrode id.
    pub id: u32,
    /// Channel label.
    pub label: String,
    /// Front-end bank (1 = A, 2 = B, …; 0 unknown) and pin.
    pub connector: u8,
    /// Pin on the bank.
    pub pin: u8,
    /// Smallest stored value.
    pub min_digital: i16,
    /// Largest stored value.
    pub max_digital: i16,
    /// Analog value of `min_digital`, in `units`.
    pub min_analog: i16,
    /// Analog value of `max_digital`, in `units`.
    pub max_analog: i16,
    /// `uV`, `mV` or `V`.
    pub units: String,
    /// High-pass and low-pass corners (mHz).
    pub high_pass_mhz: u32,
    /// Low-pass corner (mHz).
    pub low_pass_mhz: u32,
}

impl Channel {
    /// (gain, offset) to volts; `None` when the units are unknown or the ranges are empty.
    pub fn scale(&self) -> Option<(f64, f64)> {
        let unit = match self.units.trim() {
            "uV" | "µV" => 1e-6,
            "mV" => 1e-3,
            "V" => 1.0,
            _ => return None,
        };
        let digital = self.max_digital as f64 - self.min_digital as f64;
        if digital == 0.0 {
            return None;
        }
        let gain = (self.max_analog as f64 - self.min_analog as f64) / digital;
        let offset = self.min_analog as f64 - self.min_digital as f64 * gain;
        Some((gain * unit, offset * unit))
    }
}

/// A run of consecutive samples (a data block, or a gap-free run of PTP packets).
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    /// Timestamp of the first sample (ticks of the file's timestamp resolution).
    pub timestamp: u64,
    /// Samples in the part.
    pub samples: u64,
    /// Byte offset of the first sample's first channel.
    pub offset: usize,
}

/// An `.nsX` file: header, channels and data parts, memory-mapped.
#[derive(Debug)]
pub struct Nsx {
    /// The mapped file.
    pub file: Arc<MappedFile>,
    /// `2.1`, `2.2`, `2.3`, `3.0`.
    pub spec: String,
    /// Sampling group label (`30 kS/s`).
    pub label: String,
    /// Sample period in 1/30 000 s.
    pub period: u32,
    /// Timestamp ticks per second.
    pub timestamp_resolution: u64,
    /// UTC `[year, month, weekday, day, hour, minute, second, ms]`.
    pub origin: Option<[u16; 8]>,
    /// Channels, in stored order.
    pub channels: Vec<Channel>,
    /// Bytes from one sample to the next (channels × 2, or a whole PTP packet).
    pub stride: usize,
    /// PTP layout: one packet per sample with its own timestamp.
    pub ptp: bool,
    /// Runs of consecutive samples.
    pub parts: Vec<Part>,
    /// Problems met while reading.
    pub warnings: Vec<String>,
}

fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn i16_at(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

impl Nsx {
    /// Samples per second.
    pub fn sample_rate(&self) -> f64 {
        30_000.0 / self.period.max(1) as f64
    }

    /// Maps the file at `path` and parses its header and data blocks.
    pub fn open(path: &Path) -> Result<Self> {
        let file = Arc::new(MappedFile::open(path)?);
        let mapped = file.clone();
        let b = mapped.bytes();
        let bad = |m: &str| Error::format("blackrock", format!("{}: {m}", path.display()));
        if b.len() < 32 {
            return Err(bad("too short for an NSx header"));
        }
        let id = &b[..8];
        let mut warnings = Vec::new();
        if id == b"NEURALSG" {
            // 2.1: label, period, channel count, electrode ids; samples to the end
            let (period, count) = (u32_at(b, 24), u32_at(b, 28) as usize);
            let header = 32 + 4 * count;
            if b.len() < header {
                return Err(bad("truncated header"));
            }
            let channels = (0..count)
                .map(|i| {
                    let id = u32_at(b, 32 + 4 * i);
                    let label = if id < 129 { format!("chan{id}") } else { format!("ainp{}", id - 128) };
                    Channel { id, label, connector: 0, pin: 0, min_digital: 0, max_digital: 0, min_analog: 0, max_analog: 0, units: String::new(), high_pass_mhz: 0, low_pass_mhz: 0 }
                })
                .collect();
            let stride = 2 * count.max(1);
            let samples = ((b.len() - header) / stride) as u64;
            return Ok(Self {
                file,
                spec: "2.1".into(),
                label: text(&b[8..24]),
                period,
                timestamp_resolution: 30_000,
                origin: None,
                channels,
                stride,
                ptp: false,
                parts: vec![Part { timestamp: 0, samples, offset: header }],
                warnings,
            });
        }
        if id != b"NEURALCD" && id != b"BRSMPGRP" {
            return Err(bad("not an NSx file"));
        }
        if b.len() < 314 {
            return Err(bad("truncated header"));
        }
        let spec = format!("{}.{}", b[8], b[9]);
        let header = u32_at(b, 10) as usize;
        let label = text(&b[14..30]);
        let period = u32_at(b, 286);
        let timestamp_resolution = u32_at(b, 290) as u64;
        let mut origin = [0u16; 8];
        for (k, o) in origin.iter_mut().enumerate() {
            *o = u16_at(b, 294 + 2 * k);
        }
        let count = u32_at(b, 310) as usize;
        if b.len() < 314 + 66 * count || header < 314 + 66 * count {
            return Err(bad("truncated channel headers"));
        }
        let channels: Vec<Channel> = (0..count)
            .map(|i| {
                let c = 314 + 66 * i;
                Channel {
                    id: u16_at(b, c + 2) as u32,
                    label: text(&b[c + 4..c + 20]),
                    connector: b[c + 20],
                    pin: b[c + 21],
                    min_digital: i16_at(b, c + 22),
                    max_digital: i16_at(b, c + 24),
                    min_analog: i16_at(b, c + 26),
                    max_analog: i16_at(b, c + 28),
                    units: text(&b[c + 30..c + 46]),
                    high_pass_mhz: u32_at(b, c + 46),
                    low_pass_mhz: u32_at(b, c + 56),
                }
            })
            .collect();
        let wide = b[8] >= 3;
        let ts_bytes = if wide { 8 } else { 4 };
        let block_header = 1 + ts_bytes + 4;
        let row = 2 * count.max(1);
        let read_ts = |at: usize| if wide { u64_at(b, at) } else { u32_at(b, at) as u64 };

        // PTP: one sample per block, nanosecond timestamps
        let ptp = wide && timestamp_resolution == 1_000_000_000 && b.len() >= header + block_header && u32_at(b, header + 1 + ts_bytes) == 1;
        if ptp {
            let stride = block_header + row;
            let n = (b.len() - header) / stride;
            let rate = 30_000.0 / period.max(1) as f64;
            let limit = (2.0 / rate * 1e9) as u64;
            let mut parts = Vec::new();
            let mut start = 0usize;
            let mut prev = 0u64;
            for k in 0..n {
                let at = header + k * stride;
                if b[at] != 1 || u32_at(b, at + 1 + ts_bytes) != 1 {
                    warnings.push(format!("PTP packet {k} is not a one-sample block; the rest of the file is skipped"));
                    break;
                }
                let ts = read_ts(at + 1);
                if k > start && ts.saturating_sub(prev) > limit {
                    parts.push(Part { timestamp: read_ts(header + start * stride + 1), samples: (k - start) as u64, offset: header + start * stride + block_header });
                    start = k;
                }
                prev = ts;
            }
            if n > start {
                parts.push(Part { timestamp: read_ts(header + start * stride + 1), samples: (n - start) as u64, offset: header + start * stride + block_header });
            }
            if parts.len() > 1 {
                warnings.push(format!("{} gaps in the PTP timestamps (more than two sample periods): one part per gap-free run", parts.len() - 1));
            }
            return Ok(Self { file, spec, label, period, timestamp_resolution, origin: Some(origin), channels, stride, ptp, parts, warnings });
        }

        // Standard blocks
        let mut parts = Vec::new();
        let mut at = header;
        while at + block_header <= b.len() {
            if b[at] != 1 {
                warnings.push(format!("data block header at byte {at} is not 1; the rest of the file is skipped"));
                break;
            }
            let ts = read_ts(at + 1);
            let mut samples = u32_at(b, at + 1 + ts_bytes) as u64;
            let offset = at + block_header;
            let available = ((b.len() - offset) / row) as u64;
            if samples > available {
                warnings.push(format!("the last data block holds {samples} samples but the file ends after {available}"));
                samples = available;
            }
            parts.push(Part { timestamp: ts, samples, offset });
            at = offset + samples as usize * row;
        }
        Ok(Self { file, spec, label, period, timestamp_resolution, origin: Some(origin), channels, stride: row, ptp, parts, warnings })
    }
}

/// Some channels of one part of an NSx file.
pub struct NsxRecording {
    /// Description of the recording.
    pub info: RecordingInfo,
    file: Arc<MappedFile>,
    offset: usize,
    stride: usize,
    /// Column of each channel of this recording.
    columns: Vec<usize>,
}

impl NsxRecording {
    /// Channels `columns` of `part` of `nsx`.
    pub fn new(info: RecordingInfo, nsx: &Nsx, part: &Part, columns: Vec<usize>) -> Self {
        Self { info, file: nsx.file.clone(), offset: part.offset, stride: nsx.stride, columns }
    }

    fn each(&self, channels: &[usize], samples: Range<u64>, mut f: impl FnMut(usize, usize, [u8; 2])) {
        let b = self.file.bytes();
        let cols: Vec<usize> = channels.iter().map(|&c| self.columns[c] * 2).collect();
        for (t, s) in (samples.start as usize..samples.end as usize).enumerate() {
            let base = self.offset + s * self.stride;
            for (i, &c) in cols.iter().enumerate() {
                f(i, t, [b[base + c], b[base + c + 1]]);
            }
        }
    }
}

impl Recording for NsxRecording {
    fn info(&self) -> &RecordingInfo {
        &self.info
    }

    fn read(&self, channels: &[usize], samples: Range<u64>, out: &mut [f32]) -> Result<()> {
        let n = check_read(&self.info, channels, &samples, out.len())?;
        let scale: Vec<(f64, f64)> = channels.iter().map(|&c| (self.info.channels[c].gain, self.info.channels[c].offset)).collect();
        self.each(channels, samples, |i, t, v| out[i * n + t] = (i16::from_le_bytes(v) as f64 * scale[i].0 + scale[i].1) as f32);
        Ok(())
    }

    fn read_stored(&self, channels: &[usize], samples: Range<u64>, out: &mut [u8]) -> Result<bool> {
        let n = check_read(&self.info, channels, &samples, out.len() / 2)?;
        if out.len() != channels.len() * n * 2 {
            return Err(Error::BufferSize { expected: channels.len() * n * 2, actual: out.len() });
        }
        self.each(channels, samples, |i, t, v| {
            let at = (i * n + t) * 2;
            out[at..at + 2].copy_from_slice(&v);
        });
        Ok(true)
    }
}
