//! ④ Review & convert: a centered column with what will be written, the issues left (each links
//! to its fix), the output, the only Convert button, progress and the result. The NWB structure is
//! a panel on the right (its rows link back to their item); the write options are closed.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{div, px, AnyElement, Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window};
use nc_convert::core::Level;
use nc_convert::nwb::{ChunkPolicy, VerifyLevel};

use crate::actions::{Cancel, ChooseOutput, Convert};
use crate::viewmodels::nav::NavVm;
use crate::viewmodels::{ConvertVm, PlanVm};
use crate::widgets::{Card, FormRow, IssueList, IssueRow, MenuSelect, Muted, ProgressCard, Section, SidePanel};

pub struct ReviewView {
    plan: Entity<PlanVm>,
    convert: Entity<ConvertVm>,
    nav: Entity<NavVm>,
    advanced: bool,
    _subscriptions: Vec<Subscription>,
}

impl ReviewView {
    pub fn new(plan: Entity<PlanVm>, convert: Entity<ConvertVm>, nav: Entity<NavVm>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&plan, |_, _, cx| cx.notify()), cx.observe(&convert, |_, _, cx| cx.notify()), cx.observe(&nav, |_, _, cx| cx.notify())];
        Self { plan, convert, nav, advanced: false, _subscriptions: subscriptions }
    }

    fn toggle(&self, id: &'static str, open: bool, label: &'static str, cx: &mut Context<Self>, f: fn(&mut Self)) -> impl IntoElement + use<> {
        Button::new(id).ghost().small().icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }).label(label).on_click(cx.listener(move |this, _, _, cx| {
            f(this);
            cx.notify();
        }))
    }

    fn options(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = self.convert.read(cx).state.clone();
        let vm = self.convert.clone();
        let w = s.writing;
        let gzip = Switch::new("gzip").checked(s.options.gzip.is_some()).label("Compress (gzip level 1): smaller, slower to write").disabled(w).on_click({
            let vm = vm.clone();
            move |on, _, cx| vm.update(cx, |vm, cx| vm.set_gzip(*on, cx))
        });
        let chunk = |label: &'static str, policy: ChunkPolicy| {
            let vm = vm.clone();
            let b = Button::new(label).small().label(label).disabled(w);
            let b = if s.options.chunks == policy { b.primary() } else { b.outline() };
            b.on_click(move |_, _, cx| vm.update(cx, |vm, cx| vm.set_chunks(policy, cx)))
        };
        let verify = |label: &'static str, level: VerifyLevel| {
            let vm = vm.clone();
            let b = Button::new(label).small().label(label).disabled(w);
            let b = if s.options.verify == level { b.primary() } else { b.outline() };
            b.on_click(move |_, _, cx| vm.update(cx, |vm, cx| vm.set_verify(level, cx)))
        };
        let step = |id: &'static str, icon: IconName, delta: isize| {
            let vm = vm.clone();
            Button::new(id).outline().xsmall().icon(icon).disabled(w).on_click(move |_, _, cx| vm.update(cx, |vm, cx| vm.step_threads(delta, cx)))
        };
        v_flex()
            .gap_3()
            .pl_4()
            .child(gzip)
            .child(FormRow::new("Chunks", h_flex().gap_1().child(chunk("1 s", ChunkPolicy::Seconds(1.0))).child(chunk("auto (~10 MB)", ChunkPolicy::Auto))).help("How the data is split on disk; 1 s suits most readers"))
            .child(
                FormRow::new("Check after writing", h_flex().gap_1().child(verify("Sampled", VerifyLevel::Sampled)).child(verify("Full", VerifyLevel::Full)).child(verify("Off", VerifyLevel::Off)))
                    .help("Reads the store back and compares it with the source. Sampled: first, last and random blocks (seconds). Full: everything, plus the source files' own checksums (about as long as writing)"),
            )
            .child(
                FormRow::new("Threads", h_flex().gap_2().items_center().child(step("threads-down", IconName::Minus, -1)).child(div().text_sm().child(s.options.threads.to_string())).child(step("threads-up", IconName::Plus, 1)))
                    .help("CPUs used for writing; the default leaves some free for the app"),
            )
            .into_any_element()
    }
}

impl Render for ReviewView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = self.convert.read(cx).state.clone();
        if !s.opened {
            return v_flex().id("convert-view").test_support().size_full().p_6().child(Muted::new("Open a recording first.")).into_any_element();
        }
        let rows = self.plan.read(cx).rows.clone().unwrap_or_default();
        let output = self.convert.read(cx).output.clone();
        let folder = self.convert.read(cx).folder(cx);
        let issues: Vec<IssueRow> = self.nav.read(cx).issues(cx).into_iter().map(|i| IssueRow { error: i.level == Level::Error, text: i.message.into(), target: i.target }).collect();
        let nav = self.nav.clone();
        let theme = cx.theme();
        let (success, danger, muted, border, hover) = (theme.success, theme.danger, theme.muted_foreground, theme.border, theme.accent);

        let summary = Card::new()
            .title("What will be written")
            .child(div().text_base().child(rows.summary.clone()))
            .child(
                IssueList::new("review-issues", issues)
                    .when_empty("No issues: ready to convert.")
                    .on_open(move |target, _, cx| nav.update(cx, |nav, cx| nav.reveal(target, cx))),
            );

        // NWB on Zarr (a folder) or HDF5 (one file), when this build writes HDF5
        let format = self.convert.read(cx).store().read(cx).ws.output.as_deref().map_or(nc_convert::nwb::Format::Zarr, nc_convert::nwb::Format::of);
        let format_row = nc_convert::nwb::HDF5.then(|| {
            let store = self.convert.read(cx).store().clone();
            let options: Vec<SharedString> = vec!["Zarr folder (.nwb.zarr)".into(), "HDF5 file (.nwb)".into()];
            let selected = usize::from(format == nc_convert::nwb::Format::Hdf5);
            let pick = MenuSelect::new("output-format", options[selected].clone(), options, Some(selected), move |i, _, cx| {
                let format = if i == 1 { nc_convert::nwb::Format::Hdf5 } else { nc_convert::nwb::Format::Zarr };
                store.update(cx, |s, cx| s.apply(cx, |ws| {
                    let next = ws.output.as_deref().map(|p| nc_convert::nwb::backend::with_format(p, format));
                    ws.set_output(next)
                }));
            })
            .disabled(s.writing);
            FormRow::new("Format", pick).compact().help("Zarr: a folder of chunk files (DANDI, cloud). HDF5: one .nwb file (most NWB tools)")
        });
        let help = match format {
            nc_convert::nwb::Format::Hdf5 => "A file ending in .nwb; an existing one is replaced",
            nc_convert::nwb::Format::Zarr => "A folder ending in .nwb.zarr; an existing one is replaced",
        };
        let output_card = Card::new()
            .title("Output")
            .children(format_row)
            .child(
                FormRow::new(
                    "NWB store",
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_1()
                                .child(div().flex_1().child(Input::new(&output).small().disabled(s.writing)))
                                .child(Button::new("choose-output").small().outline().label("Choose…").disabled(s.writing).on_click(|_, window, cx| window.dispatch_action(Box::new(ChooseOutput), cx))),
                        )
                        .children(folder.map(|(short, full)| div().id("output-folder").text_xs().text_color(muted).child(format!("in {short}")).tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(full.clone()).build(window, cx)))),
                )
                .help(help),
            )
            .child(self.toggle("advanced", self.advanced, "Advanced: compression, chunks, checks, threads", cx, |this| this.advanced = !this.advanced))
            .children(self.advanced.then(|| self.options(cx)));

        let convert_vm = self.convert.clone();
        let buttons = h_flex()
            .gap_3()
            .items_center()
            .child(
                Button::new("convert-now")
                    .primary()
                    .large()
                    .icon(IconName::Play)
                    .label("Convert")
                    .disabled(!s.can_convert)
                    .tooltip("Write and verify the NWB store (Ctrl+Enter)")
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(Convert), cx)),
            )
            .child(Button::new("cancel-now").outline().icon(IconName::CircleX).label("Cancel").disabled(!s.can_cancel).on_click(|_, window, cx| window.dispatch_action(Box::new(Cancel), cx)))
            .children(s.blocked_reason.clone().filter(|_| !s.writing).map(|r| div().text_sm().text_color(danger).child(r)))
            .child(div().flex_1())
            .child(Button::new("diagnostics").ghost().small().icon(IconName::Copy).label("Copy diagnostics").on_click(move |_, _, cx| convert_vm.update(cx, |vm, cx| vm.copy_diagnostics(cx))));

        let result = s.result.clone().map(|r| {
            let color = match r.ok {
                Some(true) => success,
                Some(false) => danger,
                None => muted,
            };
            let reveal = r.output.map(|out| Button::new("reveal").small().outline().icon(IconName::FolderOpen).label("Show output").on_click(move |_, _, cx| cx.reveal_path(&out)));
            let rows = r.issues.into_iter().collect();
            Card::new()
                .title("Result")
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(color).child(r.headline))
                .children(r.summary.map(Muted::new))
                .child(IssueList::new("verify-issues", rows))
                .child(h_flex().gap_2().flex_wrap().children(reveal).children(r.report.map(|p| Muted::new(format!("Report: {}", p.display())))))
        });

        let structure_open = self.convert.read(cx).structure_open(cx);
        let structure_toggle = {
            let vm = self.convert.clone();
            let b = Button::new("toggle-structure").ghost().small().icon(IconName::PanelRight).label("NWB structure").tooltip("Show or hide what goes where in the NWB file");
            let b = if structure_open { b.selected(true) } else { b };
            b.on_click(move |_, _, cx| vm.update(cx, |vm, cx| vm.set_structure_open(!structure_open, cx)))
        };
        let summary = summary.aside(structure_toggle);

        let column = v_flex()
            .id("convert-view")
            .test_support()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_6()
            .items_center()
            .child(
                v_flex()
                    .gap_4()
                    .w_full()
                    .max_w(px(920.))
                    .child(summary)
                    .child(output_card)
                    .child(buttons)
                    .children(s.progress.map(|(pct, line)| ProgressCard::new(pct, line)))
                    .children(result),
            )
            .overflow_y_scrollbar();

        let panel = structure_open.then(|| {
            let vm = self.convert.clone();
            let link = |id: (SharedString, usize), row: &crate::widgets::PathRow| {
                let nav = self.nav.clone();
                let target = row.target.clone();
                h_flex()
                    .id(id)
                    .test_support()
                    .w_full()
                    .rounded_md()
                    .px_1()
                    .when(target.is_some(), |this| this.cursor_pointer().hover(|s| s.bg(hover)))
                    .on_click(move |_, _, cx| {
                        if let Some(t) = target.clone() {
                            nav.update(cx, |nav, cx| nav.reveal(t, cx));
                        }
                    })
                    .child(row.clone())
            };
            let mut sections = vec![Section::new("File").children(rows.file.iter().cloned().map(Muted::new)).into_any_element()];
            for (k, (title, paths)) in rows.sections.iter().enumerate().filter(|(_, (_, p))| !p.is_empty()) {
                sections.push(Section::new(title.clone()).children(paths.iter().enumerate().map(|(i, p)| link((SharedString::from(format!("structure-{k}")), i), p))).into_any_element());
            }
            if !rows.skipped.is_empty() {
                sections.push(Section::new(format!("Left out ({})", rows.skipped.len())).children(rows.skipped.iter().cloned().map(Muted::new)).into_any_element());
            }
            div()
                .w(px(440.))
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(border)
                .child(SidePanel::new("structure-panel", "NWB structure", move |_, cx| vm.update(cx, |vm, cx| vm.set_structure_open(false, cx))).child(Muted::new("Click a row to open its item.")).children(sections))
        });

        h_flex()
            .size_full()
            .child(column)
            .children(panel)
            .into_any_element()
    }
}
