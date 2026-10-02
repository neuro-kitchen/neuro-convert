//! Form pieces: a card, a labelled row that shows its issues (red outline + message), a dropdown
//! of fixed options, a dropdown of common values with "Other…" revealing a text box, and a text
//! input with suggestions (lists, where typing adds to the value).

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, px, AnyElement, App, ElementId, Entity, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled as _, Window,
};

/// A bordered panel with a title.
#[derive(IntoElement)]
pub struct Card {
    title: Option<SharedString>,
    aside: Option<AnyElement>,
    children: Vec<AnyElement>,
}

impl Card {
    pub fn new() -> Self {
        Self { title: None, aside: None, children: Vec::new() }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Shown at the right of the title.
    pub fn aside(mut self, e: impl IntoElement) -> Self {
        self.aside = Some(e.into_any_element());
        self
    }
}

impl ParentElement for Card {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Card {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        v_flex()
            .w_full()
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(t.border)
            .bg(t.background)
            .when(self.title.is_some() || self.aside.is_some(), |this| {
                this.child(
                    h_flex()
                        .justify_between()
                        .gap_2()
                        .children(self.title.map(|title| div().text_base().font_weight(FontWeight::SEMIBOLD).child(title)))
                        .children(self.aside),
                )
            })
            .children(self.children)
    }
}

/// `label` (required mark, help) above a control; issues outline the control in red (or
/// amber for warnings) and are listed under it.
#[derive(IntoElement)]
pub struct FormRow {
    label: SharedString,
    required: bool,
    help: Option<SharedString>,
    control: AnyElement,
    issues: Vec<(bool, String)>,
    /// The outline hugs a fixed-width control instead of spanning the row.
    compact: bool,
}

impl FormRow {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self { label: label.into(), required: false, help: None, control: control.into_any_element(), issues: Vec::new(), compact: false }
    }

    /// For controls narrower than the row (date picker, zone list, age).
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub fn help(mut self, help: impl Into<SharedString>) -> Self {
        let h = help.into();
        self.help = (!h.is_empty()).then_some(h);
        self
    }

    /// (is error, message) of the issues about this field.
    pub fn issues(mut self, issues: Vec<(bool, String)>) -> Self {
        self.issues = issues;
        self
    }
}

impl RenderOnce for FormRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        let (danger, warning, muted) = (t.danger, t.warning, t.muted_foreground);
        let ring = if self.issues.iter().any(|(e, _)| *e) { Some(danger) } else if self.issues.is_empty() { None } else { Some(warning) };
        v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.label)
                    .when(self.required, |this| this.child(div().text_color(danger).child("*"))),
            )
            .map(|this| {
                let outlined = div().rounded_md().when_some(ring, |this, c| this.border_2().border_color(c)).child(self.control);
                if self.compact { this.child(h_flex().child(outlined)) } else { this.child(outlined.w_full()) }
            })
            .children(self.issues.into_iter().map(move |(error, m)| div().text_xs().text_color(if error { danger } else { warning }).child(m)))
            .children(self.help.map(|h| div().text_xs().text_color(muted).child(h)))
    }
}

type OnPick = Rc<dyn Fn(usize, &mut Window, &mut App)>;
type OnPickValue = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// A button showing the current option; clicking opens the list of options.
#[derive(IntoElement)]
pub struct MenuSelect {
    id: ElementId,
    current: SharedString,
    options: Vec<SharedString>,
    selected: Option<usize>,
    on_pick: OnPick,
    /// Spans its row (form fields) instead of hugging its label (toolbars).
    full: bool,
    disabled: bool,
}

impl MenuSelect {
    pub fn new(id: impl Into<ElementId>, current: impl Into<SharedString>, options: Vec<SharedString>, selected: Option<usize>, on_pick: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), current: current.into(), options, selected, on_pick: Rc::new(on_pick), full: false, disabled: false }
    }

    pub fn full_width(mut self) -> Self {
        self.full = true;
        self
    }

    /// Shown greyed out with its value; does not open.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for MenuSelect {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self { id, current, options, selected, on_pick, full, disabled } = self;
        // A bordered bar with a caret and a hand cursor: it reads as something to press
        let button = Button::new(id).outline().small().label(current).dropdown_caret(true).disabled(disabled).when(!disabled, |b| b.cursor_pointer()).when(full, |b| b.w_full());
        button.dropdown_menu(move |menu, _, _| {
            let mut menu = menu.scrollable(true).max_h(px(360.));
            for (i, label) in options.iter().enumerate() {
                let on_pick = on_pick.clone();
                menu = menu.item(PopupMenuItem::new(label.clone()).checked(selected == Some(i)).on_click(move |_, window, cx| on_pick(i, window, cx)));
            }
            menu
        })
    }
}

/// One value from a list of common ones: a dropdown bar; its last entry, "Other…", shows a text
/// box under it for anything else (also shown while the value is not in the list).
#[derive(IntoElement)]
pub struct PickInput {
    id: SharedString,
    state: Entity<InputState>,
    /// (label, value)
    options: Vec<(String, String)>,
    placeholder: SharedString,
    on_pick: OnPickValue,
}

impl PickInput {
    pub fn new(id: impl Into<SharedString>, state: &Entity<InputState>, options: Vec<(String, String)>, on_pick: impl Fn(String, &mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), state: state.clone(), options, placeholder: "Choose…".into(), on_pick: Rc::new(on_pick) }
    }

    /// Picking replaces the input's text.
    pub fn replacing(id: impl Into<SharedString>, state: &Entity<InputState>, options: Vec<String>) -> Self {
        let target = state.clone();
        Self::new(id, state, options.into_iter().map(|v| (v.clone(), v)).collect(), move |v, window, cx| target.update(cx, |s, cx| s.replace_all(v, window, cx)))
    }

    /// Shown on the bar while there is no value.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }
}

impl RenderOnce for PickInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, state, options, placeholder, on_pick } = self;
        // "Other…" was chosen (kept while this field is on screen)
        let other = window.use_keyed_state(ElementId::Name(format!("{id}-other").into()), cx, |_, _| false);
        let value = state.read(cx).value().to_string();
        let known = options.iter().find(|(_, v)| *v == value).map(|(l, _)| l.clone());
        let typing = *other.read(cx) || (!value.trim().is_empty() && known.is_none());
        let label: SharedString = match (&known, typing) {
            (Some(l), false) => l.clone().into(),
            (_, true) => "Other…".into(),
            (None, false) => placeholder,
        };
        let picked = options.iter().position(|(_, v)| *v == value).filter(|_| !typing);
        let mut labels: Vec<SharedString> = options.iter().map(|(l, _)| SharedString::from(l.clone())).collect();
        labels.push("Other…".into());
        let selected = if typing { Some(options.len()) } else { picked };
        let (input_state, other_flag, clear) = (state.clone(), other.clone(), known.is_some());
        let select = MenuSelect::new(ElementId::Name(id.clone()), label, labels, selected, move |i, window, cx| match options.get(i) {
            Some((_, v)) => {
                other_flag.update(cx, |o, _| *o = false);
                on_pick(v.clone(), window, cx);
            }
            None => {
                other_flag.update(cx, |o, _| *o = true);
                // Start from an empty box rather than the listed value it replaces
                input_state.update(cx, |s, cx| {
                    if clear {
                        s.replace_all("", window, cx);
                    }
                    s.focus(window, cx);
                });
            }
        })
        .full_width();
        v_flex()
            .w_full()
            .gap_1()
            .child(select)
            .when(typing, |this| this.child(Input::new(&state).id(ElementId::Name(format!("{id}-text").into())).small()))
    }
}

/// A text input with a ▾ button listing suggestions; picking one calls `on_pick` with its value.
#[derive(IntoElement)]
pub struct SuggestInput {
    id: SharedString,
    state: Entity<InputState>,
    /// (label, value)
    suggestions: Vec<(String, String)>,
    on_pick: OnPickValue,
}

impl SuggestInput {
    pub fn new(id: impl Into<SharedString>, state: &Entity<InputState>, suggestions: Vec<(String, String)>, on_pick: impl Fn(String, &mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), state: state.clone(), suggestions, on_pick: Rc::new(on_pick) }
    }
}

impl RenderOnce for SuggestInput {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self { id, state, suggestions, on_pick } = self;
        let input = Input::new(&state).id(ElementId::Name(id.clone())).small();
        if suggestions.is_empty() {
            return input.into_any_element();
        }
        let menu = Button::new(ElementId::Name(format!("{id}-suggest").into())).ghost().xsmall().icon(IconName::ChevronDown).tooltip("Suggestions").dropdown_menu(move |menu, _, _| {
            let mut menu = menu.scrollable(true).max_h(px(320.));
            for (label, value) in &suggestions {
                let (on_pick, value) = (on_pick.clone(), value.clone());
                menu = menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, window, cx| on_pick(value.clone(), window, cx)));
            }
            menu
        });
        input.suffix(menu).into_any_element()
    }
}
