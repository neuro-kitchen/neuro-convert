//! A list of errors and warnings. Messages wrap (never run off the panel); an issue about a
//! field or an item gets a link to where it is fixed.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::{div, App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window};
use nc_convert::core::Target;

#[derive(Debug, Clone, PartialEq)]
pub struct IssueRow {
    pub error: bool,
    pub text: SharedString,
    /// Where it is fixed, for the link.
    pub target: Option<Target>,
}

type OnOpen = Rc<dyn Fn(Target, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct IssueList {
    id: SharedString,
    rows: Vec<IssueRow>,
    /// Shown when there are no issues.
    empty: Option<SharedString>,
    on_open: Option<OnOpen>,
}

impl IssueList {
    pub fn new(id: impl Into<SharedString>, rows: Vec<IssueRow>) -> Self {
        Self { id: id.into(), rows, empty: None, on_open: None }
    }

    pub fn when_empty(mut self, text: impl Into<SharedString>) -> Self {
        self.empty = Some(text.into());
        self
    }

    /// Adds a "Fix" link to issues with a target.
    pub fn on_open(mut self, f: impl Fn(Target, &mut Window, &mut App) + 'static) -> Self {
        self.on_open = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for IssueList {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        let (danger, warning, success) = (t.danger, t.warning, t.success);
        let mut list = v_flex().w_full().gap_1p5();
        if let (true, Some(e)) = (self.rows.is_empty(), self.empty) {
            list = list.child(div().text_sm().text_color(success).child(e));
        }
        let id = self.id;
        let on_open = self.on_open;
        list.children(self.rows.into_iter().enumerate().map(|(i, r)| {
            let (tag, color) = if r.error { ("Error", danger) } else { ("Warning", warning) };
            let link = match (&on_open, r.target) {
                (Some(f), Some(target)) => {
                    let (f, label) = (f.clone(), crate::domain::steps::target_label(&target));
                    Some(
                        Button::new((SharedString::from(format!("{id}-fix")), i))
                            .link()
                            .xsmall()
                            .icon(IconName::ArrowRight)
                            .label(label)
                            .on_click(move |_, window, cx| f(target.clone(), window, cx)),
                    )
                }
                _ => None,
            };
            h_flex()
                .w_full()
                .gap_2()
                .items_start()
                .text_sm()
                .child(div().flex_none().w_16().text_color(color).child(tag))
                .child(v_flex().flex_1().min_w_0().gap_0p5().child(div().w_full().child(crate::domain::format::plain_issue(&r.text))).children(link.map(|l| h_flex().child(l))))
        }))
    }
}
