//! ① Source: one way to open a recording, what each format expects, recent recordings, the
//! choice inside a tank, and what was detected.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, Context, Entity, FontWeight, InteractiveElement as _, StatefulInteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Subscription, Window};

use crate::actions::OpenRecording;
use crate::viewmodels::source::{Screen, SourceVm};
use crate::widgets::{Card, Muted};

pub struct SourceView {
    vm: Entity<SourceVm>,
    _vm: Subscription,
}

impl SourceView {
    pub fn new(vm: Entity<SourceVm>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&vm, |_, _, cx| cx.notify());
        Self { vm, _vm: sub }
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

        let formats = Card::new().title("What can be opened").children(vm.formats.iter().map(|f| {
            v_flex()
                .gap_0p5()
                .pb_2()
                .border_b_1()
                .border_color(border)
                .child(h_flex().gap_2().child(Tag::secondary().small().child(f.name.to_uppercase())).child(div().text_sm().font_weight(FontWeight::MEDIUM).child(f.description.clone())))
                .child(div().text_sm().text_color(muted).child(f.opens.clone()))
        }));

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

        v_flex()
            .id("source-step")
            .test_support()
            .size_full()
            .p_6()
            .gap_4()
            .max_w(gpui_kit::px(880.))
            .child(open)
            .children(middle)
            .children(recent)
            .child(formats)
    }
}
