//! The neutral model of one recording session.

use std::sync::Arc;

use super::electrodes::{Electrode, ElectrodeGroup};
use super::events::EventSeries;
use super::metadata::SessionMetadata;
use super::provenance::Provenance;
use super::recording::Recording;
use super::snippets::SnippetSeries;
use super::table::Table;

/// Everything read from one recording session: what inputs produce and outputs consume.
#[derive(Default)]
pub struct Session {
    /// Identification, subject, devices, notes.
    pub metadata: SessionMetadata,
    /// Continuous signals.
    pub recordings: Vec<Arc<dyn Recording>>,
    /// Event series (TTL, epocs, markers, scalars).
    pub events: Vec<EventSeries>,
    /// Spike or triggered waveform stores.
    pub snippets: Vec<SnippetSeries>,
    /// Electrode groups (probes, shanks, grids).
    pub electrode_groups: Vec<ElectrodeGroup>,
    /// Contacts, in electrode-table order; referenced by index from snippets.
    pub electrodes: Vec<Electrode>,
    /// Small text tables (impedance exports).
    pub tables: Vec<Table>,
    /// Reader, files read, warnings.
    pub provenance: Provenance,
}

impl Session {
    /// The recording named `name`.
    pub fn recording(&self, name: &str) -> Option<&Arc<dyn Recording>> {
        self.recordings.iter().find(|r| r.info().name == name)
    }

    /// The event series named `name`.
    pub fn event_series(&self, name: &str) -> Option<&EventSeries> {
        self.events.iter().find(|e| e.name == name)
    }

    /// The electrode group named `name`.
    pub fn electrode_group(&self, name: &str) -> Option<&ElectrodeGroup> {
        self.electrode_groups.iter().find(|g| g.name == name)
    }

    /// The electrode (index into [`Session::electrodes`]) of every channel of `recording`, in
    /// channel order; `None` for channels without one.
    pub fn channel_electrodes(&self, recording: &str) -> Vec<Option<usize>> {
        let channels = self.recording(recording).map_or(0, |r| r.info().channel_count());
        let mut out = vec![None; channels];
        for (i, e) in self.electrodes.iter().enumerate() {
            for c in e.channels.iter().filter(|c| c.recording == recording && c.channel < channels) {
                out[c.channel].get_or_insert(i);
            }
        }
        out
    }

    /// Longest recording duration in seconds.
    pub fn duration(&self) -> f64 {
        self.recordings.iter().map(|r| r.info().start_time + r.info().duration()).fold(0.0, f64::max)
    }
}
