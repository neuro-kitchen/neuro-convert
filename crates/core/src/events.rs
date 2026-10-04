//! Events: TTL lines, epocs, markers, text notes and sampled scalar values.

use serde::{Deserialize, Serialize};

/// Timestamped events: TTL/epoc onsets (with optional offsets) or sampled scalar values.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EventSeries {
    /// Name inside its session.
    pub name: String,
    /// What the events mark.
    pub description: String,
    /// Seconds from the session start, ascending.
    pub onsets: Vec<f64>,
    /// Matching offsets for interval events (same length as `onsets`).
    pub offsets: Option<Vec<f64>>,
    /// One value per event, or `channels` values per event for multi-channel scalars
    /// (row-major: event 0's channels, then event 1's, …).
    pub values: Vec<f64>,
    /// Values per event (1 for epocs).
    pub channels: usize,
    /// Text per event (e.g. user notes); empty when unused.
    pub labels: Vec<String>,
}

impl EventSeries {
    /// Number of events.
    pub fn len(&self) -> usize {
        self.onsets.len()
    }

    /// `true` without events.
    pub fn is_empty(&self) -> bool {
        self.onsets.is_empty()
    }
}
