//! ② Contents: the tree on the left; on the right a card for the selected item (for a stream:
//! what kind of signal, its electrodes, details under "More") and the stream's preview.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tree::tree;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window};
use nc_convert::core::{ItemKind, StreamType};

use super::preview::PreviewView;
use crate::viewmodels::contents::{ContentsVm, Selection, StreamCard};
use crate::widgets::{Card, Choice, FormRow, IncludeToggle, IssueList, IssueRow, MenuSelect, Muted, SuggestInput};

pub struct ContentsView {
    vm: Entity<ContentsVm>,
    preview: Entity<PreviewView>,
    /// "More" of the stream card is open.
    more: bool,
    _vm: Subscription,
}

impl ContentsView {
    pub fn new(vm: Entity<ContentsVm>, preview: Entity<PreviewView>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&vm, |_, _, cx| cx.notify());
        Self { vm, preview, more: false, _vm: sub }
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
        v_flex()
            .w(px(340.))
            .flex_none()
            .h_full()
            .gap_2()
            .p_3()
            .border_r_1()
            .border_color(cx.theme().border)
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

    fn stream_card(&mut self, card: StreamCard, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let _ = window;
        let vm = self.vm.clone();
        let included = self.vm.read(cx).store().read(cx).ws.included(ItemKind::Stream, &card.name);
        let include = {
            let (store, name) = (self.vm.read(cx).store().clone(), card.name.clone());
            Switch::new("stream-include").checked(included).label("Include").on_click(move |on, _, cx| ContentsVm::set_included(&store, ItemKind::Stream, vec![name.clone()], *on, cx))
        };
        let kind = |id: &'static str, title: &'static str, description: String, value: Option<StreamType>| {
            let vm = vm.clone();
            Choice::new(id, title, description, card.kind == value, move |window, cx| vm.update(cx, |vm, cx| vm.set_kind(value, window, cx)))
        };
        let automatic = format!("Neural when the recording supplies electrodes{}", if card.from_recording.is_some() { " — it does: neural" } else { " — it does not: other signal" });
        let kinds = v_flex()
            .gap_1p5()
            .child(kind("kind-auto", "Automatic", automatic, None))
            .child(kind("kind-neural", "Neural recording", "Voltage from electrodes in tissue → stored as ElectricalSeries; needs electrodes".into(), Some(StreamType::Electrical)))
            .child(kind("kind-other", "Other signal", "EMG, temperature, stimulus monitor, sync… → stored as TimeSeries".into(), Some(StreamType::Timeseries)));

        let mut card_el = Card::new()
            .title(card.name.clone())
            .aside(h_flex().gap_2().child(include))
            .child(Muted::new(card.summary.clone()))
            .children(self.issues(cx))
            .child(FormRow::new("What is this signal?", kinds));

        if card.neural {
            let mut electrodes = v_flex().gap_3();
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
            card_el = card_el.child(Card::new().title("Electrodes").child(electrodes));
        }

        // Details, closed by default
        let more = self.more;
        let toggle = Button::new("stream-more").ghost().small().icon(if more { IconName::ChevronDown } else { IconName::ChevronRight }).label("More: name, unit, scale").on_click(cx.listener(|this, _, _, cx| {
            this.more = !this.more;
            cx.notify();
        }));
        card_el = card_el.child(toggle);
        if more {
            let m = self.vm.read(cx);
            let f = &m.fields;
            let mut details = v_flex().gap_3().pl_4();
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
            card_el = card_el.child(details);
        }
        card_el.into_any_element()
    }

    fn item_card(&self, title: String, what: &str, kind: ItemKind, cx: &mut Context<Self>) -> AnyElement {
        let issues = self.issues(cx);
        let m = self.vm.read(cx);
        let included = m.store().read(cx).ws.included(kind, &title);
        let (store, name) = (m.store().clone(), title.clone());
        let include = Switch::new("item-include").checked(included).label("Include").on_click(move |on, _, cx| ContentsVm::set_included(&store, kind, vec![name.clone()], *on, cx));
        let f = &m.fields;
        let mut card = Card::new().title(title.clone()).aside(include).child(Muted::new(what.to_string())).children(issues);
        if let Some(s) = &f.name {
            card = card.child(FormRow::new("Name in the NWB file", Input::new(s).small()).help(format!("Empty: {title}")));
        }
        if let Some(s) = &f.conversion {
            card = card.child(FormRow::new("Scale factor to volts", Input::new(s).small()));
        }
        card.into_any_element()
    }
}

impl Render for ContentsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.vm.read(cx).selected.clone();
        let body: Vec<AnyElement> = match selected {
            Selection::Stream(_) => {
                let card = self.vm.read(cx).card(cx);
                let mut v = Vec::new();
                if let Some(card) = card {
                    v.push(self.stream_card(card, window, cx));
                }
                v.push(Card::new().title("Preview").child(div().h(px(460.)).child(self.preview.clone())).into_any_element());
                v
            }
            Selection::Event(n) => vec![self.item_card(n, "Event times (and values) → NWB events table or scalar series", ItemKind::Event, cx)],
            Selection::Table(n) => vec![self.item_card(n, "A table of the recording → NWB analysis table", ItemKind::Table, cx)],
            Selection::Snippet(n) => vec![self.item_card(n, "Spike waveforms → SpikeEventSeries (needs an electrode group)", ItemKind::Snippet, cx)],
            Selection::Group(g) => {
                let fields = self.group_fields(cx);
                vec![Card::new().title(format!("Electrode group {g}")).child(Muted::new("Where these electrodes are and what recorded them.")).children(self.issues(cx)).children(fields).into_any_element()]
            }
            Selection::None => vec![Muted::new("Select an item in the tree.").into_any_element()],
        };
        h_flex()
            .id("contents-step")
            .test_support()
            .size_full()
            .items_start()
            .child(self.tree_panel(cx))
            .child(v_flex().id("contents-card").flex_1().min_w_0().h_full().p_4().gap_4().children(body).overflow_y_scrollbar())
    }
}
