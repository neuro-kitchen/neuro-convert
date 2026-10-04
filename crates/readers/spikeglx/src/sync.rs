//! Digital lines of a `.bin`: TTL transitions (every change of a digital word, full scan) and the
//! sync pulse (a slow square wave shared by every stream of a run), and the clock fit that maps
//! one stream's sync edges onto another's.
//!
//! SpikeGLX sources of the sync pulse: imec probes and OneBox, bit 6 of the `SY` word; NI-DAQ, a
//! digital line (`syncNiChanType=0`, line `syncNiChan`) or an analog channel above a threshold
//! (`syncNiChanType=1`, `syncNiThresh` volts). Edges are found by sampling every few
//! milliseconds and bisecting each change, so even an hour of AP data costs only a few
//! thousand reads; pulses shorter than the step would be missed, which the sync wave (≥ 0.5 s
//! high / low) never is.

use std::collections::BTreeMap;

use nc_base::mapped::MappedFile;

/// Bit of the imec / OneBox `SY` word that carries the sync pulse.
pub const SYNC_BIT: u32 = 6;

/// A two-state signal in one column of a `.bin`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Level {
    /// Bit `bit` of the int16 word in `column`.
    Bit {
        /// Column of the `.bin`.
        column: usize,
        /// Bit of the word.
        bit: u32,
    },
    /// The int16 value of `column` above `threshold` (raw units).
    Above {
        /// Column of the `.bin`.
        column: usize,
        /// Raw value above which the level is high.
        threshold: f64,
    },
}

/// The int16 columns of a `.bin`, sample by sample.
pub struct Columns<'a> {
    bytes: &'a [u8],
    columns: usize,
    /// Samples (rows) in the file.
    pub samples: u64,
}

impl<'a> Columns<'a> {
    /// The `columns`-wide int16 rows of `file`.
    pub fn new(file: &'a MappedFile, columns: usize) -> Self {
        let bytes = file.bytes();
        Self { bytes, columns, samples: (bytes.len() / (2 * columns.max(1))) as u64 }
    }

    /// The int16 value of `column` at sample `t`.
    pub fn word(&self, column: usize, t: u64) -> i16 {
        let at = (t as usize * self.columns + column) * 2;
        i16::from_le_bytes([self.bytes[at], self.bytes[at + 1]])
    }

    /// `true` when `level` is high at sample `t`.
    pub fn level(&self, level: Level, t: u64) -> bool {
        match level {
            Level::Bit { column, bit } => (self.word(column, t) as u16 >> bit) & 1 == 1,
            Level::Above { column, threshold } => self.word(column, t) as f64 > threshold,
        }
    }
}

/// Rising edges (sample of the first high sample) of a square wave whose high and low phases
/// are longer than `step` samples.
pub fn square_edges(c: &Columns, level: Level, step: u64) -> Vec<u64> {
    let step = step.max(1);
    let mut out = Vec::new();
    if c.samples == 0 {
        return out;
    }
    let (mut a, mut va) = (0, c.level(level, 0));
    while a + 1 < c.samples {
        let b = (a + step).min(c.samples - 1);
        let vb = c.level(level, b);
        if vb != va {
            // First sample in (a, b] whose level differs from va
            let (mut lo, mut hi) = (a, b);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if c.level(level, mid) == va {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            if vb {
                out.push(hi);
            }
        }
        (a, va) = (b, vb);
    }
    out
}

/// Every change of the 16 lines of the word in `column` (full scan): per line that changes,
/// its rising and falling samples.
pub fn transitions(c: &Columns, column: usize) -> BTreeMap<u32, (Vec<u64>, Vec<u64>)> {
    let mut out: BTreeMap<u32, (Vec<u64>, Vec<u64>)> = BTreeMap::new();
    if c.samples == 0 {
        return out;
    }
    let mut prev = c.word(column, 0) as u16;
    // Lines high from the first sample start a high period there
    for bit in (0..16).filter(|b| (prev >> b) & 1 == 1) {
        out.entry(bit).or_default().0.push(0);
    }
    for t in 1..c.samples {
        let w = c.word(column, t) as u16;
        let mut changed = w ^ prev;
        while changed != 0 {
            let bit = changed.trailing_zeros();
            let e = out.entry(bit).or_default();
            if (w >> bit) & 1 == 1 {
                e.0.push(t);
            } else {
                e.1.push(t);
            }
            changed &= changed - 1;
        }
        prev = w;
    }
    // A line that is high from the start and never changes is not a TTL line
    out.retain(|_, (rise, fall)| !(rise == &[0] && fall.is_empty()));
    out
}

/// High periods `(onset, offset)` from rising and falling samples; a period still open at the
/// end closes at `end`.
pub fn periods(rising: &[u64], falling: &[u64], end: u64) -> Vec<(u64, u64)> {
    let mut out = Vec::with_capacity(rising.len());
    let mut f = falling.iter().peekable();
    for (i, &r) in rising.iter().enumerate() {
        while f.next_if(|&&x| x <= r).is_some() {}
        let next_rise = rising.get(i + 1).copied().unwrap_or(u64::MAX);
        match f.peek() {
            Some(&&x) if x < next_rise => {
                out.push((r, x));
                f.next();
            }
            _ => out.push((r, end)),
        }
    }
    out
}

/// A clock fit: reference time = `scale` × stream time + `offset` (seconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Reference seconds per stream second.
    pub scale: f64,
    /// Reference time of stream time 0, in seconds.
    pub offset: f64,
    /// Edges matched.
    pub edges: usize,
    /// Largest |residual| (s).
    pub residual: f64,
}

/// Matches every stream edge (seconds) to the nearest reference edge within `tolerance` and
/// fits a line through the pairs; `None` with fewer than two pairs.
pub fn fit(stream: &[f64], reference: &[f64], tolerance: f64) -> Option<Fit> {
    let pairs: Vec<(f64, f64)> = stream
        .iter()
        .filter_map(|&s| {
            let i = reference.partition_point(|&r| r < s);
            let near = [i.checked_sub(1), Some(i)].into_iter().flatten().filter_map(|j| reference.get(j)).min_by(|a, b| (*a - s).abs().total_cmp(&(*b - s).abs()))?;
            ((near - s).abs() <= tolerance).then_some((s, *near))
        })
        .collect();
    let n = pairs.len();
    if n < 2 {
        return None;
    }
    let (mx, my) = (pairs.iter().map(|p| p.0).sum::<f64>() / n as f64, pairs.iter().map(|p| p.1).sum::<f64>() / n as f64);
    let sxx: f64 = pairs.iter().map(|p| (p.0 - mx).powi(2)).sum();
    if sxx <= 0.0 {
        return None;
    }
    let sxy: f64 = pairs.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let scale = sxy / sxx;
    let offset = my - scale * mx;
    let residual = pairs.iter().map(|p| (p.1 - (scale * p.0 + offset)).abs()).fold(0.0, f64::max);
    Some(Fit { scale, offset, edges: n, residual })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(words: &[[i16; 2]]) -> MappedFile {
        let dir = std::env::temp_dir().join(format!("nc-sglx-sync-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.bin", words.len()));
        std::fs::write(&path, words.iter().flatten().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()).unwrap();
        MappedFile::open(&path).unwrap()
    }

    #[test]
    fn test_edges_transitions_and_fit() {
        // Column 1: bit 6 is a square wave with period 20 (high 10..20, 30..40, …); bit 0 has two
        // short pulses (3..5, 50..51) the coarse scan would miss
        let words: Vec<[i16; 2]> = (0..60u64)
            .map(|t| {
                let sync = if (t / 10) % 2 == 1 { 1 << 6 } else { 0 };
                let ttl = i16::from((3..5).contains(&t) || t == 50);
                [0, sync | ttl]
            })
            .collect();
        let f = file(&words);
        let c = Columns::new(&f, 2);
        assert_eq!(square_edges(&c, Level::Bit { column: 1, bit: SYNC_BIT }, 4), vec![10, 30, 50]);
        let lines = transitions(&c, 1);
        assert_eq!(lines[&0], (vec![3, 50], vec![5, 51]));
        assert_eq!(lines[&6].0, vec![10, 30, 50]);
        assert_eq!(periods(&lines[&0].0, &lines[&0].1, 60), vec![(3, 5), (50, 51)]);
        assert_eq!(periods(&[0, 10], &[5], 20), vec![(0, 5), (10, 20)], "open at the end");

        // Stream clock 10 ppm fast and 2 ms late: reference = 1.00001 × t − 0.002
        let reference: Vec<f64> = (1..=20).map(|k| k as f64).collect();
        let stream: Vec<f64> = reference.iter().map(|r| (r + 0.002) / 1.00001).collect();
        let fit = fit(&stream, &reference, 0.25).unwrap();
        assert_eq!(fit.edges, 20);
        assert!((fit.scale - 1.00001).abs() < 1e-9 && (fit.offset + 0.002).abs() < 1e-7 && fit.residual < 1e-9, "{fit:?}");
        assert!(super::fit(&[0.5], &reference, 0.25).is_none());
    }
}
