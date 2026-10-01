//! A progress bar with a line of text under it.

use gpui_kit::component::progress::Progress;
use gpui_kit::component::v_flex;
use gpui_kit::{App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window};

use super::Muted;

#[derive(IntoElement)]
pub struct ProgressCard {
    percent: f32,
    line: SharedString,
}

impl ProgressCard {
    pub fn new(percent: f32, line: impl Into<SharedString>) -> Self {
        Self { percent, line: line.into() }
    }
}

impl RenderOnce for ProgressCard {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        v_flex().gap_1().child(Progress::new("progress").value(self.percent)).child(Muted::new(self.line))
    }
}
