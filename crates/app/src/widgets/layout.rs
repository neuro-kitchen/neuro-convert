//! Layout pieces: a titled section, a labelled field row, a plan path row, secondary text.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _};
use gpui_kit::{div, relative, AnyElement, App, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled as _, Window};

/// A titled block.
#[derive(IntoElement)]
pub struct Section {
    title: SharedString,
    children: Vec<AnyElement>,
}

impl Section {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self { title: title.into(), children: Vec::new() }
    }
}

impl ParentElement for Section {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Section {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .gap_2()
            .pb_3()
            .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(cx.theme().muted_foreground).child(self.title.to_uppercase()))
            .children(self.children)
    }
}

/// `path  ← source  detail` (the NWB plan).
#[derive(IntoElement, Clone)]
pub struct PathRow {
    pub path: SharedString,
    pub source: SharedString,
    pub detail: SharedString,
}

impl RenderOnce for PathRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        // Wraps instead of cutting text off
        h_flex()
            .w_full()
            .gap_3()
            .items_start()
            .text_sm()
            .child(div().flex_1().min_w_0().child(self.path))
            .child(div().w(relative(0.4)).flex_none().text_xs().text_color(cx.theme().muted_foreground).child(format!("from {} · {}", self.source, self.detail)))
    }
}

/// Secondary text.
#[derive(IntoElement)]
pub struct Muted(pub SharedString);

impl Muted {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self(text.into())
    }
}

impl RenderOnce for Muted {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div().text_sm().text_color(cx.theme().muted_foreground).child(self.0)
    }
}

/// Seconds as `1 h 02 min`, `3 min 05 s` or `4.20 s`.
pub fn duration(seconds: f64) -> String {
    let s = seconds.max(0.0);
    if s >= 3600.0 {
        format!("{} h {:02} min", (s / 3600.0) as u64, ((s % 3600.0) / 60.0) as u64)
    } else if s >= 60.0 {
        format!("{} min {:02} s", (s / 60.0) as u64, (s % 60.0) as u64)
    } else {
        format!("{s:.2} s")
    }
}

#[cfg(test)]
mod tests {
    use super::duration;

    #[test]
    fn test_duration() {
        assert_eq!(duration(4.2), "4.20 s");
        assert_eq!(duration(185.0), "3 min 05 s");
        assert_eq!(duration(2832.7), "47 min 12 s");
        assert_eq!(duration(3725.0), "1 h 02 min");
    }
}
