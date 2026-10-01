//! The user's metadata file (YAML): what the source files do not say, and how each source item
//! maps to the output. Keys under `streams` / `events` / `tables` are source names (e.g. TDT
//! store names); `"*"` sets the default for every item not listed.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use nc_base::{Error, Result};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetadataFile {
    pub session: SessionSection,
    pub subject: SubjectSection,
    pub electrode_groups: Vec<ElectrodeGroupSpec>,
    pub streams: BTreeMap<String, StreamSpec>,
    pub events: BTreeMap<String, ItemSpec>,
    pub tables: BTreeMap<String, ItemSpec>,
    /// Snippet (spike waveform) stores, by source name or `*`.
    pub snippets: BTreeMap<String, SnippetSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionSection {
    pub description: Option<String>,
    /// Unique id; a UUID is generated when absent.
    pub identifier: Option<String>,
    /// ISO 8601 with a time zone, e.g. `2025-02-26T15:25:56-05:00` (overrides the recorded time).
    pub start_time: Option<String>,
    /// Zone of the recorded local start time, e.g. `-05:00` (used when `start_time` is absent).
    pub timezone: Option<String>,
    pub experiment_description: Option<String>,
    pub experimenters: Vec<String>,
    pub lab: Option<String>,
    pub institution: Option<String>,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SubjectSection {
    pub id: Option<String>,
    /// Latin binomial, e.g. `Rattus norvegicus`.
    pub species: Option<String>,
    /// `M`, `F`, `U` (unknown) or `O` (other).
    pub sex: Option<String>,
    /// ISO 8601 duration, e.g. `P90D`.
    pub age: Option<String>,
    pub strain: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ElectrodeGroupSpec {
    pub name: String,
    pub description: String,
    /// Anatomical location (e.g. `diaphragm`).
    pub location: String,
    /// Device name; defaults to the first device of the session.
    pub device: Option<String>,
    /// Impedance table for this group's channels (fills the electrodes `imp` column).
    pub impedance: Option<ImpedanceSpec>,
}

/// Where a group's impedances are: a session table with one column per channel, named
/// `<prefix><n> (kOhm)` (or `(Ohm)` / `(MOhm)`), `n` counting channels from 1.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImpedanceSpec {
    pub table: String,
    /// Column prefix before the channel number (default `R`).
    pub prefix: Option<String>,
    /// Row to use; default: the last row with a measurement (>= 0) for each channel.
    pub row: Option<usize>,
}

/// How a continuous signal is exported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamType {
    /// Voltage from electrodes (NWB `ElectricalSeries`, needs an electrode group).
    Electrical,
    /// Any other signal (NWB `TimeSeries`).
    Timeseries,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StreamSpec {
    pub include: Option<bool>,
    /// Output name (defaults to the source name, made safe).
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<StreamType>,
    pub electrode_group: Option<String>,
    pub description: Option<String>,
    /// Unit of `TimeSeries` data (electrical series are always volts).
    pub unit: Option<String>,
    /// Multiplier from stored values to `unit` (volts for electrical series).
    pub conversion: Option<f64>,
}

/// How a snippet store is written: one NWB `SpikeEventSeries` per channel, on the electrodes of
/// `electrode_group` (channel `c` is the group's `c`-th electrode, 1-based as in TDT).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SnippetSpec {
    pub include: Option<bool>,
    /// Output name prefix (defaults to the source name, made safe); series are `<name>_ch<c>`.
    pub name: Option<String>,
    pub description: Option<String>,
    pub electrode_group: Option<String>,
    /// Multiplier from stored snippet values to volts.
    pub conversion: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ItemSpec {
    pub include: Option<bool>,
    pub name: Option<String>,
    pub description: Option<String>,
}

/// The kinds of source items a metadata file can include or leave out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Stream,
    Event,
    Table,
    Snippet,
}

impl MetadataFile {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        Self::parse(&text).map_err(|e| Error::format("metadata", format!("{}: {e}", path.display())))
    }

    pub fn parse(text: &str) -> std::result::Result<Self, serde_yaml_ng::Error> {
        serde_yaml_ng::from_str(text)
    }

    /// The file as YAML with only the fields that are set (empty values left out); `parse` reads
    /// it back to an equal value. Comments of a loaded file are not kept.
    pub fn to_yaml(&self) -> String {
        let mut value = serde_yaml_ng::to_value(self).expect("metadata is plain data");
        prune(&mut value);
        let text = serde_yaml_ng::to_string(&value).expect("metadata is plain data");
        if text.trim() == "{}" { String::new() } else { text }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.to_yaml()).map_err(|e| Error::io(path, e))
    }

    /// A starting point for `session`: the recorded subject id and the reader's electrode groups
    /// (so their location can be filled in); everything else left for the user.
    pub fn template(session: &crate::Session) -> Self {
        let mut m = Self::default();
        m.subject.id = session.metadata.subject.id.clone();
        m.electrode_groups = session
            .electrode_groups
            .iter()
            .map(|g| ElectrodeGroupSpec {
                name: g.name.clone(),
                description: g.description.clone(),
                location: g.location.clone(),
                device: g.device.clone(),
                impedance: None,
            })
            .collect();
        m
    }

    /// Whether source item `name` of `kind` is written (its own `include` over the `"*"` default).
    pub fn included(&self, kind: ItemKind, name: &str) -> bool {
        let include = match kind {
            ItemKind::Stream => self.stream(name).include,
            ItemKind::Event => self.event(name).include,
            ItemKind::Table => self.table(name).include,
            ItemKind::Snippet => self.snippet(name).include,
        };
        include != Some(false)
    }

    /// Includes or leaves out source item `name`. Writes the smallest change: an explicit flag
    /// only where the `"*"` default says otherwise; entries left empty are removed.
    pub fn set_included(&mut self, kind: ItemKind, name: &str, include: bool) {
        let default_included = |include: Option<bool>| include != Some(false);
        macro_rules! set {
            ($map:expr) => {{
                let default = default_included($map.get("*").and_then(|s| s.include));
                let entry = $map.entry(name.to_string()).or_default();
                entry.include = (include != default).then_some(include);
                if *entry == Default::default() {
                    $map.remove(name);
                }
            }};
        }
        match kind {
            ItemKind::Stream => set!(self.streams),
            ItemKind::Event => set!(self.events),
            ItemKind::Table => set!(self.tables),
            ItemKind::Snippet => set!(self.snippets),
        }
    }

    /// The spec for source stream `name`: its own entry over the `"*"` default.
    pub fn stream(&self, name: &str) -> StreamSpec {
        let own = self.streams.get(name).cloned().unwrap_or_default();
        let def = self.streams.get("*").cloned().unwrap_or_default();
        StreamSpec {
            include: own.include.or(def.include),
            name: own.name,
            kind: own.kind.or(def.kind),
            electrode_group: own.electrode_group.or(def.electrode_group),
            description: own.description,
            unit: own.unit.or(def.unit),
            conversion: own.conversion.or(def.conversion),
        }
    }

    pub fn event(&self, name: &str) -> ItemSpec {
        item(&self.events, name)
    }

    pub fn table(&self, name: &str) -> ItemSpec {
        item(&self.tables, name)
    }

    pub fn snippet(&self, name: &str) -> SnippetSpec {
        let own = self.snippets.get(name).cloned().unwrap_or_default();
        let def = self.snippets.get("*").cloned().unwrap_or_default();
        SnippetSpec {
            include: own.include.or(def.include),
            name: own.name,
            description: own.description,
            electrode_group: own.electrode_group.or(def.electrode_group),
            conversion: own.conversion.or(def.conversion),
        }
    }
}

fn item(map: &BTreeMap<String, ItemSpec>, name: &str) -> ItemSpec {
    let own = map.get(name).cloned().unwrap_or_default();
    let def = map.get("*").cloned().unwrap_or_default();
    ItemSpec { include: own.include.or(def.include), name: own.name, description: own.description }
}

/// Removes nulls, empty strings, empty lists and empty maps (recursively).
fn prune(value: &mut serde_yaml_ng::Value) {
    use serde_yaml_ng::Value;
    let empty = |v: &Value| match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Sequence(s) => s.is_empty(),
        Value::Mapping(m) => m.is_empty(),
        _ => false,
    };
    match value {
        Value::Mapping(map) => {
            for (_, v) in map.iter_mut() {
                prune(v);
            }
            map.retain(|_, v| !empty(v));
        }
        Value::Sequence(seq) => seq.iter_mut().for_each(prune),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yaml_round_trip_keeps_only_set_fields() {
        assert_eq!(MetadataFile::default().to_yaml(), "");
        let text = "session:\n  description: test\n  timezone: '-05:00'\n  keywords: [a, b]\nsubject:\n  species: Rattus norvegicus\n\
electrode_groups:\n  - { name: G, description: grid, location: diaphragm, impedance: { table: Z } }\n\
streams:\n  '*': { type: timeseries, unit: a.u. }\n  HDEG: { type: electrical, electrode_group: G, conversion: 1.0e-6 }\n";
        let m = MetadataFile::parse(text).unwrap();
        let yaml = m.to_yaml();
        assert!(!yaml.contains("null") && !yaml.contains("[]") && !yaml.contains("identifier"), "{yaml}");
        assert_eq!(MetadataFile::parse(&yaml).unwrap(), m, "{yaml}");
    }

    #[test]
    fn test_include_flags_are_minimal() {
        let mut m = MetadataFile::default();
        assert!(m.included(ItemKind::Stream, "A"));
        m.set_included(ItemKind::Stream, "A", false);
        assert!(!m.included(ItemKind::Stream, "A"));
        assert_eq!(m.streams["A"].include, Some(false));
        m.set_included(ItemKind::Stream, "A", true);
        assert!(m.included(ItemKind::Stream, "A") && !m.streams.contains_key("A"), "back to default: entry removed");

        // An entry with other settings keeps them; a "*" exclusion needs an explicit include
        m.streams.insert("B".into(), StreamSpec { unit: Some("mV".into()), ..Default::default() });
        m.set_included(ItemKind::Stream, "B", false);
        m.set_included(ItemKind::Stream, "B", true);
        assert_eq!(m.streams["B"], StreamSpec { unit: Some("mV".into()), ..Default::default() });
        m.events.insert("*".into(), ItemSpec { include: Some(false), ..Default::default() });
        assert!(!m.included(ItemKind::Event, "E"));
        m.set_included(ItemKind::Event, "E", true);
        assert_eq!((m.included(ItemKind::Event, "E"), m.events["E"].include), (true, Some(true)));
    }

    #[test]
    fn test_parse_and_defaults() {
        let m = MetadataFile::parse(
            "session:\n  description: test\n  timezone: '-05:00'\nsubject:\n  species: Rattus norvegicus\n\
electrode_groups:\n  - name: HDEMG\n    description: grid\n    location: diaphragm\n\
streams:\n  '*': { type: timeseries, unit: a.u. }\n  HDEG: { type: electrical, electrode_group: HDEMG }\n  SU_1: { include: false }\n",
        )
        .unwrap();
        assert_eq!(m.session.timezone.as_deref(), Some("-05:00"));
        assert_eq!(m.stream("HDEG").kind, Some(StreamType::Electrical));
        assert_eq!(m.stream("HDEG").unit.as_deref(), Some("a.u."));
        assert_eq!(m.stream("bpPe").kind, Some(StreamType::Timeseries));
        assert_eq!(m.stream("SU_1").include, Some(false));
        assert!(MetadataFile::parse("sesion: {}\n").is_err(), "typos are rejected");
    }
}
