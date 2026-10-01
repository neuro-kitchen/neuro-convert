//! Invariants every [`Session`] must hold, whatever reader produced it. Writers rely on them, so
//! an error here means a broken reader (or a broken metadata merge), not a user mistake.
//! Checks only look at descriptions and in-memory data; continuous samples are not read.

use std::collections::BTreeSet;

use nc_base::time::parse_iso;

use crate::issue::Issue;
use crate::session::Session;

impl Session {
    /// Problems with this session's structure: errors break outputs, warnings are suspicious.
    pub fn validate(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        let mut err = |m: String| issues.push(Issue::error(m));

        if let Some(t) = self.metadata.start_time.as_deref().filter(|t| parse_iso(t).is_none()) {
            err(format!("metadata.start_time {t:?} is not an ISO 8601 date-time"));
        }

        // Recordings
        let mut names = BTreeSet::new();
        for r in &self.recordings {
            let i = r.info();
            let what = format!("recording {:?}", i.name);
            if i.name.is_empty() {
                err("a recording has no name".into());
            } else if !names.insert(i.name.as_str()) {
                err(format!("{what} appears twice"));
            }
            if i.channels.is_empty() {
                err(format!("{what} has no channels"));
            }
            if !(i.sample_rate.is_finite() && i.sample_rate > 0.0) {
                err(format!("{what}: sample rate {} is not positive", i.sample_rate));
            }
            if !i.start_time.is_finite() {
                err(format!("{what}: start time is not finite"));
            }
            if let Some(c) = i.channels.iter().find(|c| !(c.gain.is_finite() && c.offset.is_finite())) {
                err(format!("{what}: channel {:?} has a non-finite gain or offset", c.name));
            }
        }

        // Events
        let mut names = BTreeSet::new();
        for e in &self.events {
            let what = format!("events {:?}", e.name);
            if !names.insert(e.name.as_str()) {
                err(format!("{what} appear twice"));
            }
            let n = e.len();
            if e.channels == 0 {
                err(format!("{what}: channels must be at least 1"));
            } else if e.values.len() != n * e.channels {
                err(format!("{what}: {} values for {n} events × {} channels", e.values.len(), e.channels));
            }
            if !e.labels.is_empty() && e.labels.len() != n {
                err(format!("{what}: {} labels for {n} events", e.labels.len()));
            }
            if e.onsets.iter().any(|t| !t.is_finite()) {
                err(format!("{what}: an onset is not finite"));
            }
            if let Some(off) = &e.offsets {
                if off.len() != n {
                    err(format!("{what}: {} offsets for {n} onsets", off.len()));
                } else if e.onsets.iter().zip(off).any(|(a, b)| b < a) {
                    err(format!("{what}: an event ends before it starts"));
                }
            }
        }

        // Snippets
        for s in &self.snippets {
            let what = format!("snippets {:?}", s.name);
            let n = s.len();
            if s.channels.len() != n || s.sort_codes.len() != n {
                err(format!("{what}: {n} timestamps but {} channels and {} sort codes", s.channels.len(), s.sort_codes.len()));
            }
            if s.data.len() != n * s.samples_per_snippet {
                err(format!("{what}: {} values for {n} snippets × {} samples", s.data.len(), s.samples_per_snippet));
            }
            if n > 0 && !(s.sample_rate.is_finite() && s.sample_rate > 0.0) {
                err(format!("{what}: sample rate {} is not positive", s.sample_rate));
            }
            for (c, &i) in &s.electrodes {
                if i >= self.electrodes.len() {
                    err(format!("{what}: channel {c} points to electrode {i}, but there are {}", self.electrodes.len()));
                }
            }
        }

        // Electrode groups and electrodes
        let mut groups = BTreeSet::new();
        for g in &self.electrode_groups {
            if !groups.insert(g.name.as_str()) {
                err(format!("electrode group {:?} appears twice", g.name));
            }
        }
        let mut channels = BTreeSet::new();
        for e in &self.electrodes {
            let what = format!("electrode {:?}", e.name);
            if !groups.contains(e.group.as_str()) {
                err(format!("{what}: group {:?} is not an electrode group of the session", e.group));
            }
            for c in &e.channels {
                match self.recording(&c.recording) {
                    None => err(format!("{what}: recording {:?} does not exist", c.recording)),
                    Some(r) if c.channel >= r.info().channel_count() => {
                        err(format!("{what}: channel {} is outside {:?} ({} channels)", c.channel, c.recording, r.info().channel_count()))
                    }
                    Some(_) => {
                        if !channels.insert((c.recording.as_str(), c.channel)) {
                            err(format!("{what}: channel {} of {:?} already has an electrode", c.channel, c.recording));
                        }
                    }
                }
            }
            if e.position_um.is_some_and(|p| p.iter().any(|v| !v.is_finite())) {
                err(format!("{what}: position is not finite"));
            }
        }

        // Soft checks
        for e in &self.events {
            if e.onsets.windows(2).any(|w| w[1] < w[0]) {
                issues.push(Issue::warning(format!("events {:?}: onsets are not in time order", e.name)));
            }
        }
        for g in &self.electrode_groups {
            if let Some(d) = g.device.as_ref().filter(|d| !self.metadata.devices.iter().any(|x| &x.name == *d)) {
                issues.push(Issue::warning(format!("electrode group {:?}: device {d:?} is not a device of the session", g.name)));
            }
        }
        for t in &self.tables {
            if t.rows.iter().any(|r| r.len() != t.columns.len()) {
                issues.push(Issue::warning(format!("table {:?}: some rows do not have {} cells", t.name, t.columns.len())));
            }
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::electrodes::{ChannelRef, Electrode, ElectrodeGroup};
    use crate::{EventSeries, Level, MemoryRecording, Session, SnippetSeries};

    fn errors(s: &Session) -> Vec<String> {
        s.validate().into_iter().filter(|i| i.level == Level::Error).map(|i| i.message).collect()
    }

    #[test]
    fn test_valid_session_has_no_issues() {
        let mut s = Session::default();
        s.metadata.start_time = Some("2025-02-26T15:25:56".into());
        s.recordings.push(Arc::new(MemoryRecording::new("A", vec![0.0; 4], 2, 10.0, "V").unwrap()));
        s.events.push(EventSeries { name: "E".into(), onsets: vec![0.1, 0.2], offsets: Some(vec![0.15, 0.3]), values: vec![1.0, 2.0], channels: 1, ..Default::default() });
        s.electrode_groups.push(ElectrodeGroup { name: "G".into(), ..Default::default() });
        s.electrodes.push(Electrode { name: "e0".into(), group: "G".into(), channels: vec![ChannelRef { recording: "A".into(), channel: 0 }], ..Default::default() });
        // A site in two bands: one electrode, a channel in each recording
        s.recordings.push(Arc::new(MemoryRecording::new("B", vec![0.0; 2], 1, 1.0, "V").unwrap()));
        s.electrodes.push(Electrode {
            name: "e1".into(),
            group: "G".into(),
            channels: vec![ChannelRef { recording: "A".into(), channel: 1 }, ChannelRef { recording: "B".into(), channel: 0 }],
            ..Default::default()
        });
        assert!(s.validate().is_empty(), "{:?}", s.validate());
    }

    #[test]
    fn test_broken_sessions_are_reported() {
        let mut s = Session::default();
        s.metadata.start_time = Some("yesterday".into());
        s.recordings.push(Arc::new(MemoryRecording::new("A", vec![0.0; 4], 2, 10.0, "V").unwrap()));
        s.recordings.push(Arc::new(MemoryRecording::new("A", vec![0.0; 2], 1, 0.0, "V").unwrap()));
        s.events.push(EventSeries { name: "E".into(), onsets: vec![0.2, 0.1], offsets: Some(vec![0.1, 0.3]), values: vec![1.0], channels: 1, ..Default::default() });
        s.snippets.push(SnippetSeries { name: "S".into(), timestamps: vec![0.1], channels: vec![1], sort_codes: vec![], samples_per_snippet: 2, data: vec![0.0; 2], sample_rate: 1.0, electrodes: [(1, 5)].into(), ..Default::default() });
        s.electrodes.push(Electrode { name: "e".into(), group: "missing".into(), channels: vec![ChannelRef { recording: "A".into(), channel: 9 }], ..Default::default() });
        let e = errors(&s).join("\n");
        for want in [
            "not an ISO 8601",
            "\"A\" appears twice",
            "sample rate 0 is not positive",
            "1 values for 2 events",
            "ends before it starts",
            "0 sort codes",
            "points to electrode 5",
            "group \"missing\"",
            "channel 9 is outside",
        ] {
            assert!(e.contains(want), "missing {want:?} in:\n{e}");
        }
        assert!(s.validate().iter().any(|i| i.level == Level::Warning && i.message.contains("not in time order")));
    }
}
