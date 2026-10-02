//! Layout pieces: a titled section, a labelled field row, a plan path row, secondary text, an
//! empty state.

use gpui_kit::component::empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Icon, IconName};
use gpui_kit::{div, relative, AnyElement, App, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString, Styled as _, Window};

/// A titled block: the title stands above a bordered box with the block's rows, so a panel
/// reads as a few groups rather than one list of equal lines.
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
        let t = cx.theme();
        v_flex()
            .gap_1p5()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(t.foreground).child(self.title))
            .child(v_flex().gap_3().p_3().rounded_lg().border_1().border_color(t.border).bg(t.background).children(self.children))
    }
}

/// `path  ← source  detail` (the NWB plan).
#[derive(IntoElement, Clone)]
pub struct PathRow {
    pub path: SharedString,
    pub source: SharedString,
    pub detail: SharedString,
    /// The item it is written from (for links back to it).
    pub target: Option<nc_convert::core::Target>,
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

/// What an empty region is for: an icon, a title and one line, centered in the region.
#[derive(IntoElement)]
pub struct EmptyState {
    icon: IconName,
    title: SharedString,
    description: SharedString,
}

impl EmptyState {
    pub fn new(icon: IconName, title: impl Into<SharedString>, description: impl Into<SharedString>) -> Self {
        Self { icon, title: title.into(), description: description.into() }
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let header = EmptyHeader::new()
            .media(EmptyMedia::new().with_variant(EmptyMediaVariant::Icon).size_12().child(Icon::new(self.icon).size_6()))
            .title(EmptyTitle::new().text_lg().child(self.title))
            .description(EmptyDescription::new().text_base().child(self.description));
        v_flex().size_full().items_center().justify_center().child(Empty::new().flex_none().header(header))
    }
}

/// Seconds as `1 h 02 min`, `3 min 05 s`, `4.20 s` or `33 ms`.
pub fn duration(seconds: f64) -> String {
    let s = seconds.max(0.0);
    if s >= 3600.0 {
        format!("{} h {:02} min", (s / 3600.0) as u64, ((s % 3600.0) / 60.0) as u64)
    } else if s >= 60.0 {
        format!("{} min {:02} s", (s / 60.0) as u64, (s % 60.0) as u64)
    } else if s >= 1.0 {
        format!("{s:.2} s")
    } else {
        format!("{:.0} ms", s * 1e3)
    }
}

#[cfg(test)]
mod tests {
    use super::duration;

    #[test]
    fn test_duration() {
        assert_eq!(duration(4.2), "4.20 s");
        assert_eq!(duration(0.0333), "33 ms");
        assert_eq!(duration(185.0), "3 min 05 s");
        assert_eq!(duration(2832.7), "47 min 12 s");
        assert_eq!(duration(3725.0), "1 h 02 min");
    }
}
