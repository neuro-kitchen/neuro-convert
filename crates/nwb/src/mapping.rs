//! Session + user metadata → an NWB plan: what is written where, and what is missing.
//!
//! Electrodes come from the session (reader-supplied, or merged from the metadata file by
//! `MetadataFile::apply`); the metadata file here only decides naming, units and inclusion.

use nc_base::time::{format_iso, parse_iso};
use nc_core::{Calibration, Device, Issue, Level, MetadataFile, Session, StreamType, Target};

/// NWB file-level fields, resolved.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct FileFields {
    /// Session description.
    pub description: String,
    /// Unique identifier (UUID when the metadata gives none).
    pub identifier: String,
    /// ISO 8601 with a time zone.
    pub start_time: String,
    /// What the experiment is about.
    pub experiment_description: Option<String>,
    /// People who ran the session.
    pub experimenters: Vec<String>,
    /// Lab name.
    pub lab: Option<String>,
    /// Institution name.
    pub institution: Option<String>,
    /// Search keywords.
    pub keywords: Vec<String>,
    /// Session notes recorded by the source, joined.
    pub notes: Option<String>,
    /// What wrote the file (`/general/source_script`): program, reader and writer versions; its
    /// `file_name` attribute is the program. Set by the conversion job.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_script: Option<(String, String)>,
}

/// `/general/subject` fields, resolved.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SubjectFields {
    /// Subject id.
    pub id: Option<String>,
    /// Latin binomial.
    pub species: Option<String>,
    /// `M`, `F`, `U` or `O`.
    pub sex: Option<String>,
    /// ISO 8601 duration.
    pub age: Option<String>,
    /// Strain.
    pub strain: Option<String>,
    /// Free text.
    pub description: Option<String>,
}

/// One electrode group to write.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GroupPlan {
    /// Group name.
    pub name: String,
    /// What the group is.
    pub description: String,
    /// Anatomical location.
    pub location: String,
    /// Device the group links to.
    pub device: String,
}

/// One continuous signal to write.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SeriesPlan {
    /// Index into `Session::recordings`.
    pub recording: usize,
    /// Source name of the recording.
    pub source: String,
    /// Output name in `/acquisition`.
    pub name: String,
    /// Output description.
    pub description: String,
    /// Electrodes-table row of each channel for an `ElectricalSeries`; `None` for a `TimeSeries`.
    pub electrodes: Option<Vec<usize>>,
    /// Unit after `conversion`.
    pub unit: String,
    /// Multiplier from stored values to `unit` (shared gain and user conversion).
    pub conversion: f64,
}

/// One event series to write: an `EventsTable` in `/events`, or (multi-channel scalars) a
/// `TimeSeries` in `/acquisition`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EventPlan {
    /// Index into `Session::events`.
    pub event: usize,
    /// Source name of the event series.
    pub source: String,
    /// Output name.
    pub name: String,
    /// Output description.
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
    /// Source name of the snippet store.
    pub source: String,
    /// Series names are `<name>_ch<channel>`.
    pub name: String,
    /// Output description.
    pub description: String,
    /// Multiplier from the stored snippet values to volts.
    pub conversion: f64,
    /// `(source channel, electrodes-table row)` for every channel with snippets.
    pub rows: Vec<(u16, usize)>,
}

/// One table to write in `/analysis`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TablePlan {
    /// Index into `Session::tables`.
    pub table: usize,
    /// Output name.
    pub name: String,
    /// Output description.
    pub description: String,
}

/// Everything [`write`](crate::write()) writes, decided before writing, with the issues found.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct NwbPlan {
    /// Root and `/general` fields.
    pub file: FileFields,
    /// `/general/subject`.
    pub subject: SubjectFields,
    /// `/general/devices`.
    pub devices: Vec<Device>,
    /// Electrode groups.
    pub groups: Vec<GroupPlan>,
    /// Continuous series.
    pub series: Vec<SeriesPlan>,
    /// Event series.
    pub events: Vec<EventPlan>,
    /// Tables.
    pub tables: Vec<TablePlan>,
    /// Snippet stores.
    pub snippets: Vec<SnippetPlan>,
    /// Source items left out (by the metadata file).
    pub skipped: Vec<String>,
    /// Errors (block writing) and warnings.
    pub issues: Vec<Issue>,
}

impl NwbPlan {
    /// `true` when an issue blocks writing.
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

fn field(path: &str) -> Target {
    Target::Field(path.into())
}

/// Decides what to write for `session` under `meta`: names, units, storage, inclusion, file
/// fields, and the issues. Electrodes must already be in the session ([`crate::plan`] applies the
/// metadata file first). `new_identifier` is called only when the metadata gives no identifier.
pub fn resolve(session: &Session, meta: &MetadataFile, new_identifier: impl FnOnce() -> String) -> NwbPlan {
    let mut plan = NwbPlan::default();
    let issues = &mut plan.issues;
    let sm = &session.metadata;
    let ms = &meta.session;

    // File fields
    plan.file.description = ms.description.clone().unwrap_or_default();
    if plan.file.description.trim().is_empty() {
        issues.push(Issue::error("session.description is required (a sentence describing the session)").at(field("session.description")));
    }
    plan.file.identifier = ms.identifier.clone().unwrap_or_else(new_identifier);
    plan.file.start_time = match (&ms.start_time, &sm.start_time) {
        (Some(t), _) if has_zone(t) && parse_iso(t).is_some() => t.clone(),
        // A local time and a separate zone (what the app's date picker and zone list produce)
        (Some(t), _) if ms.timezone.as_deref().is_some_and(valid_zone) && parse_iso(t).is_some() => {
            format!("{}{}", parse_iso(t).map_or_else(|| t.clone(), format_iso), ms.timezone.as_deref().unwrap_or_default())
        }
        (Some(t), _) => {
            issues.push(Issue::error(format!("session.start_time {t:?} needs a time zone, e.g. 2025-02-26T15:25:56-05:00")).at(field("session.start_time")));
            t.clone()
        }
        (None, Some(rec)) if has_zone(rec) => rec.clone(),
        (None, Some(rec)) => match &ms.timezone {
            Some(z) if valid_zone(z) => format!("{}{z}", parse_iso(rec).map_or_else(|| rec.clone(), format_iso)),
            Some(z) => {
                issues.push(Issue::error(format!("session.timezone {z:?} must look like -05:00, +01:00 or Z")).at(field("session.timezone")));
                rec.clone()
            }
            None => {
                issues.push(
                    Issue::error(format!("the recorded start time {rec} has no time zone: set session.timezone (e.g. -05:00) or session.start_time"))
                        .at(field("session.timezone")),
                );
                rec.clone()
            }
        },
        (None, None) => {
            issues.push(Issue::error("no start time recorded: set session.start_time").at(field("session.start_time")));
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
        issues.push(Issue::warning("subject.species is missing (required by DANDI)").at(field("subject.species")).dandi());
    }
    if plan.subject.age.is_none() {
        issues.push(Issue::warning("subject.age is missing (required by DANDI, ISO 8601 e.g. P90D)").at(field("subject.age")).dandi());
    }
    if let Some(sex) = plan.subject.sex.as_ref().filter(|s| !["M", "F", "U", "O"].contains(&s.as_str())) {
        issues.push(Issue::warning(format!("subject.sex {sex:?} should be M, F, U or O")).at(field("subject.sex")).dandi());
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
                issues.push(
                    Issue::error(format!(
                        "stream {}: electrical, but {missing} of {} channels have no electrode; set streams.{}.electrode_group to a declared group",
                        info.name,
                        rows.len(),
                        info.name
                    ))
                    .at(Target::Stream(info.name.clone())),
                );
                None
            }
        };
        if electrical && spec.unit.as_deref().is_some_and(|u| u != "volts" && u != "V" && u != "a.u.") {
            issues.push(Issue::warning(format!("stream {}: electrical series are stored in volts; use conversion to scale", info.name)).at(Target::Stream(info.name.clone())));
        }
        // Values not known to be in physical units: written as stored unless a conversion is set
        if let (Calibration::Unknown { note }, None) = (&info.calibration, spec.conversion) {
            issues.push(
                Issue::warning(format!(
                    "stream {}: {note} and no conversion; values are written as stored. \
                     Set streams.{}.conversion (and unit) to the factor from stored units to the physical unit",
                    info.name, info.name
                ))
                .at(Target::Stream(info.name.clone())),
            );
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
            plan.issues.push(
                Issue::warning(format!(
                    "snippets {} ({} waveforms) have no electrodes and are not written: set snippets.{}.electrode_group to a declared group \
                     (NWB spike waveforms belong to electrodes)",
                    sn.name,
                    sn.len(),
                    sn.name
                ))
                .at(Target::Snippet(sn.name.clone())),
            );
            continue;
        }
        if sn.unit != "V" && spec.conversion.is_none() {
            plan.issues.push(
                Issue::warning(format!(
                    "snippets {}: values are in {} and no conversion is set; set snippets.{}.conversion to the factor to volts",
                    sn.name, sn.unit, sn.name
                ))
                .at(Target::Snippet(sn.name.clone())),
            );
        }
        let mut rows = Vec::new();
        for c in sn.channel_set() {
            match sn.electrodes.get(&c) {
                Some(&row) => rows.push((c, row)),
                None => plan.issues.push(Issue::warning(format!("snippets {}: channel {c} has no electrode; its snippets are not written", sn.name)).at(Target::Snippet(sn.name.clone()))),
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

    #[test]
    fn test_issues_point_at_what_fixes_them() {
        use nc_core::{MemoryRecording, StreamSpec};
        let mut s = Session::default();
        s.metadata.start_time = Some("2025-02-26T15:25:56".into());
        s.recordings.push(std::sync::Arc::new(MemoryRecording::new("ap", vec![0.0; 4], 2, 1000.0, "V").unwrap()));
        let mut meta = MetadataFile::default();
        meta.streams.insert("ap".into(), StreamSpec { kind: Some(StreamType::Electrical), ..Default::default() });
        let plan = resolve(&s, &meta, || "id".into());
        let targets: Vec<(Option<Target>, bool)> = plan.issues.iter().map(|i| (i.target.clone(), i.dandi)).collect();
        for t in [field("session.description"), field("session.timezone"), Target::Stream("ap".into())] {
            assert!(targets.contains(&(Some(t.clone()), false)), "{t:?} in {targets:?}");
        }
        assert!(targets.contains(&(Some(field("subject.species")), true)));
        assert!(plan.issues.iter().all(|i| i.target.is_some()), "{:?}", plan.issues);
    }

    #[test]
    fn test_local_start_time_takes_the_zone() {
        let mut meta = MetadataFile::default();
        meta.session.start_time = Some("2025-02-26T16:00:00".into());
        meta.session.timezone = Some("-05:00".into());
        let plan = resolve(&Session::default(), &meta, || "id".into());
        assert!(plan.file.start_time.starts_with("2025-02-26T16:00:00") && plan.file.start_time.ends_with("-05:00"), "{}", plan.file.start_time);
        assert!(!plan.issues.iter().any(|i| i.target == Some(field("session.start_time"))));
    }
}
