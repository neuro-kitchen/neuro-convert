//! A `.bin` file: little-endian int16, every saved channel interleaved per sample (time-major).
//! A [`BinRecording`] exposes a subset of its columns (e.g. the neural channels, or the sync word)
//! as one [`Recording`], reading straight from the memory-mapped file.

use std::ops::Range;
use std::sync::Arc;

use nc_base::mapped::MappedFile;
use nc_core::{check_read, Recording, RecordingInfo, Result};

/// A subset of the columns of a `.bin` file (int16, time-major), memory-mapped.
pub struct BinRecording {
    info: RecordingInfo,
    file: Arc<MappedFile>,
    /// Saved channels per sample (file columns).
    columns: usize,
    /// File column of each channel of this recording.
    selected: Vec<usize>,
}

impl BinRecording {
    /// `info.samples` must not exceed the complete samples in `file`.
    pub fn new(info: RecordingInfo, file: Arc<MappedFile>, columns: usize, selected: Vec<usize>) -> Self {
        debug_assert_eq!(info.channels.len(), selected.len());
        Self { info, file, columns, selected }
    }

    /// Calls `f(i, t, sample)` for every requested channel `i` and sample offset `t`, row by row
    /// so the file is read sequentially.
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

impl Recording for BinRecording {
    fn info(&self) -> &RecordingInfo {
        &self.info
    }

    fn read(&self, channels: &[usize], samples: Range<u64>, out: &mut [f32]) -> Result<()> {
        let n = check_read(&self.info, channels, &samples, out.len())?;
        let scale: Vec<(f64, f64)> = channels.iter().map(|&c| (self.info.channels[c].gain, self.info.channels[c].offset)).collect();
        self.each(channels, samples, |i, t, b| {
            let (g, o) = scale[i];
            out[i * n + t] = (i16::from_le_bytes(b) as f64 * g + o) as f32;
        });
        Ok(())
    }

    fn read_stored(&self, channels: &[usize], samples: Range<u64>, out: &mut [u8]) -> Result<bool> {
        let n = check_read(&self.info, channels, &samples, out.len() / 2)?;
        if out.len() != channels.len() * n * 2 {
            return Err(nc_core::Error::BufferSize { expected: channels.len() * n * 2, actual: out.len() });
        }
        self.each(channels, samples, |i, t, b| {
            let at = (i * n + t) * 2;
            out[at..at + 2].copy_from_slice(&b);
        });
        Ok(true)
    }
}
