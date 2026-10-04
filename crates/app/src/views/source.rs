//! ① Source: one way to open a recording, what each format expects, recent recordings, the
//! choice inside a tank, and what was detected.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, Context, Entity, FontWeight, InteractiveElement as _, StatefulInteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window};

use crate::actions::OpenRecording;
use crate::viewmodels::source::{Screen, SourceVm};
use crate::widgets::{Card, Muted};

pub struct SourceView {
    vm: Entity<SourceVm>,
    /// "What can be opened" shows each format's details (folded by default: one line of tags).
    formats_open: bool,
    _vm: Subscription,
}

impl SourceView {
    pub fn new(vm: Entity<SourceVm>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&vm, |_, _, cx| cx.notify());
        Self { vm, formats_open: false, _vm: sub }
    }
}

impl Render for SourceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vm = self.vm.read(cx);
        let screen = vm.screen(cx);
        let t = cx.theme();
        let (muted, border) = (t.muted_foreground, t.border);

        let open = Card::new()
            .title("Open a recording")
            .child(Muted::new("Choose the folder of the recording. You can also drop a folder or file on this window."))
            .child(
                h_flex().gap_3().child(
                    Button::new("open-recording")
                        .primary()
                        .icon(IconName::FolderOpen)
                        .label("Open recording…")
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenRecording), cx)),
                ),
            )
            .children(vm.detected.clone().map(|d| {
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(Tag::success().small().child(format!("Open: {}", d.format)))
                    .child(div().text_sm().child(d.detail))
                    .child(div().text_xs().text_color(muted).child(d.path))
            }));

        // What can be opened: one line of format tags (details on hover); the toggle shows what
        // each format expects. Long descriptions wrap; the maturity tag keeps its size.
        let maturity_tag = |f: &crate::viewmodels::source::FormatCard| {
            let (label, explain) = f.maturity.clone();
            let tag = if label == "verified" { Tag::success() } else { Tag::warning() };
            div().id(SharedString::from(format!("maturity-{}", f.name))).test_support().flex_none().child(tag.xsmall().child(label)).tooltip(move |window, cx| Tooltip::new(explain.clone()).build(window, cx))
        };
        let open_formats = self.formats_open;
        let view = cx.entity();
        let toggle = Button::new("formats-toggle")
            .ghost()
            .small()
            .icon(if open_formats { IconName::ChevronUp } else { IconName::ChevronDown })
            .label(if open_formats { "Hide details" } else { "What each format expects" })
            .on_click(move |_, _, cx| view.update(cx, |this, cx| {
                this.formats_open = !this.formats_open;
                cx.notify();
            }));
        let count = vm.formats.len();
        let formats = if open_formats {
            Card::new().title("What can be opened").aside(toggle).children(vm.formats.iter().enumerate().map(|(i, f)| {
                v_flex()
                    .gap_0p5()
                    .when(i + 1 < count, |d| d.pb_2().border_b_1().border_color(border))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_start()
                            .child(div().flex_none().child(Tag::secondary().small().child(f.name.to_uppercase())))
                            .child(div().flex_1().min_w_0().text_sm().font_weight(FontWeight::MEDIUM).child(f.description.clone()))
                            .child(maturity_tag(f)),
                    )
                    .child(div().text_sm().text_color(muted).child(f.opens.clone()))
            }))
        } else {
            Card::new().title("What can be opened").aside(toggle).child(h_flex().gap_2().flex_wrap().children(vm.formats.iter().map(|f| {
                let detail = format!("{}\n{}", f.description, f.opens);
                div()
                    .id(SharedString::from(format!("format-{}", f.name)))
                    .test_support()
                    .child(Tag::secondary().small().child(f.name.to_uppercase()))
                    .tooltip(move |window, cx| Tooltip::new(detail.clone()).build(window, cx))
            })))
        };

        let middle = match screen {
            Screen::Opening(path) => Some(Card::new().title("Opening…").child(Muted::new(format!("Reading {}", path.display())))),
            Screen::Choosing(choice) => Some(
                Card::new().title(format!("{} holds several recordings — choose one", choice.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()))).children(
                    choice.containers.into_iter().enumerate().map(|(i, name)| {
                        let (vm, path) = (self.vm.clone(), choice.path.clone());
                        Button::new(("container", i)).outline().icon(IconName::Folder).label(name.clone()).on_click(move |_, _, cx| SourceVm::open(&vm, path.clone(), Some(name.clone()), cx))
                    }),
                ),
            ),
            Screen::Pick => None,
        };

        let recent = (!vm.recent.is_empty()).then(|| {
            Card::new().title("Recent").children(vm.recent.iter().enumerate().map(|(i, r)| {
                let (vm, path) = (self.vm.clone(), r.path.clone());
                h_flex()
                    .id(("recent", i))
                    .gap_3()
                    .p_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(cx.theme().accent))
                    .on_click(move |_, _, cx| SourceVm::open(&vm, path.clone(), None, cx))
                    .child(gpui_kit::component::Icon::new(IconName::Folder).small())
                    .child(v_flex().flex_1().min_w_0().child(div().text_sm().font_weight(FontWeight::MEDIUM).child(r.name.clone())).child(div().text_xs().text_color(muted).child(r.short.clone())))
                    .children(r.format.clone().map(|f| Tag::secondary().small().child(f.to_uppercase())))
            }))
        });

        // A centered column that scrolls (like the other steps), so nothing runs under the footer
        let column = v_flex().w_full().max_w(gpui_kit::px(880.)).gap_4().child(open).children(middle).children(recent).child(formats);
        // (the id sits outside the scroll wrapper, which replaces its child's id)
        div().id("source-step").test_support().size_full().child(v_flex().size_full().p_6().items_center().child(column).overflow_y_scrollbar())
    }
}
