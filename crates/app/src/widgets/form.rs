//! Form pieces: a card, a labelled row that shows its issues (red outline + message), a choice
//! card (one option with what it means), a dropdown of fixed options, and a text input with
//! suggestions (type anything or pick).

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    div, px, AnyElement, App, ElementId, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
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
}

impl FormRow {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self { label: label.into(), required: false, help: None, control: control.into_any_element(), issues: Vec::new() }
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
            .child(div().w_full().rounded_md().when_some(ring, |this, c| this.border_2().border_color(c)).child(self.control))
            .children(self.issues.into_iter().map(move |(error, m)| div().text_xs().text_color(if error { danger } else { warning }).child(m)))
            .children(self.help.map(|h| div().text_xs().text_color(muted).child(h)))
    }
}

type OnClick = Rc<dyn Fn(&mut Window, &mut App)>;

/// One option of a choice: a title, what choosing it means, selected or not.
#[derive(IntoElement)]
pub struct Choice {
    id: ElementId,
    title: SharedString,
    description: SharedString,
    selected: bool,
    on_click: OnClick,
}

impl Choice {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>, description: impl Into<SharedString>, selected: bool, on_click: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), title: title.into(), description: description.into(), selected, on_click: Rc::new(on_click) }
    }
}

impl RenderOnce for Choice {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        let on_click = self.on_click;
        h_flex()
            .id(self.id)
            .test_support()
            .items_start()
            .gap_2()
            .p_2()
            .rounded_md()
            .border_1()
            .cursor_pointer()
            .border_color(if self.selected { t.primary } else { t.border })
            .when(self.selected, |this| this.bg(t.accent))
            .hover(|s| s.bg(t.accent))
            .on_click(move |_, window, cx| on_click(window, cx))
            .child(
                div()
                    .mt(px(3.))
                    .size_3()
                    .flex_none()
                    .rounded_full()
                    .border_2()
                    .border_color(if self.selected { t.primary } else { t.muted_foreground })
                    .when(self.selected, |this| this.bg(t.primary)),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(self.title))
                    .child(div().text_xs().text_color(t.muted_foreground).child(self.description)),
            )
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
}

impl MenuSelect {
    pub fn new(id: impl Into<ElementId>, current: impl Into<SharedString>, options: Vec<SharedString>, selected: Option<usize>, on_pick: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), current: current.into(), options, selected, on_pick: Rc::new(on_pick) }
    }
}

impl RenderOnce for MenuSelect {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self { id, current, options, selected, on_pick } = self;
        Button::new(id).outline().small().label(current).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
            let mut menu = menu.scrollable(true).max_h(px(360.));
            for (i, label) in options.iter().enumerate() {
                let on_pick = on_pick.clone();
                menu = menu.item(PopupMenuItem::new(label.clone()).checked(selected == Some(i)).on_click(move |_, window, cx| on_pick(i, window, cx)));
            }
            menu
        })
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

    /// Picking replaces the input's text.
    pub fn replacing(id: impl Into<SharedString>, state: &Entity<InputState>, suggestions: Vec<String>) -> Self {
        let target = state.clone();
        Self::new(id, state, suggestions.into_iter().map(|v| (v.clone(), v)).collect(), move |v, window, cx| target.update(cx, |s, cx| s.replace_all(v, window, cx)))
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
