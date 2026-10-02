use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::Result;

/// Where the waveforms of a [`SnippetSeries`] are read from. Readers serve them from the source
/// file on demand, so a store with millions of snippets is never held in memory.
pub trait Waveforms: Send + Sync + fmt::Debug {
    /// Number of snippets served.
    fn count(&self) -> usize;

    /// The waveforms of `snippets` (indices), back to back in `out`
    /// (`snippets.len() × samples_per_snippet` values), scaled to the series' unit.
    fn read(&self, snippets: &[usize], out: &mut [f32]) -> Result<()>;
}

/// Waveforms held in memory (tests, small sources): `samples` values per snippet.
#[derive(Debug, Default)]
pub struct MemoryWaveforms {
    samples: usize,
    data: Vec<f32>,
}

impl MemoryWaveforms {
    pub fn new(samples: usize, data: Vec<f32>) -> Self {
        Self { samples, data }
    }
}

impl Waveforms for MemoryWaveforms {
    fn count(&self) -> usize {
        self.data.len().checked_div(self.samples).unwrap_or(0)
    }

    fn read(&self, snippets: &[usize], out: &mut [f32]) -> Result<()> {
        let w = self.samples;
        for (o, &i) in out.chunks_exact_mut(w.max(1)).zip(snippets) {
            let src = self.data.get(i * w..(i + 1) * w).ok_or_else(|| crate::Error::format("snippets", format!("snippet {i} out of range")))?;
            o.copy_from_slice(src);
        }
        Ok(())
    }
}

/// Snippets read per block when streaming waveforms (≈ 2–16 MB for common snippet lengths).
pub const SNIPPET_BLOCK: usize = 65_536;

/// Triggered waveform snippets (e.g. spikes or evoked responses) with per-snippet metadata.
/// Timestamps, channels and sort codes are in memory; waveforms are read on demand.
#[derive(Clone)]
pub struct SnippetSeries {
    pub name: String,
    pub description: String,
    pub sample_rate: f64,
    pub samples_per_snippet: usize,
    /// Seconds from the session start.
    pub timestamps: Vec<f64>,
    /// Source channel of each snippet (numbered as the source numbers them).
    pub channels: Vec<u16>,
    /// Sort codes / unit labels (0 = unsorted).
    pub sort_codes: Vec<u16>,
    /// The waveforms, `samples_per_snippet` values each, scaled to `unit`.
    pub waveforms: Arc<dyn Waveforms>,
    pub unit: String,
    /// Electrode (index into `Session::electrodes`) of each source channel; filled by the reader
    /// or by `MetadataFile::apply`. Channels without one are not written.
    pub electrodes: BTreeMap<u16, usize>,
}

impl Default for SnippetSeries {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            sample_rate: 0.0,
            samples_per_snippet: 0,
            timestamps: Vec::new(),
            channels: Vec::new(),
            sort_codes: Vec::new(),
            waveforms: Arc::new(MemoryWaveforms::default()),
            unit: String::new(),
            electrodes: BTreeMap::new(),
        }
    }
}

impl fmt::Debug for SnippetSeries {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnippetSeries")
            .field("name", &self.name)
            .field("snippets", &self.len())
            .field("samples_per_snippet", &self.samples_per_snippet)
            .field("sample_rate", &self.sample_rate)
            .field("unit", &self.unit)
            .finish_non_exhaustive()
    }
}

impl SnippetSeries {
    pub fn len(&self) -> usize {
        self.timestamps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timestamps.is_empty()
    }

    /// Distinct source channels, ascending.
    pub fn channel_set(&self) -> Vec<u16> {
        let mut c = self.channels.clone();
        c.sort_unstable();
        c.dedup();
        c
    }

    /// Indices of the snippets on `channel`, in time order of the source.
    pub fn on_channel(&self, channel: u16) -> Vec<usize> {
        (0..self.len()).filter(|&i| self.channels[i] == channel).collect()
    }

    /// The waveforms of `snippets`, back to back (allocates; for small selections).
    pub fn read(&self, snippets: &[usize]) -> Result<Vec<f32>> {
        let mut out = vec![0.0; snippets.len() * self.samples_per_snippet];
        self.waveforms.read(snippets, &mut out)?;
        Ok(out)
    }

    /// Calls `f(first, waveforms)` for consecutive blocks of `snippets` (at most
    /// [`SNIPPET_BLOCK`] at a time): `first` is the position in `snippets` of the block's first.
    pub fn for_each_block(&self, snippets: &[usize], mut f: impl FnMut(usize, &[f32]) -> Result<()>) -> Result<()> {
        let mut buf = Vec::new();
        for (k, block) in snippets.chunks(SNIPPET_BLOCK).enumerate() {
            buf.resize(block.len() * self.samples_per_snippet, 0.0);
            self.waveforms.read(block, &mut buf)?;
            f(k * SNIPPET_BLOCK, &buf)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_waveforms_and_blocks() {
        let sn = SnippetSeries {
            samples_per_snippet: 2,
            timestamps: vec![0.0, 1.0, 2.0],
            channels: vec![1, 2, 1],
            waveforms: Arc::new(MemoryWaveforms::new(2, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0])),
            ..Default::default()
        };
        assert_eq!(sn.waveforms.count(), 3);
        assert_eq!(sn.on_channel(1), vec![0, 2]);
        assert_eq!(sn.read(&[2, 0]).unwrap(), vec![4.0, 5.0, 0.0, 1.0]);
        let mut seen = Vec::new();
        sn.for_each_block(&[1, 2], |first, w| {
            seen.push((first, w.to_vec()));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, vec![(0, vec![2.0, 3.0, 4.0, 5.0])]);
        assert!(sn.read(&[3]).is_err());
    }
}
