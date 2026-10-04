//! Merging the user's metadata file into a [`Session`]: electrode groups, the electrodes of
//! streams declared electrical, impedances from session tables, and the electrodes of snippet
//! channels. After this, writers find everything about electrodes in the session itself.

use crate::electrodes::{ChannelRef, Electrode, ElectrodeGroup};
use crate::issue::{Issue, Target};
use crate::metadata_file::{ImpedanceSpec, MetadataFile, StreamType};
use crate::session::Session;
use crate::table::Table;

impl MetadataFile {
    /// Adds what this file declares about electrodes to `session`. Declared groups replace
    /// same-named groups from the reader (keeping the reader's device unless one is declared);
    /// electrodes and positions supplied by the reader are kept, only their group is reassigned.
    /// Returns problems that make a declaration unusable.
    pub fn apply(&self, session: &mut Session) -> Vec<Issue> {
        let mut issues = Vec::new();
        for g in &self.electrode_groups {
            let declared = ElectrodeGroup { name: g.name.clone(), description: g.description.clone(), location: g.location.clone(), device: g.device.clone() };
            match session.electrode_groups.iter_mut().find(|x| x.name == g.name) {
                Some(x) => {
                    let device = declared.device.clone().or(x.device.take());
                    *x = ElectrodeGroup { device, ..declared };
                }
                None => session.electrode_groups.push(declared),
            }
        }
        self.apply_streams(session, &mut issues);
        self.apply_impedances(session, &mut issues);
        self.apply_snippets(session, &mut issues);
        issues
    }

    /// Streams declared `electrical` with an `electrode_group`: one electrode per channel.
    fn apply_streams(&self, session: &mut Session, issues: &mut Vec<Issue>) {
        let names: Vec<String> = session.recordings.iter().map(|r| r.info().name.clone()).collect();
        for name in names {
            let spec = self.stream(&name);
            if spec.include == Some(false) || spec.kind != Some(StreamType::Electrical) {
                continue;
            }
            let Some(group) = spec.electrode_group else { continue };
            if session.electrode_group(&group).is_none() {
                issues.push(Issue::error(format!("stream {name}: electrode_group {group:?} is not declared under electrode_groups")).at(Target::Stream(name.clone())));
                continue;
            }
            let existing = session.channel_electrodes(&name);
            let channel_names: Vec<String> = session.recording(&name).map(|r| r.info().channels.iter().map(|c| c.name.clone()).collect()).unwrap_or_default();
            for (c, (have, channel_name)) in existing.into_iter().zip(channel_names).enumerate() {
                match have {
                    Some(i) => session.electrodes[i].group = group.clone(),
                    None => session.electrodes.push(Electrode {
                        name: channel_name,
                        group: group.clone(),
                        channels: vec![ChannelRef { recording: name.clone(), channel: c }],
                        ..Default::default()
                    }),
                }
            }
        }
    }

    /// Fills missing impedances of each group's continuous channels from its impedance table:
    /// channel `c` (0-based) reads column `<prefix><c + 1>`.
    fn apply_impedances(&self, session: &mut Session, issues: &mut Vec<Issue>) {
        for g in &self.electrode_groups {
            let Some(spec) = &g.impedance else { continue };
            let Some(table) = session.tables.iter().find(|t| t.name == spec.table) else {
                issues.push(Issue::warning(format!("electrode group {}: impedance table {:?} not found", g.name, spec.table)).at(Target::ElectrodeGroup(g.name.clone())));
                continue;
            };
            let values: Vec<(usize, Option<f32>)> = session
                .electrodes
                .iter()
                .enumerate()
                .filter(|(_, e)| e.group == g.name && e.impedance_ohms.is_none())
                .filter_map(|(i, e)| Some((i, impedance(table, spec, e.channels.first()?.channel + 1))))
                .collect();
            for (i, v) in values {
                session.electrodes[i].impedance_ohms = v;
            }
        }
    }

    /// Snippet stores with an `electrode_group`: source channel `c` is the `c`-th channel of the
    /// group's first recording; a group without recordings gets one contact per channel.
    fn apply_snippets(&self, session: &mut Session, issues: &mut Vec<Issue>) {
        for si in 0..session.snippets.len() {
            let name = session.snippets[si].name.clone();
            let spec = self.snippet(&name);
            if spec.include == Some(false) {
                continue;
            }
            let Some(group) = spec.electrode_group else { continue };
            if session.electrode_group(&group).is_none() {
                issues.push(Issue::error(format!("snippets {name}: electrode_group {group:?} is not declared under electrode_groups")).at(Target::Snippet(name.clone())));
                continue;
            }
            let recording = session.electrodes.iter().find(|e| e.group == group).and_then(|e| e.channels.first()).map(|c| c.recording.clone());
            let map = recording.as_deref().map(|r| session.channel_electrodes(r));
            for c in session.snippets[si].channel_set() {
                let index = match &map {
                    Some(m) if c >= 1 && (c as usize) <= m.len() => m[c as usize - 1],
                    Some(m) => {
                        issues.push(Issue::error(format!("snippets {name}: channel {c} is outside the {} electrodes of group {group:?}", m.len())).at(Target::Snippet(name.clone())));
                        continue;
                    }
                    None => {
                        let contact = format!("{name} {c}");
                        Some(match session.electrodes.iter().position(|e| e.group == group && e.name == contact) {
                            Some(i) => i,
                            None => {
                                session.electrodes.push(Electrode { name: contact, group: group.clone(), ..Default::default() });
                                session.electrodes.len() - 1
                            }
                        })
                    }
                };
                if let Some(i) = index {
                    session.snippets[si].electrodes.insert(c, i);
                }
            }
        }
    }
}

/// Impedance (ohms) of 1-based channel `n` from `table` column `<prefix><n> (<unit>)`; `None` when
/// the column is missing or never measured (negative values mean "not measured").
pub fn impedance(table: &Table, spec: &ImpedanceSpec, n: usize) -> Option<f32> {
    let prefix = spec.prefix.as_deref().unwrap_or("R");
    let head = format!("{prefix}{n} (");
    let (ci, col) = table.columns.iter().enumerate().find(|(_, c)| c.starts_with(&head))?;
    let scale = if col.ends_with("(kOhm)") { 1e3 } else if col.ends_with("(MOhm)") { 1e6 } else { 1.0 };
    let value = |r: &Vec<String>| r.get(ci).and_then(|v| v.parse::<f64>().ok()).filter(|v| *v >= 0.0);
    let v = match spec.row {
        Some(row) => table.rows.get(row).and_then(value),
        None => table.rows.iter().rev().find_map(value),
    };
    v.map(|v| (v * scale) as f32)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{MemoryRecording, SnippetSeries};

    #[test]
    fn test_impedance_last_measured_row() {
        let t = Table {
            name: "Z".into(),
            description: String::new(),
            columns: vec!["TIME (S)".into(), "R1 (kOhm)".into(), "R2 (kOhm)".into()],
            rows: vec![vec!["60".into(), "1.5".into(), "0.4".into()], vec!["64".into(), "-1.00".into(), "0.5".into()]],
        };
        let spec = ImpedanceSpec { table: "Z".into(), ..Default::default() };
        assert_eq!([impedance(&t, &spec, 1), impedance(&t, &spec, 2), impedance(&t, &spec, 3)], [Some(1500.0), Some(500.0), None]);
        let first = ImpedanceSpec { row: Some(0), ..spec };
        assert_eq!(impedance(&t, &first, 2), Some(400.0));
    }

    fn session() -> Session {
        let mut s = Session::default();
        s.recordings.push(Arc::new(MemoryRecording::new("EMG1", vec![0.0; 6], 3, 1000.0, "V").unwrap()));
        s.recordings.push(Arc::new(MemoryRecording::new("Temp", vec![0.0; 2], 1, 10.0, "a.u.").unwrap()));
        let snip = |name: &str, channels: Vec<u16>| SnippetSeries {
            name: name.into(),
            sample_rate: 1000.0,
            timestamps: (0..channels.len()).map(|i| i as f64 * 0.1).collect(),
            sort_codes: vec![0; channels.len()],
            waveforms: Arc::new(crate::MemoryWaveforms::new(1, vec![0.0; channels.len()])),
            samples_per_snippet: 1,
            channels,
            ..Default::default()
        };
        s.snippets.push(snip("eNe1", vec![3, 1]));
        s.snippets.push(snip("eNe2", vec![2]));
        s.tables.push(Table { name: "Z".into(), columns: vec!["R1 (kOhm)".into(), "R3 (Ohm)".into()], rows: vec![vec!["2".into(), "7".into()]], ..Default::default() });
        s
    }

    const META: &str = "electrode_groups:\n  - { name: G, description: grid, location: diaphragm, impedance: { table: Z } }\n  - { name: Lone, description: probe, location: cortex }\n\
streams:\n  '*': { type: timeseries }\n  EMG1: { type: electrical, electrode_group: G }\nsnippets:\n  eNe1: { electrode_group: G }\n  eNe2: { electrode_group: Lone }\n";

    #[test]
    fn test_apply_builds_electrodes() {
        let mut s = session();
        let issues = MetadataFile::parse(META).unwrap().apply(&mut s);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(s.electrode_groups.len(), 2);
        // EMG1's three channels, then a snippet-only contact for group Lone
        assert_eq!(s.electrodes.len(), 4);
        assert_eq!(s.channel_electrodes("EMG1"), vec![Some(0), Some(1), Some(2)]);
        assert_eq!(s.channel_electrodes("Temp"), vec![None]);
        let imp: Vec<Option<f32>> = s.electrodes.iter().map(|e| e.impedance_ohms).collect();
        assert_eq!(imp, vec![Some(2000.0), None, Some(7.0), None]);
        assert_eq!(s.snippets[0].electrodes.iter().map(|(c, i)| (*c, *i)).collect::<Vec<_>>(), vec![(1, 0), (3, 2)]);
        assert_eq!((s.electrodes[3].name.as_str(), s.electrodes[3].group.as_str()), ("eNe2 2", "Lone"));
        assert_eq!(s.snippets[1].electrodes[&2], 3);
        assert!(s.validate().is_empty(), "{:?}", s.validate());
    }

    #[test]
    fn test_apply_keeps_reader_electrodes_and_reports_bad_declarations() {
        let mut s = session();
        // The reader already knows channel 1's position
        s.electrodes.push(Electrode {
            name: "c1".into(),
            group: "probe".into(),
            channels: vec![ChannelRef { recording: "EMG1".into(), channel: 1 }],
            position_um: Some([0.0, 20.0, 0.0]),
            ..Default::default()
        });
        s.electrode_groups.push(ElectrodeGroup { name: "G".into(), device: Some("RZ2".into()), ..Default::default() });
        MetadataFile::parse(META).unwrap().apply(&mut s);
        assert_eq!(s.electrode_groups.iter().find(|g| g.name == "G").unwrap().device.as_deref(), Some("RZ2"), "reader device kept");
        assert_eq!(s.channel_electrodes("EMG1"), vec![Some(1), Some(0), Some(2)]);
        assert_eq!((s.electrodes[0].group.as_str(), s.electrodes[0].position_um), ("G", Some([0.0, 20.0, 0.0])));

        let bad = "electrode_groups: []\nstreams:\n  EMG1: { type: electrical, electrode_group: Nope }\nsnippets:\n  eNe1: { electrode_group: Nope }\n";
        let issues = MetadataFile::parse(bad).unwrap().apply(&mut session());
        assert_eq!(issues.iter().filter(|i| i.message.contains("\"Nope\" is not declared")).count(), 2, "{issues:?}");

        let outside = "electrode_groups:\n  - { name: G, description: g, location: x }\nstreams:\n  EMG1: { type: electrical, electrode_group: G }\nsnippets:\n  eNe2: { electrode_group: G }\n";
        let mut s = session();
        s.snippets[1].channels = vec![4];
        let issues = MetadataFile::parse(outside).unwrap().apply(&mut s);
        assert!(issues.iter().any(|i| i.message.contains("channel 4 is outside the 3 electrodes")), "{issues:?}");
    }
}
