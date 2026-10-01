//! The Metadata step: session and subject. Typed inputs where the value has a format (start
//! date-time, time zone, age, sex), suggestions where values repeat (species, strain, lab…).
//! Listens to: RecordingOpened / MetadataReloaded (rebuild), PlanUpdated (field issues);
//! `NavEvent::Reveal(Field)` focuses the field.

use chrono::NaiveDateTime;
use gpui_kit::component::date_picker::{DatePickerEvent, DatePickerState};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::component::time_field::TimePrecision;
use gpui_kit::component::IndexPath;
use gpui_kit::{App, AppContext as _, Context, Entity, SharedString, Subscription, Window};
use nc_convert::core::{MetadataFile, Target};

use super::contents::merge;
use super::nav::{NavEvent, NavVm};
use crate::domain::format::{format_age, parse_age, strains, zone_label, AgeUnit, SPECIES, TIME_ZONES};
use crate::domain::AppEvent;
use crate::store::Store;

/// Text fields of the session and subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Description,
    Identifier,
    Experiment,
    Experimenters,
    Lab,
    Institution,
    Keywords,
    SubjectId,
    Species,
    Strain,
    SubjectDescription,
}

impl Field {
    /// Shown first.
    pub const ESSENTIAL: [Field; 3] = [Field::Description, Field::SubjectId, Field::Species];
    /// Under "More details".
    pub const MORE: [Field; 8] =
        [Field::Experiment, Field::Experimenters, Field::Lab, Field::Institution, Field::Keywords, Field::Strain, Field::SubjectDescription, Field::Identifier];

    pub fn label(self) -> &'static str {
        match self {
            Field::Description => "Description",
            Field::Identifier => "Identifier",
            Field::Experiment => "Experiment",
            Field::Experimenters => "Experimenters",
            Field::Lab => "Lab",
            Field::Institution => "Institution",
            Field::Keywords => "Keywords",
            Field::SubjectId => "Subject id",
            Field::Species => "Species",
            Field::Strain => "Strain",
            Field::SubjectDescription => "Subject description",
        }
    }

    pub fn required(self) -> bool {
        self == Field::Description
    }

    pub fn help(self) -> &'static str {
        match self {
            Field::Description => "One or two sentences about the session",
            Field::Identifier => "Generated (a UUID) when empty",
            Field::Experiment => "What the experiment is about",
            Field::Experimenters | Field::Keywords => "Comma-separated",
            Field::Species => "Latin name, e.g. Rattus norvegicus",
            Field::Strain => "e.g. Sprague Dawley",
            Field::SubjectId => "As named in your records",
            Field::Lab | Field::Institution | Field::SubjectDescription => "",
        }
    }

    /// The metadata-file path issues use for this field.
    pub fn path(self) -> &'static str {
        match self {
            Field::Description => "session.description",
            Field::Identifier => "session.identifier",
            Field::Experiment => "session.experiment_description",
            Field::Experimenters => "session.experimenters",
            Field::Lab => "session.lab",
            Field::Institution => "session.institution",
            Field::Keywords => "session.keywords",
            Field::SubjectId => "subject.id",
            Field::Species => "subject.species",
            Field::Strain => "subject.strain",
            Field::SubjectDescription => "subject.description",
        }
    }

    /// Field id in the window (tests, focus).
    pub fn id(self) -> String {
        format!("field-{self:?}")
    }

    pub fn get(self, m: &MetadataFile) -> Option<String> {
        let s = &m.session;
        let list = |v: &Vec<String>| (!v.is_empty()).then(|| v.join(", "));
        match self {
            Field::Description => s.description.clone(),
            Field::Identifier => s.identifier.clone(),
            Field::Experiment => s.experiment_description.clone(),
            Field::Experimenters => list(&s.experimenters),
            Field::Lab => s.lab.clone(),
            Field::Institution => s.institution.clone(),
            Field::Keywords => list(&s.keywords),
            Field::SubjectId => m.subject.id.clone(),
            Field::Species => m.subject.species.clone(),
            Field::Strain => m.subject.strain.clone(),
            Field::SubjectDescription => m.subject.description.clone(),
        }
    }

    pub fn set(self, m: &mut MetadataFile, value: &str) {
        let t = value.trim();
        let value = (!t.is_empty()).then(|| t.to_string());
        let list = |v: &Option<String>| v.as_deref().map_or_else(Vec::new, |t| t.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect());
        let s = &mut m.session;
        match self {
            Field::Description => s.description = value,
            Field::Identifier => s.identifier = value,
            Field::Experiment => s.experiment_description = value,
            Field::Experimenters => s.experimenters = list(&value),
            Field::Lab => s.lab = value,
            Field::Institution => s.institution = value,
            Field::Keywords => s.keywords = list(&value),
            Field::SubjectId => m.subject.id = value,
            Field::Species => m.subject.species = value,
            Field::Strain => m.subject.strain = value,
            Field::SubjectDescription => m.subject.description = value,
        }
    }
}

/// `2025-02-26T15:25:56[.123][±hh:mm|Z]` → the local date-time (seconds).
pub fn parse_local(iso: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(iso.get(..19)?, "%Y-%m-%dT%H:%M:%S").ok()
}

pub fn format_local(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// The zone written at the end of an ISO date-time, if any (`Z`, `-05:00`).
pub fn zone_suffix(iso: &str) -> Option<String> {
    let time = iso.split_once('T')?.1;
    if time.ends_with('Z') {
        return Some("Z".into());
    }
    let i = time.rfind(['+', '-'])?;
    Some(time[i..].to_string())
}

/// Index in [`TIME_ZONES`] of the zone in effect: the declared one, else the start time's.
pub fn zone_index(m: &MetadataFile, recorded: Option<&str>) -> Option<usize> {
    let zone = m.session.timezone.clone().or_else(|| m.session.start_time.as_deref().and_then(zone_suffix)).or_else(|| recorded.and_then(zone_suffix))?;
    let zone = if zone == "+00:00" { "Z".to_string() } else { zone };
    TIME_ZONES.iter().position(|(z, _)| *z == zone)
}

/// The start time after the picker shows `picked`: `None` when it is the recorded time.
pub fn start_value(picked: NaiveDateTime, recorded: Option<&str>) -> Option<String> {
    (recorded.and_then(parse_local) != Some(picked)).then(|| format_local(picked))
}

pub struct MetadataVm {
    store: Entity<Store>,
    pub fields: Vec<(Field, Entity<InputState>)>,
    pub start: Entity<DatePickerState>,
    pub zone: Entity<SelectState<SearchableVec<SharedString>>>,
    pub age: Entity<InputState>,
    pub age_unit: AgeUnit,
    pub age_error: Option<String>,
    /// "More details" is open.
    pub more: bool,
    pub has_recording: bool,
    inputs: Vec<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl MetadataVm {
    pub fn new(store: Entity<Store>, nav: Entity<NavVm>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let start = cx.new(|cx| DatePickerState::new(window, cx).time_precision(TimePrecision::Second).date_format("%Y-%m-%d"));
        let zones: Vec<SharedString> = TIME_ZONES.iter().map(|(z, p)| SharedString::from(zone_label(z, p))).collect();
        let zone = cx.new(|cx| SelectState::new(SearchableVec::new(zones), None, window, cx).searchable(true));
        let age = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. 90"));
        let subscriptions = vec![
            cx.subscribe_in(&store, window, |this, _, event: &AppEvent, window, cx| match event {
                AppEvent::RecordingOpened | AppEvent::MetadataReloaded => {
                    this.rebuild(window, cx);
                    cx.notify();
                }
                AppEvent::PlanUpdated | AppEvent::MetadataChanged => cx.notify(),
                _ => {}
            }),
            cx.subscribe_in(&nav, window, |this, _, event: &NavEvent, window, cx| {
                let NavEvent::Reveal(Target::Field(path)) = event else { return };
                this.focus(path, window, cx);
            }),
            cx.subscribe(&start, |this, _, event: &DatePickerEvent, cx| {
                let DatePickerEvent::Change(value) = event;
                let Some(picked) = value.start() else { return };
                let recorded = this.recorded(cx);
                let v = start_value(picked, recorded.as_deref());
                this.edit(cx, |m| m.session.start_time = v);
            }),
            cx.subscribe(&zone, |this, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
                let SelectEvent::Confirm(label) = event;
                let zone = label.as_ref().and_then(|l| TIME_ZONES.iter().find(|(z, p)| zone_label(z, p) == l.as_ref())).map(|(z, _)| z.to_string());
                this.edit(cx, |m| m.session.timezone = zone);
            }),
            cx.subscribe(&age, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = state.read(cx).value().to_string();
                    this.set_age_text(&text, cx);
                }
            }),
        ];
        let mut this = Self {
            store,
            fields: Vec::new(),
            start,
            zone,
            age,
            age_unit: AgeUnit::Days,
            age_error: None,
            more: false,
            has_recording: false,
            inputs: Vec::new(),
            _subscriptions: subscriptions,
        };
        this.rebuild(window, cx);
        this
    }

    fn recorded(&self, cx: &App) -> Option<String> {
        self.store.read(cx).ws.session().and_then(|s| s.metadata.start_time.clone())
    }

    /// Recreates the inputs from the metadata.
    fn rebuild(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let meta = self.store.read(cx).ws.meta.clone();
        let recorded = self.recorded(cx);
        self.has_recording = self.store.read(cx).ws.session().is_some();
        self.inputs.clear();
        self.fields = Field::ESSENTIAL
            .iter()
            .chain(&Field::MORE)
            .map(|&f| {
                let placeholder = SharedString::from(f.help().to_string());
                let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(f.get(&meta).unwrap_or_default()));
                self.inputs.push(cx.subscribe(&state, move |this, state, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let text = state.read(cx).value().to_string();
                        this.edit(cx, |m| f.set(m, &text));
                    }
                }));
                (f, state)
            })
            .collect();
        // Start: the declared time, else the recorded one
        if let Some(t) = meta.session.start_time.as_deref().or(recorded.as_deref()).and_then(parse_local) {
            self.start.update(cx, |s, cx| s.set_date_time(t, window, cx));
        }
        let zone = zone_index(&meta, recorded.as_deref());
        self.zone.update(cx, |s, cx| s.set_selected_index(zone.map(IndexPath::new), window, cx));
        let (text, unit) = match meta.subject.age.as_deref() {
            Some(a) => match parse_age(a) {
                Some((n, u)) => (n.to_string(), u),
                None => (a.to_string(), AgeUnit::Days),
            },
            None => (String::new(), AgeUnit::Days),
        };
        self.age_unit = unit;
        self.age_error = None;
        self.age.update(cx, |s, cx| s.set_value(text, window, cx));
        self.more = Field::MORE.iter().any(|f| f.get(&meta).is_some());
    }

    fn edit(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut MetadataFile)) {
        self.store.update(cx, |s, cx| {
            let mut meta = s.ws.meta.clone();
            f(&mut meta);
            s.apply(cx, |ws| ws.set_meta(meta));
        });
    }

    fn set_age_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let t = text.trim();
        let age = if t.is_empty() {
            Ok(None)
        } else if let Ok(n) = t.parse::<u32>() {
            Ok(Some(format_age(n, self.age_unit)))
        } else if t.starts_with('P') {
            Ok(Some(t.to_string()))
        } else {
            Err(format!("{t:?} is not a number"))
        };
        match age {
            Ok(a) => {
                self.age_error = None;
                self.edit(cx, |m| m.subject.age = a);
            }
            Err(e) => self.age_error = Some(e),
        }
        cx.notify();
    }

    pub fn set_age_unit(&mut self, unit: AgeUnit, cx: &mut Context<Self>) {
        self.age_unit = unit;
        let text = self.age.read(cx).value().to_string();
        self.set_age_text(&text, cx);
    }

    /// Back to the recorded start time.
    pub fn reset_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(t) = self.recorded(cx).as_deref().and_then(parse_local) {
            self.start.update(cx, |s, cx| s.set_date_time(t, window, cx));
        }
        self.edit(cx, |m| m.session.start_time = None);
    }

    /// The start time differs from the recorded one.
    pub fn start_overridden(&self, cx: &App) -> bool {
        self.store.read(cx).ws.meta.session.start_time.is_some()
    }

    pub fn recorded_start(&self, cx: &App) -> Option<String> {
        self.recorded(cx).map(|r| r.replace('T', " "))
    }

    pub fn sex(&self, cx: &App) -> Option<String> {
        self.store.read(cx).ws.meta.subject.sex.clone()
    }

    pub fn set_sex(&mut self, code: &str, cx: &mut Context<Self>) {
        let code = code.to_string();
        self.edit(cx, |m| m.subject.sex = Some(code));
    }

    pub fn dandi(&self, cx: &App) -> bool {
        self.store.read(cx).ws.settings.dandi
    }

    pub fn set_dandi(&mut self, on: bool, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| s.apply(cx, |ws| ws.set_dandi(on)));
        cx.notify();
    }

    pub fn toggle_more(&mut self, cx: &mut Context<Self>) {
        self.more = !self.more;
        cx.notify();
    }

    /// Messages of the issues about `path` (counted ones only).
    pub fn issues(&self, path: &str, cx: &App) -> Vec<(bool, String)> {
        let ws = &self.store.read(cx).ws;
        let target = Some(Target::Field(path.to_string()));
        ws.plan
            .as_ref()
            .map(|p| {
                p.issues
                    .iter()
                    .filter(|i| i.target == target && crate::domain::steps::counts(i, ws.settings.dandi))
                    .map(|i| (i.level == nc_convert::core::Level::Error, i.message.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// (label, value) suggestions of a field.
    pub fn suggestions(&self, field: Field, cx: &App) -> Vec<(String, String)> {
        let ws = &self.store.read(cx).ws;
        let remembered = |key: &str| ws.settings.remembered(key).iter().map(|v| (v.clone(), v.clone())).collect::<Vec<_>>();
        match field {
            Field::Species => {
                let mut out = remembered("species");
                for (latin, common) in SPECIES {
                    if !out.iter().any(|(_, v)| v == latin) {
                        out.push((format!("{latin} ({common})"), latin.to_string()));
                    }
                }
                out
            }
            Field::Strain => {
                let species = ws.meta.subject.species.clone().unwrap_or_default();
                merge(ws.settings.remembered("strain"), strains(&species).iter().map(|s| s.to_string())).into_iter().map(|v| (v.clone(), v)).collect()
            }
            Field::Lab => remembered("lab"),
            Field::Institution => remembered("institution"),
            Field::Experimenters => remembered("experimenter"),
            _ => Vec::new(),
        }
    }

    /// Picking a suggestion: replaces the value, or (lists) adds to it.
    pub fn pick(&mut self, field: Field, value: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, state)) = self.fields.iter().find(|(f, _)| *f == field) else { return };
        let current = state.read(cx).value().to_string();
        let text = if field == Field::Experimenters && !current.trim().is_empty() {
            if current.split(',').any(|v| v.trim() == value) { current } else { format!("{}, {value}", current.trim_end_matches([',', ' '])) }
        } else {
            value
        };
        state.update(cx, |s, cx| s.replace_all(text, window, cx));
    }

    fn focus(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let field = Field::ESSENTIAL.iter().chain(&Field::MORE).find(|f| f.path() == path).copied();
        if let Some(f) = field {
            if Field::MORE.contains(&f) {
                self.more = true;
            }
            if let Some((_, s)) = self.fields.iter().find(|(x, _)| *x == f) {
                s.update(cx, |s, cx| s.focus(window, cx));
            }
        } else if path == "subject.age" {
            self.age.update(cx, |s, cx| s.focus(window, cx));
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_values() {
        let t = parse_local("2025-02-26T15:25:56.25-05:00").unwrap();
        assert_eq!(format_local(t), "2025-02-26T15:25:56");
        assert_eq!(zone_suffix("2025-02-26T15:25:56-05:00").as_deref(), Some("-05:00"));
        assert_eq!(zone_suffix("2025-02-26T15:25:56Z").as_deref(), Some("Z"));
        assert_eq!(zone_suffix("2025-02-26T15:25:56"), None);
        assert_eq!(start_value(t, Some("2025-02-26T15:25:56")), None, "the recorded time is not an override");
        assert_eq!(start_value(t, Some("2025-02-26T15:00:00")).as_deref(), Some("2025-02-26T15:25:56"));

        let mut m = MetadataFile::default();
        assert_eq!(zone_index(&m, Some("2025-02-26T15:25:56")), None);
        m.session.timezone = Some("-05:00".into());
        assert_eq!(TIME_ZONES[zone_index(&m, None).unwrap()].0, "-05:00");
        m.session.timezone = None;
        assert_eq!(TIME_ZONES[zone_index(&m, Some("2025-02-26T15:25:56+00:00")).unwrap()].0, "Z");
    }

    #[test]
    fn test_fields_round_trip() {
        let mut m = MetadataFile::default();
        Field::Keywords.set(&mut m, "a, b,,");
        Field::Lab.set(&mut m, "  ");
        assert_eq!(m.session.keywords, vec!["a", "b"]);
        assert_eq!(Field::Keywords.get(&m).as_deref(), Some("a, b"));
        assert_eq!(m.session.lab, None);
    }
}
