//! `.ncs` continuous channels: after the header, 1044-byte records of uint64 timestamp (µs),
//! uint32 channel, uint32 sample rate, uint32 valid samples, 512 int16 samples.
//!
//! Records are grouped into gap-free sections as neo does: a record starting more than the
//! tolerance away from where the previous one predicts (its start + valid samples / rate) begins
//! a new section. The sample rate is the stated one (BML / Atlas), whole microseconds per sample
//! (Cheetah < 4), or measured from the longest section's timestamps (Digital Lynx and later:
//! the hardware clock drifts from the rounded stated rate).

use std::ops::Range;
use std::sync::Arc;

use nc_base::mapped::MappedFile;
use nc_core::{check_read, Error, Recording, RecordingInfo, Result};

use crate::header::{self, Acquisition, Header};

/// Bytes per `.ncs` record: µs timestamp, channel, rate, valid count, 512 samples.
pub const RECORD: usize = 8 + 4 + 4 + 4 + 512 * 2;
/// Samples per record.
pub const BLOCK: usize = 512;

/// Records `first..=last` with contiguous samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    /// First record.
    pub first: usize,
    /// Last record (inclusive).
    pub last: usize,
    /// µs of the first sample.
    pub start: u64,
    /// Valid samples in the section.
    pub samples: u64,
}

/// One `.ncs` file, memory-mapped.
#[derive(Debug)]
pub struct NcsFile {
    /// The mapped file.
    pub file: Arc<MappedFile>,
    /// Its header.
    pub header: Header,
    /// Number of whole records.
    pub records: usize,
}

impl NcsFile {
    /// Maps the file at `path` and reads its header.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let file = Arc::new(MappedFile::open(path)?);
        let header = Header::parse(file.bytes());
        let records = file.bytes().len().saturating_sub(header::SIZE) / RECORD;
        Ok(Self { file, header, records })
    }

    fn at(&self, k: usize) -> usize {
        header::SIZE + k * RECORD
    }

    /// µs timestamp of record `k`.
    pub fn timestamp(&self, k: usize) -> u64 {
        let a = self.at(k);
        u64::from_le_bytes(self.file.bytes()[a..a + 8].try_into().expect("8 bytes"))
    }

    /// Valid samples in record `k` (at most [`BLOCK`]).
    pub fn valid(&self, k: usize) -> u32 {
        let a = self.at(k) + 16;
        u32::from_le_bytes(self.file.bytes()[a..a + 4].try_into().expect("4 bytes")).min(BLOCK as u32)
    }

    fn channel(&self, k: usize) -> u32 {
        let a = self.at(k) + 8;
        u32::from_le_bytes(self.file.bytes()[a..a + 4].try_into().expect("4 bytes"))
    }

    fn record_rate(&self, k: usize) -> u32 {
        let a = self.at(k) + 12;
        u32::from_le_bytes(self.file.bytes()[a..a + 4].try_into().expect("4 bytes"))
    }

    /// Gap-free sections and the sample rate used (neo's `NcsSectionsFactory`).
    pub fn sections(&self) -> (Vec<Section>, f64) {
        let n = self.records;
        let stated = self.header.sample_rate().unwrap_or_else(|| if n > 0 { self.record_rate(0) as f64 } else { 1.0 }).max(1e-9);
        if n == 0 {
            return (Vec::new(), stated);
        }
        let acq = self.header.acquisition();
        let (freq, tolerance) = match acq {
            Acquisition::Pre4 => (1e6 / (1e6 / stated).floor(), 0i64),
            Acquisition::Stated => (stated, 0),
            Acquisition::Measured => (stated, (0.2 * 1e6 / stated).round() as i64),
        };
        let predicted_last = (self.timestamp(0) as f64 + 1e6 / freq * (BLOCK * (n - 1)) as f64).round() as u64;
        let mut sections = Vec::new();
        if self.channel(0) == self.channel(n - 1) && self.record_rate(0) == self.record_rate(n - 1) && self.timestamp(n - 1) == predicted_last {
            sections.push(Section { first: 0, last: n - 1, start: self.timestamp(0), samples: (BLOCK * (n - 1)) as u64 + self.valid(n - 1) as u64 });
        } else {
            let mut first = 0;
            for k in 0..n {
                let gap = k + 1 < n && {
                    let delta = self.timestamp(k + 1) as i64 - self.timestamp(k) as i64;
                    let predicted = (self.valid(k) as f64 / freq * 1e6) as i64;
                    (delta - predicted).abs() > tolerance
                };
                if gap || k + 1 == n {
                    let samples = (first..=k).map(|r| self.valid(r) as u64).sum();
                    sections.push(Section { first, last: k, start: self.timestamp(first), samples });
                    first = k + 1;
                }
            }
        }
        let rate = match acq {
            Acquisition::Measured => {
                let s = sections.iter().max_by_key(|s| s.last - s.first).copied().expect("one section");
                if s.last != s.first {
                    (s.samples - self.valid(s.last) as u64) as f64 * 1e6 / (self.timestamp(s.last) - s.start) as f64
                } else {
                    freq
                }
            }
            _ => freq,
        };
        (sections, rate)
    }
}

/// One section of a stream of `.ncs` channels (one file per channel).
pub struct NcsRecording {
    /// Description of the stream section.
    pub info: RecordingInfo,
    files: Vec<Arc<MappedFile>>,
    section: Section,
    /// Cumulative valid samples before each record of the section, when some record before the
    /// last is not full (else samples map to records directly).
    offsets: Option<Vec<u64>>,
}

impl NcsRecording {
    /// A recording over `section` of `files` (one per channel); `reference` gives the valid counts.
    pub fn new(info: RecordingInfo, files: Vec<Arc<MappedFile>>, reference: &NcsFile, section: Section) -> Self {
        let full = (section.first..section.last).all(|k| reference.valid(k) as usize == BLOCK);
        let offsets = (!full).then(|| {
            let mut acc = 0;
            (section.first..=section.last)
                .map(|k| {
                    let at = acc;
                    acc += reference.valid(k) as u64;
                    at
                })
                .collect()
        });
        Self { info, files, section, offsets }
    }

    /// File byte offset of sample `s` of the section.
    fn locate(&self, s: u64) -> usize {
        let (record, within) = match &self.offsets {
            None => (s as usize / BLOCK, s as usize % BLOCK),
            Some(o) => {
                let r = o.partition_point(|&x| x <= s) - 1;
                (r, (s - o[r]) as usize)
            }
        };
        header::SIZE + (self.section.first + record) * RECORD + 20 + within * 2
    }

    fn each(&self, channels: &[usize], samples: Range<u64>, mut f: impl FnMut(usize, usize, [u8; 2])) {
        for (i, &c) in channels.iter().enumerate() {
            let b = self.files[c].bytes();
            for (t, s) in (samples.start..samples.end).enumerate() {
                let at = self.locate(s);
                f(i, t, [b[at], b[at + 1]]);
            }
        }
    }
}

impl Recording for NcsRecording {
    fn info(&self) -> &RecordingInfo {
        &self.info
    }

    fn read(&self, channels: &[usize], samples: Range<u64>, out: &mut [f32]) -> Result<()> {
        let n = check_read(&self.info, channels, &samples, out.len())?;
        let gains: Vec<f64> = channels.iter().map(|&c| self.info.channels[c].gain).collect();
        self.each(channels, samples, |i, t, v| out[i * n + t] = (i16::from_le_bytes(v) as f64 * gains[i]) as f32);
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
