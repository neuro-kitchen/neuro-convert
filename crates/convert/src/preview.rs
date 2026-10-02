//! Data for signal previews: a min / max envelope per screen column, so a window of any length
//! draws as one vertical line per pixel column without reading more than a bounded chunk at a
//! time.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use nc_core::{Error, Recording, Result};

/// Samples read per chunk (all requested channels), bounding memory.
const CHUNK: u64 = 1 << 16;

/// `[channel][column]` min / max pairs.
pub type Envelope = Vec<Vec<(f32, f32)>>;

/// Min and max of each of `columns` equal slices of `samples`, per channel (`[channel][column]`).
/// With fewer samples than columns each sample is its own column. Values are scaled as
/// [`Recording::read`] returns them. `cancel`, when set, stops between chunks.
pub fn envelope(rec: &dyn Recording, channels: &[usize], samples: Range<u64>, columns: usize, cancel: Option<&AtomicBool>) -> Result<Envelope> {
    // Many channels: split them over threads (each reads its own channels)
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1)).clamp(1, 8);
    if channels.len() >= 2 * PER_THREAD && threads > 1 {
        let per = channels.len().div_ceil(threads).max(PER_THREAD);
        let parts: Vec<Result<Envelope>> = std::thread::scope(|scope| {
            let handles: Vec<_> = channels.chunks(per).map(|part| scope.spawn(|| envelope_of(rec, part, samples.clone(), columns, cancel))).collect();
            handles.into_iter().map(|h| h.join().expect("envelope thread")).collect()
        });
        let mut out = Vec::with_capacity(channels.len());
        for p in parts {
            out.extend(p?);
        }
        return Ok(out);
    }
    envelope_of(rec, channels, samples, columns, cancel)
}

/// Fewest channels a thread takes in [`envelope`].
const PER_THREAD: usize = 16;

fn envelope_of(rec: &dyn Recording, channels: &[usize], samples: Range<u64>, columns: usize, cancel: Option<&AtomicBool>) -> Result<Envelope> {
    let n = samples.end.saturating_sub(samples.start);
    let columns = columns.min(n as usize).max(usize::from(n > 0));
    let mut out = vec![vec![(f32::INFINITY, f32::NEG_INFINITY); columns]; channels.len()];
    if columns == 0 || channels.is_empty() {
        return Ok(out);
    }
    // Sample offset → column: floor(offset * columns / n); first offset of a column: ceil(c * n / columns)
    let column_of = |offset: u64| ((offset as u128 * columns as u128) / n as u128) as usize;
    let first_of = |c: usize| ((c as u128 * n as u128).div_ceil(columns as u128)) as u64;
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
        let offset0 = s0 - samples.start;
        for (ci, values) in buf.chunks_exact(len).enumerate() {
            let lane = &mut out[ci];
            // Walk the columns instead of dividing per sample
            let mut col = column_of(offset0);
            let mut next = first_of(col + 1);
            for (t, &v) in values.iter().enumerate() {
                let offset = offset0 + t as u64;
                while offset >= next {
                    col += 1;
                    next = first_of(col + 1);
                }
                if v.is_nan() {
                    continue;
                }
                let cell = &mut lane[col];
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
    fn test_threads_match_one_thread() {
        let (c, n) = (40usize, 5_000usize);
        let data: Vec<f32> = (0..c * n).map(|i| ((i * 31) % 997) as f32).collect();
        let rec = MemoryRecording::new("r", data, c, 1000.0, "V").unwrap();
        let channels: Vec<usize> = (0..c).rev().collect();
        assert_eq!(envelope(&rec, &channels, 100..4_900, 700, None).unwrap(), envelope_of(&rec, &channels, 100..4_900, 700, None).unwrap());
    }

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

/// Speed of preview reads on the real 384-channel Neuropixels file (skipped without it). Run with
/// `cargo test --release -p nc-convert preview_speed -- --ignored --nocapture`.
#[cfg(all(test, feature = "spikeglx"))]
mod speed {
    use std::time::Instant;

    use nc_core::{OpenOptions, Reader};

    #[test]
    #[ignore = "timing on real data"]
    fn preview_speed() {
        let dir = std::env::var_os("NC_DATA_DIR").map_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"), Into::into);
        let path = dir.join("raw/spikeglx/imec_385_100s");
        if !path.exists() {
            eprintln!("skipped: {} not found", path.display());
            return;
        }
        let s = nc_spikeglx::SpikeGlx.open(&path, &OpenOptions::default()).unwrap();
        let ap = s.recording("imec0.ap").unwrap();
        let all: Vec<usize> = (0..384).collect();
        for (label, channels, seconds) in [("384 ch, 1 s", &all[..], 1u64), ("64 ch, 3 s (one fetch of the app)", &all[..64], 3), ("16 ch, 3 s", &all[..16], 3)] {
            // Second pass: the file is in the page cache, as while browsing
            let mut ms = 0.0;
            for _ in 0..2 {
                let t = Instant::now();
                super::envelope(ap.as_ref(), channels, 1_500_000..1_500_000 + seconds * 30_000, 3600, None).unwrap();
                ms = t.elapsed().as_secs_f64() * 1e3;
            }
            println!("{label}: {ms:.1} ms");
        }
    }
}
