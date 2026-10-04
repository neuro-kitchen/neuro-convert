//! Where samples are and how they decode. Every Intan sample is 16 bits; what differs is the
//! layout and the encoding:
//! - traditional file: data blocks of 60 / 128 samples; inside a block each field (timestamps,
//!   amplifier, …) holds every channel's samples of the block back to back ([`Place::Blocks`]);
//! - one file per signal type: `amplifier.dat` etc., channels interleaved per sample
//!   ([`Place::Interleaved`]);
//! - one file per channel: `amp-A-000.dat` etc. ([`Place::Whole`]).

use std::ops::Range;
use std::sync::Arc;

use nc_base::mapped::MappedFile;
use nc_core::{check_read, Recording, RecordingInfo, Result};

/// How a stored 16-bit value becomes a number (before the channel's gain and offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decode {
    /// Signed 16-bit.
    I16,
    /// Unsigned 16-bit.
    U16,
    /// Unsigned with a 32768 offset: read as `raw − 32768`, i.e. the int16 of `raw ^ 0x8000`
    /// (kept as int16 so stores keep the native type).
    Offset,
    /// RHS stimulation word: bits 0–7 magnitude, bit 8 negative; flags above are dropped.
    Stim,
}

impl Decode {
    /// The number `raw` stands for.
    pub fn value(self, raw: u16) -> f64 {
        match self {
            Decode::I16 => raw as i16 as f64,
            Decode::U16 => raw as f64,
            Decode::Offset => (raw ^ 0x8000) as i16 as f64,
            Decode::Stim => {
                let magnitude = (raw & 0xFF) as f64;
                if raw & 0x100 != 0 { -magnitude } else { magnitude }
            }
        }
    }
}

/// Where a channel's samples lie in its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Byte `start + block · block_bytes + at + (sample % per_block) · 2`.
    Blocks {
        /// First data block.
        start: u64,
        /// Bytes per block.
        block_bytes: u64,
        /// Samples per block.
        per_block: u64,
        /// Offset of this channel's run within a block.
        at: u64,
    },
    /// Column `column` of `columns` interleaved 16-bit values per sample.
    Interleaved {
        /// Values per sample.
        columns: u64,
        /// This channel's column.
        column: u64,
    },
    /// The whole file is this channel.
    Whole,
}

/// One channel's samples.
#[derive(Clone)]
pub struct Column {
    /// The file holding the samples.
    pub file: Arc<MappedFile>,
    /// Where they lie in it.
    pub place: Place,
}

impl Column {
    /// Byte offset of `sample`.
    pub fn byte(&self, sample: u64) -> usize {
        (match self.place {
            Place::Blocks { start, block_bytes, per_block, at } => start + (sample / per_block) * block_bytes + at + (sample % per_block) * 2,
            Place::Interleaved { columns, column } => (sample * columns + column) * 2,
            Place::Whole => sample * 2,
        }) as usize
    }

    /// The stored 16-bit value of `sample`.
    pub fn raw(&self, sample: u64) -> u16 {
        let b = self.file.bytes();
        let i = self.byte(sample);
        u16::from_le_bytes([b[i], b[i + 1]])
    }
}

/// 32-bit values (timestamps) at the same kind of place, 4 bytes each.
pub fn raw32(file: &MappedFile, place: Place, sample: u64) -> u32 {
    let i = match place {
        Place::Blocks { start, block_bytes, per_block, at } => start + (sample / per_block) * block_bytes + at + (sample % per_block) * 4,
        Place::Interleaved { columns, column } => (sample * columns + column) * 4,
        Place::Whole => sample * 4,
    } as usize;
    let b = file.bytes();
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// A stream of channels with one layout and one encoding.
pub struct IntanRecording {
    info: RecordingInfo,
    columns: Vec<Column>,
    decode: Decode,
}

impl IntanRecording {
    /// A recording of `columns` (one per channel of `info`), decoded with `decode`.
    pub fn new(info: RecordingInfo, columns: Vec<Column>, decode: Decode) -> Self {
        debug_assert_eq!(info.channels.len(), columns.len());
        Self { info, columns, decode }
    }
}

impl Recording for IntanRecording {
    fn info(&self) -> &RecordingInfo {
        &self.info
    }

    fn read(&self, channels: &[usize], samples: Range<u64>, out: &mut [f32]) -> Result<()> {
        let n = check_read(&self.info, channels, &samples, out.len())?;
        for (i, &c) in channels.iter().enumerate() {
            let (col, ch) = (&self.columns[c], &self.info.channels[c]);
            for (t, s) in samples.clone().enumerate() {
                out[i * n + t] = (self.decode.value(col.raw(s)) * ch.gain + ch.offset) as f32;
            }
        }
        Ok(())
    }

    fn read_stored(&self, channels: &[usize], samples: Range<u64>, out: &mut [u8]) -> Result<bool> {
        if self.decode == Decode::Stim {
            return Ok(false);
        }
        let n = check_read(&self.info, channels, &samples, out.len() / 2)?;
        if out.len() != channels.len() * n * 2 {
            return Err(nc_core::Error::BufferSize { expected: channels.len() * n * 2, actual: out.len() });
        }
        let flip = if self.decode == Decode::Offset { 0x8000 } else { 0 };
        for (i, &c) in channels.iter().enumerate() {
            let col = &self.columns[c];
            for (t, s) in samples.clone().enumerate() {
                let at = (i * n + t) * 2;
                out[at..at + 2].copy_from_slice(&(col.raw(s) ^ flip).to_le_bytes());
            }
        }
        Ok(true)
    }
}

/// High periods of `bit` (or of nonzero values when `bit` is `None`) over `samples`: onset and
/// offset sample indices; a period still high at the end has no offset (`None`).
pub fn high_periods(col: &Column, samples: u64, bit: Option<u16>) -> Vec<(u64, Option<u64>)> {
    let high = |s: u64| {
        let v = col.raw(s);
        match bit {
            Some(b) => v & (1 << b) != 0,
            None => v != 0,
        }
    };
    let mut out = Vec::new();
    let mut since: Option<u64> = None;
    for s in 0..samples {
        match (high(s), since) {
            (true, None) => since = Some(s),
            (false, Some(on)) => {
                out.push((on, Some(s)));
                since = None;
            }
            _ => {}
        }
    }
    if let Some(on) = since {
        out.push((on, None));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode() {
        assert_eq!(Decode::Offset.value(32768), 0.0);
        assert_eq!(Decode::Offset.value(32767), -1.0);
        assert_eq!(Decode::Offset.value(0), -32768.0);
        assert_eq!(Decode::I16.value(0xFFFF), -1.0);
        assert_eq!(Decode::U16.value(0xFFFF), 65535.0);
        // magnitude 5, negative, with the compliance flag set
        assert_eq!(Decode::Stim.value(0x8000 | 0x100 | 5), -5.0);
        assert_eq!(Decode::Stim.value(7), 7.0);
    }

    #[test]
    fn test_places() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/intan-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("places.dat");
        let words: Vec<u16> = (0..64).collect();
        std::fs::write(&path, words.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        let file = Arc::new(MappedFile::open(&path).unwrap());
        // Blocks of 32 bytes; this field starts at byte 8 and holds 4 samples per block
        let blocks = Column { file: file.clone(), place: Place::Blocks { start: 0, block_bytes: 32, per_block: 4, at: 8 } };
        assert_eq!([blocks.raw(0), blocks.raw(3), blocks.raw(4), blocks.raw(5)], [4, 7, 20, 21]);
        let inter = Column { file: file.clone(), place: Place::Interleaved { columns: 4, column: 1 } };
        assert_eq!([inter.raw(0), inter.raw(2)], [1, 9]);
        // Column 0 holds 0, 4, 8, 12: bit 2 is high at samples 1 and 3
        let first = Column { file: file.clone(), place: Place::Interleaved { columns: 4, column: 0 } };
        assert_eq!(high_periods(&first, 4, Some(2)), vec![(1, Some(2)), (3, None)]);
        assert_eq!(high_periods(&Column { file, place: Place::Whole }, 3, None), vec![(1, None)]);
    }
}
