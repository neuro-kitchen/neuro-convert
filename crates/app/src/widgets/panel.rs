//! A side panel (tree, inspector, NWB structure): a header with its title and a close button,
//! and a scrolling body. Panels are shown and hidden by their view; widths come from the
//! resizable group around them.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, AnyElement, App, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement, RenderOnce, SharedString, Styled as _, Window};

type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct SidePanel {
    id: SharedString,
    title: SharedString,
    on_close: OnClose,
    /// Scroll the body (off for bodies that scroll themselves, like the tree).
    scroll: bool,
    children: Vec<AnyElement>,
}

impl SidePanel {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>, on_close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { id: id.into(), title: title.into(), on_close: Rc::new(on_close), scroll: true, children: Vec::new() }
    }

    pub fn no_scroll(mut self) -> Self {
        self.scroll = false;
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
        let header = h_flex()
            .flex_none()
            .justify_between()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(t.border)
            .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(t.muted_foreground).child(self.title.to_uppercase()))
            .child(
                Button::new(ElementId::Name(format!("{}-close", self.id).into()))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip("Hide this panel")
                    .on_click(move |_, window, cx| on_close(window, cx)),
            );
        let body = v_flex().id(ElementId::Name(format!("{}-body", self.id).into())).flex_1().min_h_0().p_3().gap_3().children(self.children);
        let body = if self.scroll { body.overflow_y_scrollbar().into_any_element() } else { body.into_any_element() };
        v_flex().id(ElementId::Name(self.id)).test_support().size_full().bg(t.background).child(header).child(body)
    }
}
