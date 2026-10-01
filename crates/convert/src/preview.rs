//! Data for signal previews: a min / max envelope per screen column, so a window of any length
//! draws as one vertical line per pixel column without reading more than a bounded chunk at a
//! time.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use nc_core::{Error, Recording, Result};

/// Samples read per chunk (all requested channels), bounding memory.
const CHUNK: u64 = 1 << 16;

/// Min and max of each of `columns` equal slices of `samples`, per channel (`[channel][column]`).
/// With fewer samples than columns each sample is its own column. Values are scaled as
/// [`Recording::read`] returns them. `cancel`, when set, stops between chunks.
pub fn envelope(rec: &dyn Recording, channels: &[usize], samples: Range<u64>, columns: usize, cancel: Option<&AtomicBool>) -> Result<Vec<Vec<(f32, f32)>>> {
    let n = samples.end.saturating_sub(samples.start);
    let columns = columns.min(n as usize).max(usize::from(n > 0));
    let mut out = vec![vec![(f32::INFINITY, f32::NEG_INFINITY); columns]; channels.len()];
    if columns == 0 || channels.is_empty() {
        return Ok(out);
    }
    // Sample offset → column: floor(offset * columns / n)
    let column_of = |offset: u64| ((offset as u128 * columns as u128) / n as u128) as usize;
    let mut buf = Vec::new();
    let mut s0 = samples.start;
    while s0 < samples.end {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(Error::Cancelled);
        }
        let s1 = (s0 + CHUNK).min(samples.end);
        let len = (s1 - s0) as usize;
        buf.resize(channels.len() * len, 0.0);
        rec.read(channels, s0..s1, &mut buf)?;
        for (ci, values) in buf.chunks_exact(len).enumerate() {
            for (t, &v) in values.iter().enumerate() {
                if v.is_nan() {
                    continue;
                }
                let cell = &mut out[ci][column_of(s0 - samples.start + t as u64)];
                cell.0 = cell.0.min(v);
                cell.1 = cell.1.max(v);
            }
        }
        s0 = s1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc_core::MemoryRecording;

    #[test]
    fn test_envelope_matches_brute_force() {
        // 2 channels, 200 000 samples (several chunks), values with a known shape
        let n = 200_000usize;
        let data: Vec<f32> = (0..2).flat_map(|c| (0..n).map(move |t| ((t * 7919 + c * 13) % 1000) as f32 - 500.0)).collect();
        let rec = MemoryRecording::new("r", data.clone(), 2, 1000.0, "V").unwrap();
        let (range, columns) = (12_345u64..187_654u64, 333usize);
        let env = envelope(&rec, &[1, 0], range.clone(), columns, None).unwrap();
        let len = (range.end - range.start) as usize;
        for (i, &c) in [1usize, 0].iter().enumerate() {
            let values = &data[c * n + range.start as usize..c * n + range.end as usize];
            for col in [0, 1, 166, 332] {
                let (a, b) = (col * len / columns, ((col + 1) * len).div_ceil(columns).min(len));
                // Columns are floor(offset * columns / len): recompute exactly
                let cell: Vec<f32> = (0..len).filter(|&o| o * columns / len == col).map(|o| values[o]).collect();
                assert!(!cell.is_empty() && a <= b);
                let want = (cell.iter().copied().fold(f32::INFINITY, f32::min), cell.iter().copied().fold(f32::NEG_INFINITY, f32::max));
                assert_eq!(env[i][col], want, "channel {c} column {col}");
            }
        }
    }

    #[test]
    fn test_short_ranges_and_cancel() {
        let rec = MemoryRecording::new("r", vec![1.0, 2.0, 3.0], 1, 10.0, "V").unwrap();
        assert_eq!(envelope(&rec, &[0], 0..3, 100, None).unwrap(), vec![vec![(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]]);
        assert_eq!(envelope(&rec, &[0], 1..1, 100, None).unwrap(), vec![Vec::<(f32, f32)>::new()]);
        let stop = AtomicBool::new(true);
        assert!(matches!(envelope(&rec, &[0], 0..3, 2, Some(&stop)), Err(Error::Cancelled)));
    }
}
