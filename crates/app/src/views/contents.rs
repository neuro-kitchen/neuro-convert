//! ② Contents: three regions. Left, the tree of the recording (a panel that can be hidden);
//! middle, the data of the selected item (a stream's preview, the rows of events, tables,
//! snippets and electrode groups); right, the item's settings (a panel that can be hidden). The
//! middle's header says what is shown and where it goes in the NWB file; a hidden panel leaves
//! its toggle at the same edge of that header.

use gpui_kit::component::input::Input;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::table::{DataTable, TableState};
use gpui_kit::component::{h_resizable, resizable_panel};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::tree::tree;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window};
use nc_convert::core::{ItemKind, StreamType};

use super::preview::PreviewView;
use crate::settings::Panels;
use crate::viewmodels::contents::{ContentsVm, Selection, StreamCard};
use crate::widgets::{Edge, EmptyState, FormRow, IncludeToggle, IssueList, IssueRow, MenuSelect, Muted, PanelToggle, Section, PickInput, SidePanel, TextTable, HEADER_HEIGHT};

/// Width of a tree row's chevron and checkbox slots, and the gap after each.
const TREE_SLOT: f32 = 16.;
const TREE_GAP: f32 = 6.;

pub struct ContentsView {
    vm: Entity<ContentsVm>,
    preview: Entity<PreviewView>,
    /// The rows shown for the selected item (rebuilt when the selection changes).
    table: Option<(Selection, Entity<TableState<TextTable>>)>,
    /// The stream whose "Detect automatically" was just switched off: its type is asked for
    /// (highlighted dropdown) until one is picked.
    choosing_kind: Option<String>,
    _vm: Subscription,
}

impl ContentsView {
    pub fn new(vm: Entity<ContentsVm>, preview: Entity<PreviewView>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&vm, |_, _, cx| cx.notify());
        Self { vm, preview, table: None, choosing_kind: None, _vm: sub }
    }

    fn panels(&self, cx: &Context<Self>) -> Panels {
        self.vm.read(cx).store().read(cx).ws.settings.panels
    }

    fn set_panels(&self, cx: &mut gpui_kit::App, f: impl FnOnce(&mut Panels)) {
        let store = self.vm.read(cx).store().clone();
        store.update(cx, |s, cx| {
            let mut p = s.ws.settings.panels;
            f(&mut p);
            s.ws.set_panels(p);
            cx.notify();
        });
    }

    /// A toggle for the panel on `edge` (the same button in the panel's header and, when the
    /// panel is hidden, at that edge of the middle header).
    fn toggle(&self, edge: Edge, open: bool, cx: &mut Context<Self>) -> PanelToggle {
        let view = cx.entity();
        let (id, what) = match edge {
            Edge::Left => ("toggle-tree", "the recording's items"),
            Edge::Right => ("toggle-inspector", "the settings of the selected item"),
        };
        PanelToggle::new(id, edge, open, what, move |_, cx| {
            view.update(cx, |this, cx| {
                this.set_panels(cx, |p| match edge {
                    Edge::Left => p.tree = !p.tree,
                    Edge::Right => p.inspector = !p.inspector,
                });
                cx.notify();
            })
        })
    }

    /// The middle header: what is shown, its facts and where it goes; the toggles of hidden panels
    /// at its ends.
    fn heading(&self, panels: Panels, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let (muted, border) = (t.muted_foreground, t.border);
        let title = self.vm.read(cx).heading(cx).map(|h| {
            h_flex()
                .gap_2()
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .items_baseline()
                .child(div().flex_none().text_xs().text_color(muted).child(h.kind.to_uppercase()))
                .child(div().flex_none().text_sm().font_weight(FontWeight::SEMIBOLD).child(h.name))
                .child(div().min_w_0().truncate().text_xs().text_color(muted).child(h.facts))
                .child(div().flex_none().child(if h.dest == "left out" { Tag::secondary().xsmall().child(h.dest) } else { Tag::info().xsmall().child(h.dest) }))
        });
        h_flex()
            .id("contents-heading")
            .test_support()
            .flex_none()
            .h(px(HEADER_HEIGHT))
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(border)
            .when(!panels.tree, |this| this.child(self.toggle(Edge::Left, false, cx)))
            .children(title)
            .child(div().flex_1())
            .when(!panels.inspector, |this| this.child(self.toggle(Edge::Right, false, cx)))
            .into_any_element()
    }

    fn tree_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let vm = self.vm.clone();
        let rows = tree(&self.vm.read(cx).tree, move |ix, entry, selected, _window, cx| {
            let item = entry.item();
            let m = vm.read(cx);
            let toggle = m.inclusion(&item.id, cx).map(|(kind, names, state)| {
                let store = m.store().clone();
                IncludeToggle::new(format!("include-{}", item.id), state, move |checked, cx| ContentsVm::set_included(&store, kind, names.clone(), checked, cx))
            });
            let row = m.rows.get(item.id.as_ref()).cloned();
            let flagged = m.flagged.contains(item.id.as_ref());
            let tag = item.id.strip_prefix("s/").and_then(|n| m.stream_tag(n, cx));
            let chevron = item.is_folder().then(|| Icon::new(if entry.is_expanded() { IconName::ChevronDown } else { IconName::ChevronRight }).small());
            let t = cx.theme();
            let (muted, warning) = (t.muted_foreground, t.warning);
            // Fixed slots (empty when absent) keep chevrons, checkboxes and labels in columns;
            // a level is indented by one slot, so a child's checkbox sits under its parent's label
            let slot = || h_flex().flex_none().w(px(TREE_SLOT)).justify_center();
            let detail = row.as_ref().map(|r| {
                let full = r.detail.clone();
                div()
                    .id(SharedString::from(format!("detail-{}", item.id)))
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(muted)
                    .child(r.detail.clone())
                    .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
            });
            ListItem::new(ix).selected(selected).child(
                h_flex()
                    .w_full()
                    .gap(px(TREE_GAP))
                    .pl(px(entry.depth() as f32 * (TREE_SLOT + TREE_GAP)))
                    .child(slot().children(chevron))
                    .child(slot().children(toggle))
                    .child(div().flex_none().text_sm().when(item.is_folder(), |d| d.font_weight(FontWeight::SEMIBOLD)).child(row.as_ref().map_or_else(|| item.label.to_string(), |r| r.title.clone())))
                    .children(tag.map(|t| div().flex_none().child(if t == "ElectricalSeries" { Tag::info().xsmall().child(t) } else { Tag::secondary().xsmall().child(t) })))
                    .child(div().flex_1())
                    .when(flagged, |this| this.child(Icon::new(IconName::TriangleAlert).xsmall().text_color(warning)))
                    .children(detail),
            )
        });
        let view = cx.entity();
        SidePanel::new("tree-panel", "Recording", move |_, cx| view.update(cx, |this, cx| this.set_panels(cx, |p| p.tree = false)))
            .edge(Edge::Left, "toggle-tree", "the recording's items")
            .no_scroll()
            .child(div().flex_1().min_h_0().child(rows))
            .into_any_element()
    }

    fn issues(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let issues = self.vm.read(cx).issues(cx);
        (!issues.is_empty()).then(|| {
            let rows = issues.into_iter().map(|i| IssueRow { error: i.level == nc_convert::core::Level::Error, text: i.message.into(), target: None }).collect();
            IssueList::new("item-issues", rows).into_any_element()
        })
    }

    fn group_fields(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let m = self.vm.read(cx);
        let f = &m.fields;
        let mut out = Vec::new();
        if let Some(s) = &f.group_location {
            out.push(FormRow::new("Location", PickInput::replacing("group-location", s, m.location_suggestions(cx))).help("Brain area (or muscle) the electrodes are in").into_any_element());
        }
        if let Some(s) = &f.group_description {
            out.push(FormRow::new("Description", Input::new(s).small()).into_any_element());
        }
        if let Some(s) = &f.group_device {
            out.push(FormRow::new("Device", PickInput::replacing("group-device", s, m.device_suggestions(cx)).placeholder("First device of the session")).help("Array, probe or headstage").into_any_element());
        }
        out
    }

    /// The selected stream's settings (inspector): include, signal kind, electrodes, details.
    fn stream_settings(&mut self, card: StreamCard, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let vm = self.vm.clone();
        let included = self.vm.read(cx).store().read(cx).ws.included(ItemKind::Stream, &card.name);
        let include = {
            let (store, name) = (self.vm.read(cx).store().clone(), card.name.clone());
            Switch::new("stream-include").checked(included).label("Include in the NWB file").on_click(move |on, _, cx| ContentsVm::set_included(&store, ItemKind::Stream, vec![name.clone()], *on, cx))
        };
        // Signal kind: "Detect automatically" on → the detected type, greyed out; off → a
        // dropdown to choose it, highlighted until a type is picked
        const KINDS: [(&str, StreamType); 2] = [("ElectricalSeries", StreamType::Electrical), ("TimeSeries", StreamType::Timeseries)];
        let about = |k: StreamType| match k {
            StreamType::Electrical => "Voltage from electrodes: spikes, LFP, EEG, ECoG, EMG. Needs electrodes.",
            StreamType::Timeseries => "Any other signal: EMG envelope, temperature, stimulus monitor, sync, accelerometer.",
        };
        let choosing = card.kind.is_none() && self.choosing_kind.as_deref() == Some(card.name.as_str());
        let auto = card.kind.is_none() && !choosing;
        let label_of = |k: StreamType| KINDS.iter().find(|(_, x)| *x == k).map_or("", |(l, _)| l);
        let current = match card.kind {
            Some(k) => label_of(k).to_string(),
            None if choosing => "Choose a type…".to_string(),
            None => label_of(if card.neural { StreamType::Electrical } else { StreamType::Timeseries }).to_string(),
        };
        let selected = card.kind.and_then(|k| KINDS.iter().position(|(_, x)| *x == k));
        let view = cx.entity();
        let name = card.name.clone();
        let detect = Switch::new("kind-auto").checked(auto).label("Detect automatically").on_click(move |on, window, cx| {
            let (on, name) = (*on, name.clone());
            view.update(cx, |this, cx| {
                this.choosing_kind = (!on).then_some(name);
                if on {
                    this.vm.update(cx, |vm, cx| vm.set_kind(None, window, cx));
                }
                cx.notify();
            })
        });
        let view = cx.entity();
        let select = MenuSelect::new("kind-select", current, KINDS.iter().map(|(l, _)| SharedString::from(*l)).collect(), selected, move |i, window, cx| {
            let kind = KINDS[i].1;
            view.update(cx, |this, cx| {
                this.choosing_kind = None;
                this.vm.update(cx, |vm, cx| vm.set_kind(Some(kind), window, cx));
            })
        })
        .full_width()
        .disabled(auto);
        let primary = cx.theme().primary;
        // One line under the dropdown: what the shown type holds (and, when detected, why)
        let shown = card.kind.unwrap_or(if card.neural { StreamType::Electrical } else { StreamType::Timeseries });
        let line = match (auto, choosing) {
            (_, true) => None,
            (true, _) if card.from_recording.is_some() => Some(format!("{} The recording supplies electrodes.", about(shown))),
            (true, _) => Some(format!("{} The recording gives no electrodes.", about(shown))),
            (false, _) => Some(about(shown).to_string()),
        };
        let kinds = v_flex()
            .gap_2()
            .child(detect)
            .child(div().rounded_md().when(choosing, |d| d.border_2().border_color(primary)).child(select))
            .children(line.map(Muted::new))
            .when(card.looks_electrical && card.kind.is_none(), |c| c.child(Muted::new("Many channels at a high rate: this looks like electrode data. If it is, switch off detection and choose ElectricalSeries.")));

        let mut out = vec![include.into_any_element()];
        out.extend(self.issues(cx));
        out.push(Section::new("What is this signal?").child(kinds).into_any_element());

        if card.neural {
            let mut electrodes = Section::new("Electrodes");
            match &card.from_recording {
                Some(text) => electrodes = electrodes.child(Muted::new(format!("{text}, with positions and device."))),
                None => {
                    let mut options: Vec<SharedString> = card.groups.iter().map(|g| SharedString::from(g.clone())).collect();
                    options.push("New group…".into());
                    let selected = card.group.as_ref().and_then(|g| card.groups.iter().position(|x| x == g));
                    let groups = card.groups.clone();
                    let vm = vm.clone();
                    let select = MenuSelect::new("group-select", card.group.clone().unwrap_or_else(|| "Choose a group…".into()), options, selected, move |i, window, cx| {
                        vm.update(cx, |vm, cx| match groups.get(i) {
                            Some(g) => vm.set_group(g.clone(), window, cx),
                            None => vm.new_group(window, cx),
                        })
                    });
                    electrodes = electrodes.child(
                        FormRow::new("Electrode group", select)
                            .required(true)
                            .help("Every channel of this stream becomes one electrode of the group"),
                    );
                }
            }
            if card.group.is_some() {
                if !card.shared_with.is_empty() {
                    electrodes = electrodes.child(Muted::new(format!("This group is also used by {}; changes apply to all.", card.shared_with.join(", "))));
                }
                electrodes = electrodes.children(self.group_fields(cx));
            }
            out.push(electrodes.into_any_element());
        }

        let m = self.vm.read(cx);
        let f = &m.fields;
        let mut details = Section::new("Details");
        if let Some(s) = &f.name {
            details = details.child(FormRow::new("Name in the NWB file", Input::new(s).small()).help(format!("Empty: {}", card.name)));
        }
        if let Some(s) = &f.unit {
            details = details.child(FormRow::new("Unit", PickInput::replacing("stream-unit", s, m.unit_suggestions())).help("Physical unit after scaling (electrical series are stored in volts)"));
        }
        if let Some(s) = &f.conversion {
            let mut help = "Multiplies stored values to get the unit. Empty: values are already in the unit".to_string();
            if let Some(note) = &card.scale_note {
                help = format!("The recording does not give it ({note}). {help}");
            }
            let errors = m.errors.iter().map(|e| (true, e.clone())).collect();
            details = details.child(FormRow::new("Scale factor", Input::new(s).small()).help(help).issues(errors));
        }
        out.push(details.into_any_element());
        out
    }

    /// Settings of an event series, table or snippet store (inspector).
    fn item_settings(&self, title: String, what: &str, kind: ItemKind, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let issues = self.issues(cx);
        let m = self.vm.read(cx);
        let included = m.store().read(cx).ws.included(kind, &title);
        let (store, name) = (m.store().clone(), title.clone());
        let include = Switch::new("item-include").checked(included).label("Include in the NWB file").on_click(move |on, _, cx| ContentsVm::set_included(&store, kind, vec![name.clone()], *on, cx));
        let f = &m.fields;
        let mut out = vec![include.into_any_element(), Muted::new(what.to_string()).into_any_element()];
        out.extend(issues);
        if let Some(s) = &f.name {
            out.push(FormRow::new("Name in the NWB file", Input::new(s).small()).help(format!("Empty: {title}")).into_any_element());
        }
        if let Some(s) = &f.conversion {
            out.push(FormRow::new("Scale factor to volts", Input::new(s).small()).into_any_element());
        }
        out
    }

    /// The rows of the selected item, in a table kept while the selection stays.
    fn data_table(&mut self, selected: &Selection, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.table.as_ref().is_none_or(|(s, _)| s != selected) {
            self.table = self.vm.read(cx).table_data(cx).map(|data| (selected.clone(), cx.new(|cx| TableState::new(TextTable::new(data), window, cx))));
        }
        match &self.table {
            Some((_, state)) if state.read(cx).delegate().data().rows.is_empty() => Muted::new("No rows.").into_any_element(),
            Some((_, state)) => div().id("item-table").test_support().size_full().child(DataTable::new(state).stripe(true).bordered(true).scrollbar_visible(true, true)).into_any_element(),
            None => Muted::new("Nothing to show.").into_any_element(),
        }
    }
}

impl Render for ContentsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.vm.read(cx).selected.clone();
        let panels = self.panels(cx);

        let main: AnyElement = match &selected {
            Selection::Stream(_) => div().size_full().p_2().child(self.preview.clone()).into_any_element(),
            Selection::None => EmptyState::new(IconName::Inbox, "Nothing selected", "Pick a stream or an event series in the recording to preview it.").into_any_element(),
            other => div().size_full().p_2().child(self.data_table(other, window, cx)).into_any_element(),
        };

        let settings: Vec<AnyElement> = match selected.clone() {
            Selection::Stream(_) => match self.vm.read(cx).card(cx) {
                Some(card) => self.stream_settings(card, cx),
                None => Vec::new(),
            },
            Selection::Event(n) => self.item_settings(n, "Event times (and values), written as an events table; a TimeSeries when an event has several values", ItemKind::Event, cx),
            Selection::Table(n) => self.item_settings(n, "A table of the recording, written as an analysis table", ItemKind::Table, cx),
            Selection::Snippet(n) => self.item_settings(n, "Spike waveforms, written as a SpikeEventSeries (needs an electrode group)", ItemKind::Snippet, cx),
            Selection::Group(_) => {
                let mut v: Vec<AnyElement> = self.issues(cx).into_iter().collect();
                v.push(Section::new("Where these electrodes are").children(self.group_fields(cx)).into_any_element());
                v
            }
            Selection::None => vec![EmptyState::new(IconName::Settings2, "No settings yet", "The settings of the selected item appear here.").into_any_element()],
        };
        let view = cx.entity();
        let inspector = SidePanel::new("inspector-panel", "Settings", move |_, cx| view.update(cx, |this, cx| this.set_panels(cx, |p| p.inspector = false)))
            .edge(Edge::Right, "toggle-inspector", "the settings of the selected item")
            .children(settings);
        let middle = v_flex().id("contents-card").size_full().min_w_0().child(self.heading(panels, cx)).child(div().flex_1().min_h_0().child(main));

        let regions = h_resizable("contents-regions")
            .child(resizable_panel().visible(panels.tree).size(px(320.)).size_range(px(220.)..px(560.)).child(self.tree_panel(cx)))
            .child(resizable_panel().child(middle))
            .child(resizable_panel().visible(panels.inspector).size(px(380.)).size_range(px(280.)..px(620.)).child(inspector));

        div().id("contents-step").test_support().size_full().child(regions)
    }
}
