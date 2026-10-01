//! The preview: stream / channels / markers dropdowns, labelled time controls, gain, traces with
//! aligned channel labels, a time axis, an overview of the whole recording and a scale bar.
//! Mouse wheel: channels; Ctrl+wheel: zoom; Shift+wheel: pan.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, Disableable as _, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    canvas, div, fill, point, px, relative, size, Bounds, Context, Entity, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Pixels,
    Render, ScrollWheelEvent, SharedString, Styled as _, Subscription, Window,
};

use crate::viewmodels::PreviewVm;
use crate::widgets::{MenuSelect, Muted, ProbeMap, Traces};

/// Width of the channel label column.
const LABELS: f32 = 84.;

pub struct PreviewView {
    vm: Entity<PreviewVm>,
    /// Wheel movement not yet turned into a step (trackpads send many small deltas).
    wheel: f32,
    /// Where the overview bar was drawn (to turn a click into a time).
    overview: Rc<Cell<Bounds<Pixels>>>,
    _vm: Subscription,
}

impl PreviewView {
    pub fn new(vm: Entity<PreviewVm>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&vm, |_, _, cx| cx.notify());
        Self { vm, wheel: 0.0, overview: Rc::new(Cell::new(Bounds::default())), _vm: sub }
    }

    fn on_wheel(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let d = e.delta.pixel_delta(px(20.));
        let dy = if d.y != px(0.) { d.y.as_f32() } else { d.x.as_f32() };
        self.wheel += dy;
        if self.wheel.abs() >= 20.0 {
            let step = self.wheel;
            self.wheel = 0.0;
            let (ctrl, shift) = (e.modifiers.control || e.modifiers.platform, e.modifiers.shift || d.x != px(0.));
            self.vm.update(cx, |vm, cx| vm.wheel(step, ctrl, shift, cx));
        }
        cx.stop_propagation();
    }
}

impl Render for PreviewView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vm = self.vm.read(cx);
        let Some(rec) = vm.recording.clone() else {
            return v_flex().id("preview-view").test_support().size_full().child(Muted::new("Select a stream to see its signals.")).into_any_element();
        };
        let info = rec.info();
        let key = vm.key().expect("recording set");
        let rate = info.sample_rate.max(1e-9);
        let t = cx.theme();
        let (fg, muted, accent, danger, border, warning) = (t.foreground, t.muted_foreground, t.primary, t.danger, t.border, t.warning);

        // Dropdowns
        let handle = self.vm.clone();
        let streams = vm.streams(cx);
        let stream_ix = streams.iter().position(|s| *s == info.name);
        let stream_select = {
            let (vm, names) = (handle.clone(), streams.clone());
            MenuSelect::new("preview-stream", info.name.clone(), streams.iter().map(|s| SharedString::from(s.clone())).collect(), stream_ix, move |i, _, cx| {
                if let Some(n) = names.get(i) {
                    PreviewVm::select_stream(&vm, n.clone(), cx);
                }
            })
        };
        let set_select = {
            let vm = handle.clone();
            let current = vm.read(cx).sets.get(vm.read(cx).set).map_or_else(|| "Channels".to_string(), |s| s.label.clone());
            MenuSelect::new("preview-channels", current, vm.read(cx).sets.iter().map(|s| SharedString::from(s.label.clone())).collect(), Some(vm.read(cx).set), move |i, _, cx| {
                vm.update(cx, |vm, cx| vm.select_set(i, cx))
            })
        };
        let events = vm.event_names(cx);
        let marker_select = {
            let vm = handle.clone();
            let names = events.clone();
            let mut options: Vec<SharedString> = vec!["No markers".into()];
            options.extend(events.iter().map(|e| SharedString::from(format!("Markers: {e}"))));
            let selected = vm.read(cx).marker.as_ref().and_then(|m| names.iter().position(|n| n == m)).map_or(0, |i| i + 1);
            MenuSelect::new("preview-markers", options[selected].clone(), options, Some(selected), move |i, _, cx| {
                let name = i.checked_sub(1).and_then(|i| names.get(i).cloned());
                vm.update(cx, |vm, cx| vm.set_marker(name, cx))
            })
        };
        let button = |id: &'static str, label: &'static str, icon: Option<IconName>, tip: &'static str, enabled: bool, f: fn(&mut PreviewVm, &mut Context<PreviewVm>)| {
            let vm = handle.clone();
            let b = Button::new(id).ghost().xsmall().tooltip(tip).disabled(!enabled);
            let b = match icon {
                Some(i) => b.icon(i),
                None => b,
            };
            let b = if label.is_empty() { b } else { b.label(label) };
            b.on_click(move |_, _, cx| vm.update(cx, f))
        };
        let fit_each = vm.fit_each;
        let fit = {
            let vm = handle.clone();
            Switch::new("fit-each").small().checked(fit_each).label("Fit each channel").on_click(move |on, _, cx| vm.update(cx, |vm, cx| vm.set_fit_each(*on, cx)))
        };
        let controls = h_flex()
            .gap_2()
            .flex_wrap()
            .child(stream_select)
            .child(set_select)
            .when(!events.is_empty(), |this| this.child(marker_select))
            .child(div().w(px(1.)).h(px(18.)).bg(border))
            .child(button("pan-left", "Earlier", Some(IconName::ChevronLeft), "Show the previous window (Shift+wheel)", vm.can_pan(-1.0), |vm, cx| vm.pan(-1.0, cx)))
            .child(button("pan-right", "Later", Some(IconName::ChevronRight), "Show the next window (Shift+wheel)", vm.can_pan(1.0), |vm, cx| vm.pan(1.0, cx)))
            .child(button("zoom-out", "", Some(IconName::Minus), "Zoom out: twice the time (Ctrl+wheel)", true, |vm, cx| vm.zoom(2.0, cx)))
            .child(button("zoom-in", "", Some(IconName::Plus), "Zoom in: half the time (Ctrl+wheel)", true, |vm, cx| vm.zoom(0.5, cx)))
            .child(h_flex().gap_1().child(div().text_xs().text_color(muted).child("Gain")).child(div().w(px(110.)).child(Slider::new(&vm.gain))))
            .child(fit);

        let window_text = format!("{:.3}–{:.3} s of {:.1} s", key.samples.start as f64 / rate, key.samples.end as f64 / rate, info.duration());
        let status = vm.error.clone().map(|e| (true, e)).or_else(|| vm.waiting.then(|| (false, "reading…".to_string())));
        let lanes = vm.shown.as_ref().filter(|(k, _)| k.recording == info.name).map(|(_, l)| l.clone());
        let scale = vm.scale(cx);
        let gain = vm.gain_value(cx);
        let labels: Vec<String> = key.channels.iter().map(|&c| info.channels.get(c).map_or_else(|| (c + 1).to_string(), |ch| ch.name.clone())).collect();
        let sites = vm.sites(cx);
        let markers = vm.marker_positions(cx);
        let ticks = vm.ticks();
        let (start, span, total) = (vm.start, vm.span, vm.duration().max(1e-9));

        let traces = Traces::new(lanes, gain, fg).shared_peak(scale.as_ref().map(|s| s.0)).scale_bar(scale.as_ref().map(|s| s.1)).markers(markers, warning.opacity(0.7));
        let label_col = v_flex().w(px(LABELS)).flex_none().children(labels.into_iter().map(|l| div().flex_1().min_h_0().flex().items_center().text_xs().text_color(muted).overflow_hidden().child(l)));
        let main = div()
            .id("preview-traces")
            .flex()
            .flex_row()
            .flex_1()
            .min_h_0()
            .gap_1()
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .child(label_col)
            .child(div().flex_1().h_full().child(traces))
            .when(!sites.is_empty(), |this| this.child(ProbeMap::new(sites, muted, accent)));
        let axis = h_flex().h(px(16.)).child(div().w(px(LABELS + 4.)).flex_none()).child(
            div().relative().flex_1().h_full().children(ticks.into_iter().map(|(x, label)| div().absolute().left(relative(x)).top_0().text_xs().text_color(muted).child(label))),
        );
        let bounds = self.overview.clone();
        let overview = h_flex().h(px(10.)).child(div().w(px(LABELS + 4.)).flex_none()).child(
            div()
                .id("preview-overview")
                .flex_1()
                .h_full()
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, {
                    let (vm, bounds) = (handle.clone(), bounds.clone());
                    move |e: &MouseDownEvent, _, cx| {
                        let b = bounds.get();
                        let w = b.size.width.as_f32();
                        if w > 0.0 {
                            let f = ((e.position.x - b.origin.x).as_f32() / w) as f64;
                            vm.update(cx, |vm, cx| vm.jump(f, cx));
                        }
                    }
                })
                .child(
                    canvas(
                        move |b, _, _| bounds.set(b),
                        move |b: Bounds<Pixels>, _, window, _| {
                            let w = b.size.width.as_f32();
                            window.paint_quad(fill(b, border));
                            let x0 = (start / total) as f32 * w;
                            let x1 = (((start + span) / total) as f32 * w).max(x0 + 2.0);
                            window.paint_quad(fill(Bounds::new(point(b.origin.x + px(x0), b.origin.y), size(px(x1 - x0), b.size.height)), accent));
                        },
                    )
                    .size_full(),
                ),
        );
        let legend = match &scale {
            Some((_, _, label)) => format!("Scale bar (right) = {label} · one scale for all channels · values in {}", info.unit),
            None => format!("Each channel fitted to its own peak × gain · values in {}", info.unit),
        };
        v_flex()
            .id("preview-view")
            .test_support()
            .size_full()
            .gap_1()
            .child(controls)
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_xs().text_color(muted).child(window_text))
                    .children(status.map(|(err, text)| div().text_xs().text_color(if err { danger } else { muted }).child(text))),
            )
            .child(main)
            .child(axis)
            .child(overview)
            .child(Muted::new(legend))
            .into_any_element()
    }
}
