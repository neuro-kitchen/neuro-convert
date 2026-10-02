//! The Contents step: the tree of the recording (include checkboxes, issue flags) and a card for
//! the selected item — for a stream: what kind of signal it is, its electrodes, and its details.
//! Listens to: RecordingOpened (tree), MetadataReloaded (card fields), MetadataChanged and
//! PlanUpdated (card and flags), PreviewRequested (a stream picked in the preview becomes the
//! selection); `NavEvent::Reveal` selects an item.

use std::collections::{HashMap, HashSet};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::tree::{TreeItem, TreeState};
use gpui_kit::{App, AppContext as _, Context, Entity, ScrollStrategy, SharedString, Subscription, Window};
use nc_convert::core::{Calibration, ElectrodeGroupSpec, Issue, ItemKind, MetadataFile, Session, StreamType, Target};

use super::nav::{NavEvent, NavVm};
use crate::domain::format::{LOCATIONS, UNITS};
use crate::domain::AppEvent;
use crate::store::Store;
use crate::widgets::{duration, Inclusion, TableData};

/// What the card shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    None,
    Stream(String),
    Event(String),
    Table(String),
    Snippet(String),
    Group(String),
}

impl Selection {
    fn of(id: &str) -> Self {
        match id.split_once('/') {
            Some(("s", n)) => Selection::Stream(n.into()),
            Some(("e", n)) => Selection::Event(n.into()),
            Some(("t", n)) => Selection::Table(n.into()),
            Some(("n", n)) => Selection::Snippet(n.into()),
            Some(("g", n)) => Selection::Group(n.into()),
            _ => Selection::None,
        }
    }

    pub fn target(&self) -> Option<Target> {
        Some(match self {
            Selection::Stream(n) => Target::Stream(n.clone()),
            Selection::Event(n) => Target::Event(n.clone()),
            Selection::Table(n) => Target::Table(n.clone()),
            Selection::Snippet(n) => Target::Snippet(n.clone()),
            Selection::Group(n) => Target::ElectrodeGroup(n.clone()),
            Selection::None => return None,
        })
    }
}

/// The tree id of what an issue is about.
pub fn tree_id(target: &Target) -> Option<String> {
    Some(match target {
        Target::Stream(n) => format!("s/{n}"),
        Target::Event(n) => format!("e/{n}"),
        Target::Table(n) => format!("t/{n}"),
        Target::Snippet(n) => format!("n/{n}"),
        Target::ElectrodeGroup(n) => format!("g/{n}"),
        Target::Field(_) => return None,
    })
}

/// One tree row: name, a muted detail and a tag.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub title: String,
    pub detail: String,
}

/// Everything the stream card shows (built by [`stream_card`]).
#[derive(Debug, Clone, PartialEq)]
pub struct StreamCard {
    pub name: String,
    pub summary: String,
    /// As declared (`None` = automatic).
    pub kind: Option<StreamType>,
    /// What it is written as.
    pub neural: bool,
    /// The reader supplies an electrode for every channel (e.g. a Neuropixels probe).
    pub from_recording: Option<String>,
    /// The electrode group the channels belong to.
    pub group: Option<String>,
    /// Groups that can be chosen.
    pub groups: Vec<String>,
    /// Other streams on the same group (editing the group changes them too).
    pub shared_with: Vec<String>,
    /// The reader could not tell the physical scale.
    pub scale_note: Option<String>,
    /// Automatic made it an other signal, but it looks like electrode data (many channels at a
    /// spike-band rate): the user should decide. Never applied on its own.
    pub looks_electrical: bool,
}

pub fn stream_card(s: &Session, meta: &MetadataFile, name: &str) -> Option<StreamCard> {
    let i = s.recording(name)?.info();
    let spec = meta.stream(name);
    let rows = s.channel_electrodes(name);
    let complete = !rows.is_empty() && rows.iter().all(Option::is_some);
    let reader_groups: Vec<String> = {
        let mut g: Vec<String> = rows.iter().flatten().map(|&e| s.electrodes[e].group.clone()).collect();
        g.dedup();
        g
    };
    let mut groups: Vec<String> = meta.electrode_groups.iter().map(|g| g.name.clone()).collect();
    for g in s.electrode_groups.iter().map(|g| &g.name).chain(&reader_groups) {
        if !groups.contains(g) {
            groups.push(g.clone());
        }
    }
    let group = spec.electrode_group.clone().or_else(|| reader_groups.first().cloned());
    let shared_with = group
        .as_ref()
        .map(|g| s.recordings.iter().map(|r| r.info().name.clone()).filter(|n| n != name && meta.stream(n).electrode_group.as_ref() == Some(g)).collect())
        .unwrap_or_default();
    let neural = match spec.kind {
        Some(k) => k == StreamType::Electrical,
        None => complete,
    };
    Some(StreamCard {
        looks_electrical: spec.kind.is_none() && !neural && i.channel_count() >= 4 && i.sample_rate >= 10_000.0,
        name: name.to_string(),
        summary: format!("{} ch · {} Hz · {} · {}", i.channel_count(), group_digits(i.sample_rate), duration(i.duration()), i.unit),
        kind: spec.kind,
        neural,
        from_recording: complete.then(|| format!("{} electrodes from the recording ({})", rows.len(), reader_groups.join(", "))),
        group,
        groups,
        shared_with,
        scale_note: match &i.calibration {
            Calibration::Unknown { note } => Some(note.clone()),
            _ => None,
        },
    })
}

/// `24414.0625` → `24 414`.
fn group_digits(rate: f64) -> String {
    let n = rate.round() as u64;
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// The line above the selected item: what it is, its name, facts, and where it goes in the NWB
/// file.
#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    pub kind: &'static str,
    pub name: String,
    pub facts: String,
    /// `ElectricalSeries`, `events table`, `left out`.
    pub dest: String,
}

pub fn heading(s: &Session, meta: &MetadataFile, sel: &Selection) -> Option<Heading> {
    let left_out = |kind, name: &str, dest: String| if meta.included(kind, name) { dest } else { "left out".to_string() };
    Some(match sel {
        Selection::Stream(n) => {
            let card = stream_card(s, meta, n)?;
            let dest = if card.neural { "ElectricalSeries" } else { "TimeSeries" };
            Heading { kind: "Stream", name: n.clone(), facts: card.summary, dest: left_out(ItemKind::Stream, n, dest.into()) }
        }
        Selection::Event(n) => {
            let e = s.events.iter().find(|e| &e.name == n)?;
            let dest = if e.channels > 1 { format!("TimeSeries ({} values per event)", e.channels) } else { "events table".into() };
            let facts = format!("{} events{}", e.len(), if e.offsets.is_some() { " with durations" } else { "" });
            Heading { kind: "Events", name: n.clone(), facts, dest: left_out(ItemKind::Event, n, dest) }
        }
        Selection::Table(n) => {
            let t = s.tables.iter().find(|t| &t.name == n)?;
            Heading { kind: "Table", name: n.clone(), facts: format!("{} rows × {} columns", t.rows.len(), t.columns.len()), dest: left_out(ItemKind::Table, n, "analysis table".into()) }
        }
        Selection::Snippet(n) => {
            let sn = s.snippets.iter().find(|x| &x.name == n)?;
            let mut channels: Vec<u16> = sn.channels.clone();
            channels.sort_unstable();
            channels.dedup();
            let facts = format!("{} waveforms × {} samples on {} channels", sn.len(), sn.samples_per_snippet, channels.len());
            Heading { kind: "Snippets", name: n.clone(), facts, dest: left_out(ItemKind::Snippet, n, "SpikeEventSeries per channel".into()) }
        }
        Selection::Group(n) => {
            let count = s.electrodes.iter().filter(|e| &e.group == n).count();
            Heading { kind: "Electrode group", name: n.clone(), facts: format!("{count} electrodes"), dest: "electrodes table".into() }
        }
        Selection::None => return None,
    })
}

fn num(v: f64) -> String {
    if v.is_finite() && v == v.trunc() && v.abs() < 1e15 { format!("{v:.0}") } else { format!("{v}") }
}

/// The rows of a selected event series, table, snippet store or electrode group.
pub fn table_data(s: &Session, sel: &Selection) -> Option<TableData> {
    let col = |n: &str, numeric: bool| (n.to_string(), numeric);
    match sel {
        Selection::Event(n) => {
            let e = s.events.iter().find(|e| &e.name == n)?;
            let mut columns = vec![col("#", true), col("Onset (s)", true)];
            if e.offsets.is_some() {
                columns.extend([col("Offset (s)", true), col("Duration (s)", true)]);
            }
            let c = e.channels.max(1);
            if c == 1 {
                columns.push(col("Value", true));
            } else {
                columns.extend((1..=c).map(|i| col(&format!("Value {i}"), true)));
            }
            let labelled = e.labels.iter().any(|l| !l.is_empty());
            if labelled {
                columns.push(col("Label", false));
            }
            let rows = (0..e.len())
                .map(|i| {
                    let mut r = vec![(i + 1).to_string(), format!("{:.6}", e.onsets[i])];
                    if let Some(off) = &e.offsets {
                        r.extend([format!("{:.6}", off[i]), format!("{:.6}", off[i] - e.onsets[i])]);
                    }
                    r.extend(e.values.get(i * c..(i + 1) * c).unwrap_or(&[]).iter().map(|v| num(*v)));
                    if labelled {
                        r.push(e.labels.get(i).cloned().unwrap_or_default());
                    }
                    r
                })
                .collect();
            Some(TableData { columns, rows })
        }
        Selection::Table(n) => {
            let t = s.tables.iter().find(|t| &t.name == n)?;
            let columns = (0..t.columns.len())
                .map(|i| (t.columns[i].clone(), !t.rows.is_empty() && t.rows.iter().all(|r| r.get(i).is_some_and(|c| c.trim().parse::<f64>().is_ok()))))
                .collect();
            Some(TableData { columns, rows: t.rows.clone() })
        }
        Selection::Snippet(n) => {
            let sn = s.snippets.iter().find(|x| &x.name == n)?;
            let mut channels: Vec<u16> = sn.channels.clone();
            channels.sort_unstable();
            channels.dedup();
            let rows = channels
                .iter()
                .map(|&c| {
                    let mine: Vec<usize> = (0..sn.len()).filter(|&i| sn.channels[i] == c).collect();
                    let mut codes: Vec<u16> = mine.iter().filter_map(|&i| sn.sort_codes.get(i).copied()).filter(|&x| x != 0).collect();
                    codes.sort_unstable();
                    codes.dedup();
                    let electrode = sn.electrodes.get(&c).and_then(|&e| s.electrodes.get(e)).map_or_else(|| "—".to_string(), |e| e.name.clone());
                    vec![c.to_string(), mine.len().to_string(), if codes.is_empty() { "unsorted".into() } else { codes.iter().map(u16::to_string).collect::<Vec<_>>().join(", ") }, electrode]
                })
                .collect();
            Some(TableData { columns: vec![col("Channel", true), col("Waveforms", true), col("Sort codes", false), col("Electrode", false)], rows })
        }
        Selection::Group(n) => {
            let pos = |e: &nc_convert::core::Electrode, k: usize| e.position_um.map_or_else(String::new, |p| num(p[k] as f64));
            let rows = s
                .electrodes
                .iter()
                .filter(|e| &e.group == n)
                .map(|e| {
                    let channels = e.channels.iter().map(|c| format!("{} {}", c.recording, c.channel + 1)).collect::<Vec<_>>().join(", ");
                    vec![e.name.clone(), channels, pos(e, 0), pos(e, 1), pos(e, 2), e.impedance_ohms.map_or_else(String::new, |z| format!("{:.1}", z / 1000.0))]
                })
                .collect();
            Some(TableData { columns: vec![col("Electrode", false), col("Channels", false), col("x (µm)", true), col("y (µm)", true), col("z (µm)", true), col("Impedance (kΩ)", true)], rows })
        }
        _ => None,
    }
}

/// The tree. Ids: `f/<folder>`, `s/` streams, `e/` events, `n/` snippets, `t/` tables,
/// `g/` electrode groups, `d/` devices. Labels are names; details are kept apart.
pub fn tree(s: &Session) -> (Vec<TreeItem>, HashMap<String, Row>) {
    let mut rows = HashMap::new();
    let mut leaf = |id: String, title: &str, detail: String| {
        rows.insert(id.clone(), Row { title: title.into(), detail });
        TreeItem::new(id, title.to_string())
    };
    let streams: Vec<TreeItem> = s
        .recordings
        .iter()
        .map(|r| {
            let i = r.info();
            leaf(format!("s/{}", i.name), &i.name, format!("{} ch · {} Hz", i.channel_count(), group_digits(i.sample_rate)))
        })
        .collect();
    let events: Vec<TreeItem> = s.events.iter().map(|e| leaf(format!("e/{}", e.name), &e.name, format!("{} events", e.len()))).collect();
    let snippets: Vec<TreeItem> = s.snippets.iter().map(|n| leaf(format!("n/{}", n.name), &n.name, format!("{} waveforms", n.len()))).collect();
    let tables: Vec<TreeItem> = s.tables.iter().map(|t| leaf(format!("t/{}", t.name), &t.name, format!("{} rows", t.rows.len()))).collect();
    let groups: Vec<TreeItem> = s
        .electrode_groups
        .iter()
        .map(|g| leaf(format!("g/{}", g.name), &g.name, format!("{} electrodes", s.electrodes.iter().filter(|e| e.group == g.name).count())))
        .collect();
    let mut out = Vec::new();
    for (id, title, children) in [("streams", "Streams", streams), ("events", "Events", events), ("snippets", "Snippets", snippets), ("tables", "Tables", tables), ("electrodes", "Electrode groups", groups)] {
        if children.is_empty() && id != "streams" {
            continue;
        }
        let id = format!("f/{id}");
        rows.insert(id.clone(), Row { title: title.into(), detail: children.len().to_string() });
        out.push(TreeItem::new(id, title.to_string()).children(children).expanded(true));
    }
    (out, rows)
}

/// The items a tree node's checkbox includes or leaves out; `None` for nodes without one.
pub fn selection(id: &str, s: &Session) -> Option<(ItemKind, Vec<String>)> {
    let (prefix, name) = id.split_once('/')?;
    let one = |kind| Some((kind, vec![name.to_string()]));
    match (prefix, name) {
        ("f", "streams") => Some((ItemKind::Stream, s.recordings.iter().map(|r| r.info().name.clone()).collect())),
        ("f", "events") => Some((ItemKind::Event, s.events.iter().map(|e| e.name.clone()).collect())),
        ("f", "snippets") => Some((ItemKind::Snippet, s.snippets.iter().map(|n| n.name.clone()).collect())),
        ("f", "tables") => Some((ItemKind::Table, s.tables.iter().map(|t| t.name.clone()).collect())),
        ("s", _) => one(ItemKind::Stream),
        ("e", _) => one(ItemKind::Event),
        ("n", _) => one(ItemKind::Snippet),
        ("t", _) => one(ItemKind::Table),
        _ => None,
    }
}

/// Text inputs of the selected item's card.
#[derive(Default)]
pub struct CardFields {
    pub name: Option<Entity<InputState>>,
    pub unit: Option<Entity<InputState>>,
    pub conversion: Option<Entity<InputState>>,
    pub group_location: Option<Entity<InputState>>,
    pub group_description: Option<Entity<InputState>>,
    pub group_device: Option<Entity<InputState>>,
}

pub struct ContentsVm {
    store: Entity<Store>,
    pub tree: Entity<TreeState>,
    pub rows: HashMap<String, Row>,
    /// Tree ids with an issue.
    pub flagged: HashSet<String>,
    pub selected: Selection,
    pub fields: CardFields,
    /// Card values that could not be read (e.g. a scale that is not a number).
    pub errors: Vec<String>,
    inputs: Vec<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl ContentsVm {
    pub fn new(store: Entity<Store>, nav: Entity<NavVm>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let subscriptions = vec![
            cx.subscribe_in(&store, window, |this, _, event: &AppEvent, window, cx| match event {
                AppEvent::RecordingOpened => {
                    this.load_tree(cx);
                    this.select_first_stream(window, cx);
                }
                AppEvent::MetadataReloaded => {
                    this.rebuild_fields(window, cx);
                    cx.notify();
                }
                AppEvent::PlanUpdated => {
                    this.flag(cx);
                    cx.notify();
                }
                AppEvent::MetadataChanged => cx.notify(),
                // A stream picked elsewhere (the preview's dropdown) becomes the selection, so the
                // tree, heading and settings follow it
                AppEvent::PreviewRequested => {
                    let preview = this.store.read(cx).ws.preview.clone();
                    if let Some(name) = preview.filter(|n| this.selected != Selection::Stream(n.clone())) {
                        this.select_id(&format!("s/{name}"), window, cx);
                    }
                }
                _ => {}
            }),
            cx.observe_in(&tree, window, |this, tree, window, cx| {
                let selected = tree.read(cx).selected_item().map_or(Selection::None, |i| Selection::of(&i.id));
                if selected != this.selected {
                    this.selected = selected;
                    this.on_selected(window, cx);
                }
            }),
            cx.subscribe_in(&nav, window, |this, _, event: &NavEvent, window, cx| {
                let NavEvent::Reveal(target) = event;
                if let Some(id) = tree_id(target) {
                    this.select_id(&id, window, cx);
                }
            }),
        ];
        let mut this = Self {
            store,
            tree,
            rows: HashMap::new(),
            flagged: HashSet::new(),
            selected: Selection::None,
            fields: CardFields::default(),
            errors: Vec::new(),
            inputs: Vec::new(),
            _subscriptions: subscriptions,
        };
        if this.store.read(cx).ws.session().is_some() {
            this.load_tree(cx);
            this.select_first_stream(window, cx);
        }
        this
    }

    pub fn store(&self) -> &Entity<Store> {
        &self.store
    }

    fn load_tree(&mut self, cx: &mut Context<Self>) {
        let Some((items, rows)) = self.store.read(cx).ws.session().map(tree) else { return };
        self.rows = rows;
        self.tree.update(cx, |t, cx| t.set_items(items, cx));
        self.flag(cx);
    }

    fn flag(&mut self, cx: &App) {
        let ws = &self.store.read(cx).ws;
        self.flagged = ws.plan.as_ref().map(|p| p.issues.iter().filter(|i| crate::domain::steps::counts(i, ws.settings.dandi)).filter_map(|i| tree_id(i.target.as_ref()?)).collect()).unwrap_or_default();
    }

    fn select_first_stream(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let first = self.store.read(cx).ws.session().and_then(|s| {
            // The previewed stream (a neural one when there is one), else the first
            let ws = &self.store.read(cx).ws;
            ws.preview.clone().or_else(|| s.recordings.first().map(|r| r.info().name.clone()))
        });
        if let Some(name) = first {
            self.select_id(&format!("s/{name}"), window, cx);
        }
    }

    pub fn select_id(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = SharedString::from(id.to_string());
        self.tree.update(cx, |t, cx| {
            t.reveal_item(&id, ScrollStrategy::Center, cx);
            let ix = t.index_of(&id);
            t.set_selected_index(ix, cx);
        });
        let selected = Selection::of(&id);
        if selected != self.selected {
            self.selected = selected;
            self.on_selected(window, cx);
        }
    }

    fn on_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Selection::Stream(name) = &self.selected {
            let name = name.clone();
            self.store.update(cx, |s, cx| s.apply(cx, |ws| ws.request_preview(&name)));
        }
        self.rebuild_fields(window, cx);
        cx.notify();
    }

    fn input(&mut self, value: Option<String>, placeholder: &str, window: &mut Window, cx: &mut Context<Self>, on_change: fn(&mut Self, String, &mut Context<Self>)) -> Entity<InputState> {
        let placeholder = SharedString::from(placeholder.to_string());
        let state = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value.unwrap_or_default()));
        self.inputs.push(cx.subscribe(&state, move |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).value().to_string();
                on_change(this, text, cx);
            }
        }));
        state
    }

    /// Recreates the card's inputs for the selection (not on every metadata change: typing
    /// would lose its caret).
    fn rebuild_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inputs.clear();
        self.errors.clear();
        self.fields = CardFields::default();
        let meta = self.store.read(cx).ws.meta.clone();
        match self.selected.clone() {
            Selection::Stream(name) => {
                let spec = meta.stream(&name);
                self.fields.name = Some(self.input(spec.name, &name, window, cx, |this, v, cx| this.edit_stream(cx, |s| s.name = text(&v))));
                self.fields.unit = Some(self.input(spec.unit, "as recorded", window, cx, |this, v, cx| this.edit_stream(cx, |s| s.unit = text(&v))));
                self.fields.conversion = Some(self.input(spec.conversion.map(|c| c.to_string()), "1 (as recorded)", window, cx, |this, v, cx| {
                    match text(&v).map(|t| t.parse::<f64>()) {
                        Some(Err(_)) => this.errors = vec![format!("Scale factor {v:?} is not a number")],
                        parsed => {
                            this.errors.clear();
                            let c = parsed.and_then(Result::ok);
                            this.edit_stream(cx, |s| s.conversion = c);
                        }
                    }
                    cx.notify();
                }));
                self.rebuild_group_fields(window, cx);
            }
            Selection::Event(name) => {
                let v = meta.event(&name).name;
                self.fields.name = Some(self.input(v, &name, window, cx, |this, v, cx| this.edit_item(ItemKind::Event, v, cx)));
            }
            Selection::Table(name) => {
                let v = meta.table(&name).name;
                self.fields.name = Some(self.input(v, &name, window, cx, |this, v, cx| this.edit_item(ItemKind::Table, v, cx)));
            }
            Selection::Snippet(name) => {
                let spec = meta.snippet(&name);
                self.fields.name = Some(self.input(spec.name, &name, window, cx, |this, v, cx| this.edit_item(ItemKind::Snippet, v, cx)));
                self.fields.conversion = Some(self.input(spec.conversion.map(|c| c.to_string()), "factor to volts", window, cx, |this, v, cx| {
                    let Selection::Snippet(name) = this.selected.clone() else { return };
                    let c = text(&v).and_then(|t| t.parse::<f64>().ok());
                    this.edit_meta(cx, |m| m.snippets.entry(name).or_default().conversion = c);
                }));
            }
            Selection::Group(_) => self.rebuild_group_fields(window, cx),
            Selection::None => {}
        }
    }

    /// The group being edited: the selected group, or the selected stream's group.
    pub fn edited_group(&self, cx: &App) -> Option<String> {
        match &self.selected {
            Selection::Group(g) => Some(g.clone()),
            Selection::Stream(_) => self.card(cx).and_then(|c| c.group),
            _ => None,
        }
    }

    fn rebuild_group_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group) = self.edited_group(cx) else {
            (self.fields.group_location, self.fields.group_description, self.fields.group_device) = (None, None, None);
            return;
        };
        let ws = &self.store.read(cx).ws;
        let declared = ws.meta.electrode_groups.iter().find(|g| g.name == group).cloned();
        let reader = ws.session().and_then(|s| s.electrode_group(&group).cloned());
        let location = declared.as_ref().map(|g| g.location.clone()).or_else(|| reader.as_ref().map(|g| g.location.clone()));
        let description = declared.as_ref().map(|g| g.description.clone()).or_else(|| reader.as_ref().map(|g| g.description.clone()));
        let device = declared.as_ref().and_then(|g| g.device.clone()).or_else(|| reader.as_ref().and_then(|g| g.device.clone()));
        self.fields.group_location = Some(self.input(location, "brain area, e.g. M1", window, cx, |this, v, cx| this.edit_group(cx, |g| g.location = v.trim().to_string())));
        self.fields.group_description = Some(self.input(description, "e.g. 32-channel array", window, cx, |this, v, cx| this.edit_group(cx, |g| g.description = v.trim().to_string())));
        self.fields.group_device = Some(self.input(device, "first device of the session", window, cx, |this, v, cx| this.edit_group(cx, |g| g.device = text(&v))));
    }

    fn edit_meta(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut MetadataFile)) {
        self.store.update(cx, |s, cx| {
            let mut meta = s.ws.meta.clone();
            f(&mut meta);
            s.apply(cx, |ws| ws.set_meta(meta));
        });
    }

    fn edit_stream(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut nc_convert::core::StreamSpec)) {
        let Selection::Stream(name) = self.selected.clone() else { return };
        self.edit_meta(cx, |m| {
            let e = m.streams.entry(name.clone()).or_default();
            f(e);
            if *e == Default::default() {
                m.streams.remove(&name);
            }
        });
    }

    fn edit_item(&mut self, kind: ItemKind, value: String, cx: &mut Context<Self>) {
        let name = match &self.selected {
            Selection::Event(n) | Selection::Table(n) | Selection::Snippet(n) => n.clone(),
            _ => return,
        };
        self.edit_meta(cx, |m| match kind {
            ItemKind::Snippet => m.snippets.entry(name).or_default().name = text(&value),
            ItemKind::Event => m.events.entry(name).or_default().name = text(&value),
            _ => m.tables.entry(name).or_default().name = text(&value),
        });
    }

    /// Edits the declared spec of the edited group (declaring it when it came from the reader).
    fn edit_group(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut ElectrodeGroupSpec)) {
        let Some(group) = self.edited_group(cx) else { return };
        self.edit_meta(cx, |m| f(group_spec(m, &group)));
    }

    pub fn card(&self, cx: &App) -> Option<StreamCard> {
        let Selection::Stream(name) = &self.selected else { return None };
        let ws = &self.store.read(cx).ws;
        stream_card(ws.session()?, &ws.meta, name)
    }

    pub fn set_kind(&mut self, kind: Option<StreamType>, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_stream(cx, |s| s.kind = kind);
        self.rebuild_group_fields(window, cx);
        cx.notify();
    }

    /// Puts the selected stream's channels in `group`.
    pub fn set_group(&mut self, group: String, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_stream(cx, |s| s.electrode_group = Some(group.clone()));
        self.edit_meta(cx, |m| {
            group_spec(m, &group);
        });
        self.rebuild_group_fields(window, cx);
        cx.notify();
    }

    /// A new group for the selected stream (`group1`, `group2`, …).
    pub fn new_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let taken = self.card(cx).map(|c| c.groups).unwrap_or_default();
        let name = (1..).map(|i| format!("group{i}")).find(|n| !taken.contains(n)).expect("unbounded");
        self.set_group(name, window, cx);
    }

    /// The include checkbox of tree item `id`: kind, item names and their state.
    pub fn inclusion(&self, id: &str, cx: &App) -> Option<(ItemKind, Vec<String>, Inclusion)> {
        let ws = &self.store.read(cx).ws;
        let (kind, names) = selection(id, ws.session()?)?;
        let included = names.iter().filter(|n| ws.included(kind, n)).count();
        Some((kind, names.clone(), Inclusion::of(included, names.len())))
    }

    pub fn set_included(store: &Entity<Store>, kind: ItemKind, names: Vec<String>, include: bool, cx: &mut App) {
        store.update(cx, |s, cx| s.apply(cx, |ws| ws.set_included(kind, &names, include)));
    }

    pub fn heading(&self, cx: &App) -> Option<Heading> {
        let ws = &self.store.read(cx).ws;
        heading(ws.session()?, &ws.meta, &self.selected)
    }

    pub fn table_data(&self, cx: &App) -> Option<TableData> {
        table_data(self.store.read(cx).ws.session()?, &self.selected)
    }

    /// The signal-kind tag of a stream row (`neural` / `other`).
    pub fn stream_tag(&self, name: &str, cx: &App) -> Option<&'static str> {
        let ws = &self.store.read(cx).ws;
        let card = stream_card(ws.session()?, &ws.meta, name)?;
        Some(if card.neural {
            "ElectricalSeries"
        } else if card.looks_electrical {
            "TimeSeries?"
        } else {
            "TimeSeries"
        })
    }

    /// Issues about the selected item.
    pub fn issues(&self, cx: &App) -> Vec<Issue> {
        let Some(target) = self.selected.target() else { return Vec::new() };
        let ws = &self.store.read(cx).ws;
        let group = self.edited_group(cx).map(Target::ElectrodeGroup);
        ws.plan
            .as_ref()
            .map(|p| p.issues.iter().filter(|i| crate::domain::steps::counts(i, ws.settings.dandi) && (i.target == Some(target.clone()) || (group.is_some() && i.target == group))).cloned().collect())
            .unwrap_or_default()
    }

    /// Location suggestions: typed before, then common areas.
    pub fn location_suggestions(&self, cx: &App) -> Vec<String> {
        let ws = &self.store.read(cx).ws;
        merge(ws.settings.remembered("location"), LOCATIONS.iter().map(|s| s.to_string()))
    }

    pub fn device_suggestions(&self, cx: &App) -> Vec<String> {
        let ws = &self.store.read(cx).ws;
        let session = ws.session().map(|s| s.metadata.devices.iter().map(|d| d.name.clone()).collect::<Vec<_>>()).unwrap_or_default();
        merge(ws.settings.remembered("device"), session)
    }

    pub fn unit_suggestions(&self) -> Vec<String> {
        UNITS.iter().map(|u| u.to_string()).collect()
    }
}

/// The declared spec of `group`, added (from the reader's group when there is one) if missing.
fn group_spec<'a>(m: &'a mut MetadataFile, group: &str) -> &'a mut ElectrodeGroupSpec {
    match m.electrode_groups.iter().position(|g| g.name == group) {
        Some(i) => &mut m.electrode_groups[i],
        None => {
            m.electrode_groups.push(ElectrodeGroupSpec { name: group.to_string(), ..Default::default() });
            m.electrode_groups.last_mut().expect("just pushed")
        }
    }
}

fn text(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// `first` then `rest`, without duplicates.
pub fn merge(first: &[String], rest: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = first.to_vec();
    for v in rest {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace::tests::{open, scratch, workspace};

    #[test]
    fn test_tree_rows_and_selection() {
        let mut ws = workspace(&scratch("contents-tree"));
        open(&mut ws, "session.fake");
        let s = ws.session().unwrap();
        let (items, rows) = tree(s);
        assert_eq!(items.iter().map(|i| i.id.to_string()).collect::<Vec<_>>(), vec!["f/streams", "f/events"]);
        assert_eq!(rows["s/Wav1"], Row { title: "Wav1".into(), detail: "2 ch · 1 000 Hz".into() });
        assert_eq!(selection("f/streams", s).unwrap().1, vec!["Wav1", "Temp"]);
        assert!(selection("d/x", s).is_none());
        assert_eq!(Selection::of("s/Wav1").target(), Some(Target::Stream("Wav1".into())));
        assert_eq!(tree_id(&Target::Event("Tick".into())).as_deref(), Some("e/Tick"));
    }

    #[test]
    fn test_stream_card_follows_the_metadata() {
        let mut ws = workspace(&scratch("contents-card"));
        open(&mut ws, "session.fake");
        let card = stream_card(ws.session().unwrap(), &ws.meta, "Wav1").unwrap();
        assert_eq!((card.kind, card.neural, card.group.clone(), card.from_recording.clone()), (None, false, None, None));
        assert_eq!(card.summary, "2 ch · 1 000 Hz · 2.00 s · V");

        let mut meta = ws.meta.clone();
        meta.streams.entry("Wav1".into()).or_default().kind = Some(StreamType::Electrical);
        meta.streams.entry("Wav1".into()).or_default().electrode_group = Some("A".into());
        meta.streams.entry("Temp".into()).or_default().electrode_group = Some("A".into());
        group_spec(&mut meta, "A").location = "M1".into();
        let card = stream_card(ws.session().unwrap(), &meta, "Wav1").unwrap();
        assert!(card.neural);
        assert_eq!((card.group.as_deref(), card.groups.clone(), card.shared_with.clone()), (Some("A"), vec!["A".to_string()], vec!["Temp".to_string()]));
        assert_eq!(merge(&["b".into()], ["a".into(), "b".into()]), vec!["b", "a"]);
        assert_eq!(group_digits(24414.0625), "24 414");
    }

    #[test]
    fn test_heading_and_table_data() {
        let mut ws = workspace(&scratch("contents-heading"));
        open(&mut ws, "session.fake");
        let s = ws.session().unwrap();
        let h = heading(s, &ws.meta, &Selection::Stream("Wav1".into())).unwrap();
        assert_eq!((h.kind, h.facts.as_str(), h.dest.as_str()), ("Stream", "2 ch · 1 000 Hz · 2.00 s · V", "TimeSeries"));
        let tick = Selection::Event(s.events[0].name.clone());
        let h = heading(s, &ws.meta, &tick).unwrap();
        assert_eq!(h.dest, "events table");
        let t = table_data(s, &tick).unwrap();
        assert_eq!(t.columns[..2], [("#".to_string(), true), ("Onset (s)".to_string(), true)]);
        assert_eq!(t.rows.len(), s.events[0].len());
        assert_eq!(t.rows[0][0], "1");
        assert!(table_data(s, &Selection::Stream("Wav1".into())).is_none(), "streams show the preview");

        let mut meta = ws.meta.clone();
        meta.set_included(ItemKind::Stream, "Wav1", false);
        assert_eq!(heading(s, &meta, &Selection::Stream("Wav1".into())).unwrap().dest, "left out");
    }
}
