//! Snip stores → [`SnippetSeries`]: each record points at one waveform in the TEV.
//! Sort codes come from the TSQ, or from an offline sort (`sort::load`) indexed by record.
//! Waveforms stay in the TEV ([`TevWaveforms`] keeps one offset per snippet) and are decoded
//! when written.

use std::sync::Arc;

use super::codes;
use super::tsq::{session_time, StoreIndex};
use nc_base::codec::decode_into;
use nc_base::mapped::MappedFile;
use nc_core::{Error, Result, SampleType, SnippetSeries, Waveforms};

/// Snippet waveforms in the TEV: `points` samples of `ty` at each offset.
pub struct TevWaveforms {
    tev: Arc<MappedFile>,
    ty: SampleType,
    points: usize,
    offsets: Vec<u64>,
}

impl std::fmt::Debug for TevWaveforms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TevWaveforms").field("ty", &self.ty).field("points", &self.points).field("snippets", &self.offsets.len()).finish()
    }
}

impl Waveforms for TevWaveforms {
    fn count(&self) -> usize {
        self.offsets.len()
    }

    fn read(&self, snippets: &[usize], out: &mut [f32]) -> Result<()> {
        let (bytes, len) = (self.tev.bytes(), self.points * self.ty.bytes());
        for (o, &i) in out.chunks_exact_mut(self.points.max(1)).zip(snippets) {
            let at = *self.offsets.get(i).ok_or_else(|| Error::format("tdt", format!("snippet {i} out of range")))? as usize;
            // Offsets were checked against the file when the store was indexed
            decode_into(self.ty, &bytes[at..at + len], o, 1.0, 0.0);
        }
        Ok(())
    }
}

pub fn build(store: &StoreIndex, tev: &Arc<MappedFile>, block_start: f64, sort: Option<(&str, &[u8])>, warnings: &mut Vec<String>) -> Result<SnippetSeries> {
    let ty = codes::sample_type(store.format)
        .ok_or_else(|| Error::Unsupported(format!("TDT snip store {}: data format code {}", store.name, store.format)))?;
    let points = (store.packet_bytes / ty.bytes() as u64) as usize;
    let mut s = SnippetSeries {
        name: store.name.clone(),
        description: "TDT snippet store".into(),
        sample_rate: store.frequency,
        samples_per_snippet: points,
        unit: if matches!(ty, SampleType::F32 | SampleType::F64) { "V".into() } else { "a.u.".into() },
        ..Default::default()
    };
    let bytes = tev.bytes();
    let mut skipped = 0;
    let mut unsorted = 0;
    let mut offsets = Vec::with_capacity(store.records.len());
    for r in &store.records {
        let (at, len) = (r.offset as usize, points * ty.bytes());
        if at + len > bytes.len() || r.data_bytes() as usize != len {
            skipped += 1;
            continue;
        }
        offsets.push(r.offset);
        s.timestamps.push(session_time(r.timestamp, block_start));
        s.channels.push(r.chan);
        s.sort_codes.push(match sort {
            Some((_, codes)) => codes.get(r.seq as usize).map_or_else(
                || {
                    unsorted += 1;
                    r.sortcode
                },
                |&c| c as u16,
            ),
            None => r.sortcode,
        });
    }
    s.waveforms = Arc::new(TevWaveforms { tev: tev.clone(), ty, points, offsets });
    if let Some((id, _)) = sort {
        s.description = format!("TDT snippet store, sort codes from offline sort {id:?}");
        if unsorted > 0 {
            warnings.push(format!("{}: sort {id:?} has no code for {unsorted} snippets (online codes kept)", store.name));
        }
    }
    if skipped > 0 {
        warnings.push(format!("{}: {skipped} snippets were outside the TEV or had a different size", store.name));
    }
    Ok(s)
}
