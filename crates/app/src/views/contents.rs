//! ② Contents: three regions. Left, the tree of the recording (a panel that can be hidden);
//! middle, the data of the selected item (a stream's preview, the rows of events, tables,
//! snippets and electrode groups); right, the item's settings (a panel that can be hidden). A
//! heading above says what is shown and where it goes in the NWB file.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::table::{DataTable, TableState};
use gpui_kit::component::{h_resizable, resizable_panel};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tree::tree;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window};
use nc_convert::core::{ItemKind, StreamType};

use super::preview::PreviewView;
use crate::settings::Panels;
use crate::viewmodels::contents::{ContentsVm, Selection, StreamCard};
use crate::widgets::{FormRow, IncludeToggle, IssueList, IssueRow, MenuSelect, Muted, Section, SidePanel, SuggestInput, TextTable};

pub struct ContentsView {
    vm: Entity<ContentsVm>,
    preview: Entity<PreviewView>,
    /// The rows shown for the selected item (rebuilt when the selection changes).
    table: Option<(Selection, Entity<TableState<TextTable>>)>,
    _vm: Subscription,
}

impl ContentsView {
    pub fn new(vm: Entity<ContentsVm>, preview: Entity<PreviewView>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&vm, |_, _, cx| cx.notify());
        Self { vm, preview, table: None, _vm: sub }
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

    /// What is shown, its facts and where it goes; with the panel toggles at both ends.
    fn heading(&self, panels: Panels, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let (muted, border) = (t.muted_foreground, t.border);
        let toggle = |id: &'static str, icon: IconName, open: bool, tip: &'static str, cx: &mut Context<Self>, f: fn(&mut Panels)| {
            let b = Button::new(id).ghost().small().icon(icon).tooltip(tip);
            let b = if open { b.selected(true) } else { b };
            b.on_click(cx.listener(move |this, _, _, cx| {
                this.set_panels(cx, f);
                cx.notify();
            }))
        };
        let title = self.vm.read(cx).heading(cx).map(|h| {
            h_flex()
                .gap_2()
                .min_w_0()
                .flex_1()
                .items_baseline()
                .child(div().text_xs().text_color(muted).child(h.kind.to_uppercase()))
                .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(h.name))
                .child(div().text_sm().text_color(muted).child(h.facts))
                .child(if h.dest == "left out" { Tag::secondary().small().child(h.dest) } else { Tag::info().small().child(h.dest) })
        });
        h_flex()
            .id("contents-heading")
            .test_support()
            .flex_none()
            .gap_2()
            .px_2()
            .py_1p5()
            .border_b_1()
            .border_color(border)
            .child(toggle("toggle-tree", IconName::PanelLeft, panels.tree, "Show or hide the recording's items", cx, |p| p.tree = !p.tree))
            .children(title)
            .child(div().flex_1())
            .child(toggle("toggle-inspector", IconName::PanelRight, panels.inspector, "Show or hide the settings of the selected item", cx, |p| p.inspector = !p.inspector))
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
            ListItem::new(ix).selected(selected).child(
                h_flex()
                    .w_full()
                    .gap_1p5()
                    .pl(px(entry.depth() as f32 * 14.))
                    .children(chevron)
                    .when_some(toggle, |this, t| this.child(t))
                    .child(div().text_sm().when(item.is_folder(), |d| d.font_weight(FontWeight::SEMIBOLD)).child(row.as_ref().map_or_else(|| item.label.to_string(), |r| r.title.clone())))
                    .children(tag.map(|t| if t == "neural" { Tag::info().xsmall().child(t) } else { Tag::secondary().xsmall().child(t) }))
                    .child(div().flex_1())
                    .when(flagged, |this| this.child(Icon::new(IconName::TriangleAlert).xsmall().text_color(warning)))
                    .children(row.map(|r| div().text_xs().text_color(muted).child(r.detail))),
            )
        });
        let view = cx.entity();
        SidePanel::new("tree-panel", "Recording", move |_, cx| view.update(cx, |this, cx| this.set_panels(cx, |p| p.tree = false)))
            .no_scroll()
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(self.vm.read(cx).source_line(cx)))
            .child(Muted::new("Tick what goes into the NWB file; select an item to set it up."))
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
            out.push(FormRow::new("Location", SuggestInput::replacing("group-location", s, m.location_suggestions(cx))).help("Brain area (or muscle) the electrodes are in").into_any_element());
        }
        if let Some(s) = &f.group_description {
            out.push(FormRow::new("Description", Input::new(s).small()).into_any_element());
        }
        if let Some(s) = &f.group_device {
            out.push(FormRow::new("Device", SuggestInput::replacing("group-device", s, m.device_suggestions(cx))).help("Array, probe or headstage; empty: the first device of the session").into_any_element());
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
        // Signal kind: a segmented choice and one line saying what the chosen one means
        let kind = |id: &'static str, label: &'static str, value: Option<StreamType>| {
            let vm = vm.clone();
            let b = Button::new(id).small().label(label);
            let b = if card.kind == value { b.primary() } else { b.outline() };
            b.on_click(move |_, window, cx| vm.update(cx, |vm, cx| vm.set_kind(value, window, cx)))
        };
        let meaning = match card.kind {
            None if card.from_recording.is_some() => "Automatic: the recording supplies electrodes, so it is stored as a neural recording (ElectricalSeries).",
            None => "Automatic: the recording supplies no electrodes, so it is stored as an other signal (TimeSeries).",
            Some(StreamType::Electrical) => "Voltage from electrodes (neural, EMG) → ElectricalSeries; needs an electrode group.",
            Some(StreamType::Timeseries) => "EMG envelope, temperature, stimulus monitor, sync… → TimeSeries.",
        };
        let kinds = v_flex()
            .gap_1p5()
            .child(h_flex().gap_1().child(kind("kind-auto", "Automatic", None)).child(kind("kind-neural", "Neural", Some(StreamType::Electrical))).child(kind("kind-other", "Other", Some(StreamType::Timeseries))))
            .child(Muted::new(meaning))
            .when(card.looks_electrical, |c| c.child(Muted::new("Many channels at a high rate: this looks like electrode data. If it is, choose Neural and pick an electrode group.")));

        let mut out = vec![include.into_any_element()];
        out.extend(self.issues(cx));
        out.push(FormRow::new("What is this signal?", kinds).into_any_element());

        if card.neural {
            let mut electrodes = Section::new("Electrodes");
            match &card.from_recording {
                Some(text) => electrodes = electrodes.child(Muted::new(format!("{text}. Positions and device come from the recording."))),
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
                            .help("Every channel of this stream becomes one electrode of the group (positions and per-channel mapping come with probe definitions)"),
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
            details = details.child(FormRow::new("Unit", SuggestInput::replacing("stream-unit", s, m.unit_suggestions())).help("Physical unit after scaling (electrical series are stored in volts)"));
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
            Selection::None => div().p_4().child(Muted::new("Select an item in the tree.")).into_any_element(),
            other => div().size_full().p_2().child(self.data_table(other, window, cx)).into_any_element(),
        };

        let settings: Vec<AnyElement> = match selected.clone() {
            Selection::Stream(_) => match self.vm.read(cx).card(cx) {
                Some(card) => self.stream_settings(card, cx),
                None => Vec::new(),
            },
            Selection::Event(n) => self.item_settings(n, "Event times (and values) → NWB events table, or a TimeSeries for several values per event", ItemKind::Event, cx),
            Selection::Table(n) => self.item_settings(n, "A table of the recording → NWB analysis table", ItemKind::Table, cx),
            Selection::Snippet(n) => self.item_settings(n, "Spike waveforms → SpikeEventSeries (needs an electrode group)", ItemKind::Snippet, cx),
            Selection::Group(_) => {
                let mut v = vec![Muted::new("Where these electrodes are and what recorded them.").into_any_element()];
                v.extend(self.issues(cx));
                v.extend(self.group_fields(cx));
                v
            }
            Selection::None => vec![Muted::new("Select an item in the tree.").into_any_element()],
        };
        let view = cx.entity();
        let inspector = SidePanel::new("inspector-panel", "Settings", move |_, cx| view.update(cx, |this, cx| this.set_panels(cx, |p| p.inspector = false))).children(settings);

        let regions = h_resizable("contents-regions")
            .child(resizable_panel().visible(panels.tree).size(px(320.)).size_range(px(220.)..px(560.)).child(self.tree_panel(cx)))
            .child(resizable_panel().child(v_flex().id("contents-card").size_full().min_w_0().child(main)))
            .child(resizable_panel().visible(panels.inspector).size(px(380.)).size_range(px(280.)..px(620.)).child(inspector));

        v_flex()
            .id("contents-step")
            .test_support()
            .size_full()
            .child(self.heading(panels, cx))
            .child(div().flex_1().min_h_0().child(regions))
    }
}
