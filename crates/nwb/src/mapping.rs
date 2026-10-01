//! Session + user metadata → an NWB plan: what is written where, and what is missing.
//!
//! Electrodes come from the session (reader-supplied, or merged from the metadata file by
//! `MetadataFile::apply`); the metadata file here only decides naming, units and inclusion.

use nc_base::time::{format_iso, parse_iso};
use nc_core::{Calibration, Device, Issue, Level, MetadataFile, Session, StreamType};

/// NWB file-level fields, resolved.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct FileFields {
    pub description: String,
    pub identifier: String,
    /// ISO 8601 with a time zone.
    pub start_time: String,
    pub experiment_description: Option<String>,
    pub experimenters: Vec<String>,
    pub lab: Option<String>,
    pub institution: Option<String>,
    pub keywords: Vec<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SubjectFields {
    pub id: Option<String>,
    pub species: Option<String>,
    pub sex: Option<String>,
    pub age: Option<String>,
    pub strain: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GroupPlan {
    pub name: String,
    pub description: String,
    pub location: String,
    pub device: String,
}

/// One continuous signal to write.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SeriesPlan {
    /// Index into `Session::recordings`.
    pub recording: usize,
    pub source: String,
    pub name: String,
    pub description: String,
    /// Electrodes-table row of each channel for an `ElectricalSeries`; `None` for a `TimeSeries`.
    pub electrodes: Option<Vec<usize>>,
    pub unit: String,
    pub conversion: f64,
}

/// One event series to write: an `EventsTable` in `/events`, or (multi-channel scalars) a
/// `TimeSeries` in `/acquisition`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EventPlan {
    /// Index into `Session::events`.
    pub event: usize,
    pub source: String,
    pub name: String,
    pub description: String,
    /// `EventsTable` (single-value events) vs `TimeSeries` (one row of values per event).
    pub table: bool,
}

/// One snippet store to write: a `SpikeEventSeries` per channel in `/acquisition`, and its sorted
/// units (non-zero sort codes) in `/units`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SnippetPlan {
    /// Index into `Session::snippets`.
    pub snippet: usize,
    pub source: String,
    /// Series names are `<name>_ch<channel>`.
    pub name: String,
    pub description: String,
    /// Multiplier from the stored snippet values to volts.
    pub conversion: f64,
    /// `(source channel, electrodes-table row)` for every channel with snippets.
    pub rows: Vec<(u16, usize)>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TablePlan {
    pub table: usize,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct NwbPlan {
    pub file: FileFields,
    pub subject: SubjectFields,
    pub devices: Vec<Device>,
    pub groups: Vec<GroupPlan>,
    pub series: Vec<SeriesPlan>,
    pub events: Vec<EventPlan>,
    pub tables: Vec<TablePlan>,
    pub snippets: Vec<SnippetPlan>,
    /// Source items left out (by the metadata file).
    pub skipped: Vec<String>,
    pub issues: Vec<Issue>,
}

impl NwbPlan {
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.level == Level::Error)
    }
}

/// NWB object names cannot contain `/`; TDT store names often end in one (`MET/`).
pub fn safe_name(source: &str) -> String {
    let trimmed = source.trim_end_matches(['/', '\\']);
    let s: String = trimmed.chars().map(|c| if c == '/' || c == '\\' { '_' } else { c }).collect();
    if s.is_empty() { "unnamed".into() } else { s }
}

/// True for `...Z`, `...+hh:mm` or `...-hh:mm` after the time part.
fn has_zone(iso: &str) -> bool {
    let Some((_, time)) = iso.split_once('T') else { return false };
    time.ends_with('Z') || time.contains('+') || time.rfind('-').is_some()
}

fn valid_zone(z: &str) -> bool {
    z == "Z" || (z.len() == 6 && (z.starts_with('+') || z.starts_with('-')) && z.as_bytes()[3] == b':')
}

pub fn resolve(session: &Session, meta: &MetadataFile, new_identifier: impl FnOnce() -> String) -> NwbPlan {
    let mut plan = NwbPlan::default();
    let issues = &mut plan.issues;
    let sm = &session.metadata;
    let ms = &meta.session;

    // File fields
    plan.file.description = ms.description.clone().unwrap_or_default();
    if plan.file.description.trim().is_empty() {
        issues.push(Issue::error("session.description is required (a sentence describing the session)"));
    }
    plan.file.identifier = ms.identifier.clone().unwrap_or_else(new_identifier);
    plan.file.start_time = match (&ms.start_time, &sm.start_time) {
        (Some(t), _) if has_zone(t) && parse_iso(t).is_some() => t.clone(),
        (Some(t), _) => {
            issues.push(Issue::error(format!("session.start_time {t:?} needs a time zone, e.g. 2025-02-26T15:25:56-05:00")));
            t.clone()
        }
        (None, Some(rec)) if has_zone(rec) => rec.clone(),
        (None, Some(rec)) => match &ms.timezone {
            Some(z) if valid_zone(z) => format!("{}{z}", parse_iso(rec).map_or_else(|| rec.clone(), format_iso)),
            Some(z) => {
                issues.push(Issue::error(format!("session.timezone {z:?} must look like -05:00, +01:00 or Z")));
                rec.clone()
            }
            None => {
                issues.push(Issue::error(format!(
                    "the recorded start time {rec} has no time zone: set session.timezone (e.g. -05:00) or session.start_time"
                )));
                rec.clone()
            }
        },
        (None, None) => {
            issues.push(Issue::error("no start time recorded: set session.start_time"));
            String::new()
        }
    };
    plan.file.experiment_description = ms.experiment_description.clone().or_else(|| sm.experiment.clone());
    plan.file.experimenters = if ms.experimenters.is_empty() { sm.experimenters.clone() } else { ms.experimenters.clone() };
    plan.file.lab = ms.lab.clone().or_else(|| sm.lab.clone());
    plan.file.institution = ms.institution.clone().or_else(|| sm.institution.clone());
    plan.file.keywords = ms.keywords.clone();
    plan.file.notes = (!sm.notes.is_empty()).then(|| sm.notes.join("\n"));

    // Subject
    let s = &meta.subject;
    plan.subject = SubjectFields {
        id: s.id.clone().or_else(|| sm.subject.id.clone()),
        species: s.species.clone().or_else(|| sm.subject.species.clone()),
        sex: s.sex.clone().or_else(|| sm.subject.sex.clone()).or(Some("U".into())),
        age: s.age.clone().or_else(|| sm.subject.age.clone()),
        strain: s.strain.clone(),
        description: s.description.clone().or_else(|| sm.subject.description.clone()),
    };
    if plan.subject.species.is_none() {
        issues.push(Issue::warning("subject.species is missing (required by DANDI)"));
    }
    if plan.subject.age.is_none() {
        issues.push(Issue::warning("subject.age is missing (required by DANDI, ISO 8601 e.g. P90D)"));
    }
    if let Some(sex) = &plan.subject.sex {
        if !["M", "F", "U", "O"].contains(&sex.as_str()) {
            issues.push(Issue::warning(format!("subject.sex {sex:?} should be M, F, U or O")));
        }
    }

    // Devices and electrode groups (from the session)
    plan.devices = sm.devices.clone();
    for g in &session.electrode_groups {
        let device = match &g.device {
            Some(d) => {
                if !plan.devices.iter().any(|x| &x.name == d) {
                    plan.devices.push(Device { name: d.clone(), description: format!("device of electrode group {}", g.name), manufacturer: None, model: None });
                }
                d.clone()
            }
            None => match plan.devices.first() {
                Some(d) => d.name.clone(),
                None => {
                    plan.devices.push(Device { name: "acquisition_system".into(), description: "recording hardware".into(), manufacturer: None, model: None });
                    "acquisition_system".into()
                }
            },
        };
        plan.groups.push(GroupPlan { name: g.name.clone(), description: g.description.clone(), location: g.location.clone(), device });
    }

    // Streams: electrical when declared so, or (undeclared) when every channel has an electrode
    for (i, rec) in session.recordings.iter().enumerate() {
        let info = rec.info();
        let spec = meta.stream(&info.name);
        if spec.include == Some(false) {
            plan.skipped.push(format!("stream {}", info.name));
            continue;
        }
        let rows = session.channel_electrodes(&info.name);
        let complete = !rows.is_empty() && rows.iter().all(Option::is_some);
        let electrical = match spec.kind {
            Some(kind) => kind == StreamType::Electrical,
            None => complete,
        };
        let electrodes = match (electrical, complete) {
            (false, _) => None,
            (true, true) => Some(rows.into_iter().flatten().collect()),
            (true, false) => {
                let missing = rows.iter().filter(|r| r.is_none()).count();
                issues.push(Issue::error(format!(
                    "stream {}: electrical, but {missing} of {} channels have no electrode; set streams.{}.electrode_group to a declared group",
                    info.name,
                    rows.len(),
                    info.name
                )));
                None
            }
        };
        if electrical && spec.unit.as_deref().is_some_and(|u| u != "volts" && u != "V" && u != "a.u.") {
            issues.push(Issue::warning(format!("stream {}: electrical series are stored in volts; use conversion to scale", info.name)));
        }
        // Values not known to be in physical units: written as stored unless a conversion is set
        if let (Calibration::Unknown { note }, None) = (&info.calibration, spec.conversion) {
            issues.push(Issue::warning(format!(
                "stream {}: {note} and no conversion; values are written as stored. \
                 Set streams.{}.conversion (and unit) to the factor from stored units to the physical unit",
                info.name, info.name
            )));
        }
        plan.series.push(SeriesPlan {
            recording: i,
            source: info.name.clone(),
            name: spec.name.unwrap_or_else(|| safe_name(&info.name)),
            description: spec.description.unwrap_or_else(|| {
                if info.description.is_empty() {
                    format!("{} from the {} recording ({} channels)", info.name, session.provenance.format, info.channel_count())
                } else {
                    info.description.clone()
                }
            }),
            electrodes,
            unit: if electrical { "volts".into() } else { spec.unit.unwrap_or_else(|| "a.u.".into()) },
            conversion: spec.conversion.unwrap_or(1.0),
        });
    }

    // Events and tables
    for (i, e) in session.events.iter().enumerate() {
        let spec = meta.event(&e.name);
        if spec.include == Some(false) {
            plan.skipped.push(format!("events {}", e.name));
            continue;
        }
        plan.events.push(EventPlan {
            event: i,
            source: e.name.clone(),
            name: spec.name.unwrap_or_else(|| safe_name(&e.name)),
            description: spec.description.unwrap_or_else(|| {
                if e.description.is_empty() { format!("{} events from the {} recording", e.name, session.provenance.format) } else { e.description.clone() }
            }),
            table: e.channels == 1,
        });
    }
    for (i, t) in session.tables.iter().enumerate() {
        let spec = meta.table(&t.name);
        if spec.include == Some(false) {
            plan.skipped.push(format!("table {}", t.name));
            continue;
        }
        plan.tables.push(TablePlan { table: i, name: spec.name.unwrap_or_else(|| safe_name(&t.name)), description: spec.description.unwrap_or_else(|| t.description.clone()) });
    }

    // Snippets: one SpikeEventSeries per channel, on that channel's electrode
    for (i, sn) in session.snippets.iter().enumerate() {
        let spec = meta.snippet(&sn.name);
        if spec.include == Some(false) {
            plan.skipped.push(format!("snippets {}", sn.name));
            continue;
        }
        if sn.electrodes.is_empty() {
            plan.issues.push(Issue::warning(format!(
                "snippets {} ({} waveforms) have no electrodes and are not written: set snippets.{}.electrode_group to a declared group \
                 (NWB spike waveforms belong to electrodes)",
                sn.name,
                sn.len(),
                sn.name
            )));
            continue;
        }
        if sn.unit != "V" && spec.conversion.is_none() {
            plan.issues.push(Issue::warning(format!(
                "snippets {}: values are in {} and no conversion is set; set snippets.{}.conversion to the factor to volts",
                sn.name, sn.unit, sn.name
            )));
        }
        let mut rows = Vec::new();
        for c in sn.channel_set() {
            match sn.electrodes.get(&c) {
                Some(&row) => rows.push((c, row)),
                None => plan.issues.push(Issue::warning(format!("snippets {}: channel {c} has no electrode; its snippets are not written", sn.name))),
            }
        }
        plan.snippets.push(SnippetPlan {
            snippet: i,
            source: sn.name.clone(),
            name: spec.name.unwrap_or_else(|| safe_name(&sn.name)),
            description: spec.description.unwrap_or_else(|| {
                if sn.description.is_empty() { format!("{} spike snippets from the {} recording", sn.name, session.provenance.format) } else { sn.description.clone() }
            }),
            conversion: spec.conversion.unwrap_or(1.0),
            rows,
        });
    }

    // Names must be unique within their NWB group
    let mut seen = std::collections::BTreeSet::new();
    let snippet_names: Vec<String> = plan.snippets.iter().flat_map(|p| p.rows.iter().map(move |(c, _)| format!("{}_ch{c}", p.name))).collect();
    for n in plan.series.iter().map(|s| &s.name).chain(plan.events.iter().filter(|e| !e.table).map(|e| &e.name)).chain(&snippet_names) {
        if !seen.insert(n.clone()) {
            plan.issues.push(Issue::error(format!("two acquisition items are named {n:?}: rename one in the metadata file")));
        }
    }
    for key in meta.streams.keys().filter(|k| *k != "*") {
        if session.recording(key).is_none() {
            plan.issues.push(Issue::warning(format!("metadata streams.{key}: no such stream in the recording")));
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_names_and_zones() {
        assert_eq!(safe_name("MET/"), "MET");
        assert_eq!(safe_name("CnA\\"), "CnA");
        assert_eq!(safe_name("a/b"), "a_b");
        assert!(has_zone("2025-02-26T15:25:56-05:00"));
        assert!(has_zone("2025-02-26T15:25:56Z"));
        assert!(!has_zone("2025-02-26T15:25:56"));
        assert!(valid_zone("-05:00") && valid_zone("Z") && !valid_zone("EST"));
    }
}
