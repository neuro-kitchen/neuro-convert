//! The "include in the conversion" checkbox, used by the recording tree and the metadata form.

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::{App, IntoElement, RenderOnce, SharedString, Window};

/// How many of the items behind a checkbox are included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inclusion {
    All,
    Some,
    None,
}

impl Inclusion {
    pub fn of(included: usize, total: usize) -> Self {
        match included {
            0 => Inclusion::None,
            n if n == total => Inclusion::All,
            _ => Inclusion::Some,
        }
    }
}

type OnChange = Box<dyn Fn(bool, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct IncludeToggle {
    id: SharedString,
    state: Inclusion,
    on_change: OnChange,
}

impl IncludeToggle {
    pub fn new(id: impl Into<SharedString>, state: Inclusion, on_change: impl Fn(bool, &mut App) + 'static) -> Self {
        Self { id: id.into(), state, on_change: Box::new(on_change) }
    }
}

impl RenderOnce for IncludeToggle {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let tip = match self.state {
            Inclusion::Some => "Partly included — click to include all",
            _ => "Include in the conversion",
        };
        let on_change = self.on_change;
        Checkbox::new(self.id).checked(self.state == Inclusion::All).tooltip(tip).on_click(move |checked, _, cx| on_change(*checked, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::Inclusion;

    #[test]
    fn test_inclusion() {
        assert_eq!(Inclusion::of(0, 3), Inclusion::None);
        assert_eq!(Inclusion::of(2, 3), Inclusion::Some);
        assert_eq!(Inclusion::of(3, 3), Inclusion::All);
    }
}
