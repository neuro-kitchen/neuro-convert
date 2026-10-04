//! A side panel (tree, inspector, NWB structure): a header with its title and a close button,
//! and a scrolling body, on the theme's sidebar tint (the data in the middle stays on the plain
//! background). Panels are shown and hidden by their view; widths come from the
//! resizable group around them.
//!
//! A panel on an edge of a region ([`SidePanel::edge`]) has a [`PanelToggle`] beside its title
//! instead of the close button. Closing it hides the title, not the button: the region's middle
//! header shows the same toggle at the same edge, so it stays where it was.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, App, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement, RenderOnce, SharedString, Styled as _, Window};

type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// Height of a panel header (the middle header of a region uses it too, so all line up).
pub const HEADER_HEIGHT: f32 = 34.;

/// Which edge of its region a panel sits on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
}

/// The button that shows or hides the panel on `edge`; it looks the same in the panel's header
/// (open) and in the middle header (closed).
#[derive(IntoElement)]
pub struct PanelToggle {
    id: SharedString,
    edge: Edge,
    open: bool,
    /// What the panel holds, for the tooltip ("the recording's items").
    what: SharedString,
    on_click: OnClose,
}

impl PanelToggle {
    pub fn new(id: impl Into<SharedString>, edge: Edge, open: bool, what: impl Into<SharedString>, on_click: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), edge, open, what: what.into(), on_click: Rc::new(on_click) }
    }
}

impl RenderOnce for PanelToggle {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let icon = match (self.edge, self.open) {
            (Edge::Left, true) => IconName::PanelLeftClose,
            (Edge::Left, false) => IconName::PanelLeftOpen,
            (Edge::Right, true) => IconName::PanelRightClose,
            (Edge::Right, false) => IconName::PanelRightOpen,
        };
        let tip = format!("{} {}", if self.open { "Hide" } else { "Show" }, self.what);
        let on_click = self.on_click;
        Button::new(ElementId::Name(self.id)).ghost().xsmall().icon(icon).tooltip(tip).on_click(move |_, window, cx| on_click(window, cx))
    }
}

#[derive(IntoElement)]
pub struct SidePanel {
    id: SharedString,
    title: SharedString,
    on_close: OnClose,
    /// Scroll the body (off for bodies that scroll themselves, like the tree).
    scroll: bool,
    /// On an edge: a toggle beside the title (with this id and tooltip) instead of a close button.
    edge: Option<(Edge, SharedString, SharedString)>,
    children: Vec<AnyElement>,
}

impl SidePanel {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>, on_close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), title: title.into(), on_close: Rc::new(on_close), scroll: true, edge: None, children: Vec::new() }
    }

    pub fn no_scroll(mut self) -> Self {
        self.scroll = false;
        self
    }

    /// Sits on `edge` of its region: the toggle `id` (tooltip "Hide `what`") hides it.
    pub fn edge(mut self, edge: Edge, id: impl Into<SharedString>, what: impl Into<SharedString>) -> Self {
        self.edge = Some((edge, id.into(), what.into()));
        self
    }
}

impl ParentElement for SidePanel {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for SidePanel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        let on_close = self.on_close;
        let title = div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(t.muted_foreground).child(self.title.to_uppercase());
        let header = h_flex().flex_none().h(px(HEADER_HEIGHT)).gap_1().px_2().border_b_1().border_color(t.border);
        let header = match self.edge {
            Some((edge, id, what)) => {
                let toggle = PanelToggle::new(id, edge, true, what, move |window, cx| on_close(window, cx));
                match edge {
                    Edge::Left => header.child(toggle).child(title),
                    Edge::Right => header.justify_between().child(title).child(toggle),
                }
            }
            None => header.justify_between().pl_3().child(title).child(
                Button::new(ElementId::Name(format!("{}-close", self.id).into()))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip("Hide this panel")
                    .on_click(move |_, window, cx| on_close(window, cx)),
            ),
        };
        let body = v_flex().id(ElementId::Name(format!("{}-body", self.id).into())).flex_1().min_h_0().p_3().gap_3().children(self.children);
        let body = if self.scroll { body.overflow_y_scrollbar().into_any_element() } else { body.into_any_element() };
        v_flex().id(ElementId::Name(self.id)).test_support().size_full().bg(t.sidebar).child(header).child(body)
    }
}
