//! The preview: stream / channels / lanes / markers dropdowns, zoom and gain, traces with aligned
//! channel labels and a channel scrollbar, a time axis, an overview of the whole recording and a
//! scale bar.
//!
//! Navigation without buttons: drag the traces (or swipe a trackpad sideways, or Shift+wheel) to
//! move in time; Ctrl+wheel zooms around the pointer; the wheel scrolls channels; the overview's
//! window is dragged to move and its edges to zoom. Keys (after a click on the traces): ← / →
//! (Shift: half a window), + / −, ↑ / ↓, Page Up / Down, Home / End.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme as _, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::{
    canvas, div, fill, point, px, relative, size, Bounds, Context, CursorStyle, Entity, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render, ScrollWheelEvent, SharedString, Styled as _, Subscription, Window,
};

use crate::viewmodels::preview::LANE_CHOICES;
use crate::viewmodels::PreviewVm;
use crate::widgets::{MenuSelect, Muted, ProbeMap, Traces};

/// Width of the channel label column.
const LABELS: f32 = 84.;
/// Pixels of an overview window edge that resize instead of move.
const EDGE: f32 = 5.;

/// What a mouse drag is doing.
#[derive(Debug, Clone, Copy)]
enum Drag {
    /// Moving the traces: pointer x and view start when it began.
    Pan { x: f32, start: f64 },
    /// Moving the overview window: seconds between its start and the pointer.
    Window { grab: f64 },
    /// Moving an edge of the overview window.
    Edge { left: bool },
    /// The channel scrollbar.
    Channels,
}

pub struct PreviewView {
    vm: Entity<PreviewVm>,
    focus: FocusHandle,
    /// Wheel movement not yet turned into channel steps.
    wheel: f32,
    drag: Option<Drag>,
    /// Where the traces, the overview bar and the channel scrollbar were drawn.
    traces: Rc<Cell<Bounds<Pixels>>>,
    overview: Rc<Cell<Bounds<Pixels>>>,
    scrollbar: Rc<Cell<Bounds<Pixels>>>,
    _vm: Subscription,
}

/// x of `pos` across `b`, 0..1.
fn across(b: Bounds<Pixels>, x: Pixels) -> f64 {
    let w = b.size.width.as_f32();
    if w <= 0.0 { 0.0 } else { (((x - b.origin.x).as_f32()) / w).clamp(0.0, 1.0) as f64 }
}

impl PreviewView {
    pub fn new(vm: Entity<PreviewVm>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&vm, |_, _, cx| cx.notify());
        let cell = || Rc::new(Cell::new(Bounds::default()));
        Self { vm, focus: cx.focus_handle(), wheel: 0.0, drag: None, traces: cell(), overview: cell(), scrollbar: cell(), _vm: sub }
    }

    fn on_wheel(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let d = e.delta.pixel_delta(px(20.));
        let (dx, dy) = (d.x.as_f32(), d.y.as_f32());
        let b = self.traces.get();
        let w = b.size.width.as_f32().max(1.0) as f64;
        let ctrl = e.modifiers.control || e.modifiers.platform;
        self.vm.update(cx, |vm, cx| {
            if ctrl {
                // Zoom around the pointer: up = closer
                let anchor = across(b, e.position.x);
                vm.zoom_at((-(dy as f64) * 0.004).exp(), anchor, cx);
            } else if dx != 0.0 || e.modifiers.shift {
                // Sideways (trackpad) or Shift+wheel: move in time, content follows the fingers
                let moved = if dx != 0.0 { dx } else { dy } as f64;
                vm.set_view(vm.start - moved / w * vm.span, vm.span, cx);
            }
        });
        if !ctrl && dx == 0.0 && !e.modifiers.shift {
            self.wheel += dy;
            let steps = (self.wheel / 20.0).trunc();
            if steps != 0.0 {
                self.wheel -= steps * 20.0;
                // Three channels per notch; down shows later channels
                self.vm.update(cx, |vm, cx| vm.scroll_channels(-(steps as isize) * 3, cx));
            }
        }
        cx.stop_propagation();
    }

    fn on_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let shift = e.keystroke.modifiers.shift;
        let handled = self.vm.update(cx, |vm, cx| {
            let lanes = vm.lanes as isize;
            match e.keystroke.key.as_str() {
                "left" => vm.pan(if shift { -0.5 } else { -0.1 }, cx),
                "right" => vm.pan(if shift { 0.5 } else { 0.1 }, cx),
                "+" | "=" => vm.zoom(0.5, cx),
                "-" => vm.zoom(2.0, cx),
                "up" => vm.scroll_channels(-1, cx),
                "down" => vm.scroll_channels(1, cx),
                "pageup" => vm.scroll_channels(-lanes, cx),
                "pagedown" => vm.scroll_channels(lanes, cx),
                "home" => vm.set_view(0.0, vm.span, cx),
                "end" => vm.set_view(vm.duration(), vm.span, cx),
                _ => return false,
            }
            true
        });
        if handled {
            cx.stop_propagation();
        }
    }

    fn on_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let vm = self.vm.read(cx);
        let (start, span, total) = (vm.start, vm.span, vm.duration().max(1e-9));
        let (traces, overview, scrollbar) = (self.traces.get(), self.overview.get(), self.scrollbar.get());
        self.drag = if overview.contains(&e.position) {
            let w = overview.size.width.as_f32();
            let x = (e.position.x - overview.origin.x).as_f32();
            let (x0, x1) = ((start / total) as f32 * w, ((start + span) / total) as f32 * w);
            let t = across(overview, e.position.x) * total;
            if (x - x0).abs() <= EDGE {
                Some(Drag::Edge { left: true })
            } else if (x - x1).abs() <= EDGE {
                Some(Drag::Edge { left: false })
            } else if x > x0 && x < x1 {
                Some(Drag::Window { grab: t - start })
            } else {
                // Outside the window: center it there, then keep dragging it
                self.vm.update(cx, |vm, cx| vm.set_view(t - span / 2.0, span, cx));
                Some(Drag::Window { grab: span / 2.0 })
            }
        } else if scrollbar.contains(&e.position) {
            self.scroll_to(e.position.y, cx);
            Some(Drag::Channels)
        } else if traces.contains(&e.position) {
            Some(Drag::Pan { x: e.position.x.as_f32(), start })
        } else {
            None
        };
        cx.stop_propagation();
    }

    fn scroll_to(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let b = self.scrollbar.get();
        let h = b.size.height.as_f32();
        if h <= 0.0 {
            return;
        }
        let f = ((y - b.origin.y).as_f32() / h).clamp(0.0, 1.0);
        self.vm.update(cx, |vm, cx| {
            let n = vm.set_channels().len();
            vm.set_first((f * n as f32) as isize - vm.lanes as isize / 2, cx);
        });
    }

    fn on_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else { return };
        if e.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            return;
        }
        let (traces, overview) = (self.traces.get(), self.overview.get());
        self.vm.update(cx, |vm, cx| {
            let total = vm.duration().max(1e-9);
            match drag {
                Drag::Pan { x, start } => {
                    let w = traces.size.width.as_f32().max(1.0) as f64;
                    vm.set_view(start - (e.position.x.as_f32() - x) as f64 / w * vm.span, vm.span, cx);
                }
                Drag::Window { grab } => vm.set_view(across(overview, e.position.x) * total - grab, vm.span, cx),
                Drag::Edge { left } => {
                    let t = across(overview, e.position.x) * total;
                    let (a, b) = (vm.start, vm.start + vm.span);
                    let (a, b) = if left { (t.min(b - 0.002), b) } else { (a, t.max(a + 0.002)) };
                    vm.set_view(a, b - a, cx);
                }
                Drag::Channels => {}
            }
        });
        if let Drag::Channels = drag {
            self.scroll_to(e.position.y, cx);
        }
    }

    fn on_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.drag = None;
    }
}

/// Records where its parent was drawn.
fn measure(cell: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |b, _, _| cell.set(b), |_, _, _, _| {}).absolute().size_full()
}

impl Render for PreviewView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vm = self.vm.read(cx);
        let Some(rec) = vm.recording.clone() else {
            return v_flex().id("preview-view").test_support().size_full().child(Muted::new("Select a stream to see its signals.")).into_any_element();
        };
        let info = rec.info();
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
        let lanes_select = {
            let vm = handle.clone();
            let current = vm.read(cx).lanes;
            MenuSelect::new(
                "preview-lanes",
                format!("{current} lanes"),
                LANE_CHOICES.iter().map(|n| SharedString::from(format!("{n} lanes"))).collect(),
                LANE_CHOICES.iter().position(|n| *n == current),
                move |i, _, cx| vm.update(cx, |vm, cx| vm.set_lanes(LANE_CHOICES[i], cx)),
            )
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
        let button = |id: &'static str, icon: IconName, tip: &'static str, f: fn(&mut PreviewVm, &mut Context<PreviewVm>)| {
            let vm = handle.clone();
            Button::new(id).ghost().xsmall().icon(icon).tooltip(tip).on_click(move |_, _, cx| vm.update(cx, f))
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
            .child(lanes_select)
            .when(!events.is_empty(), |this| this.child(marker_select))
            .child(div().w(px(1.)).h(px(18.)).bg(border))
            .child(button("zoom-out", IconName::Minus, "Zoom out: twice the time (−, or Ctrl+wheel)", |vm, cx| vm.zoom(2.0, cx)))
            .child(button("zoom-in", IconName::Plus, "Zoom in: half the time (+, or Ctrl+wheel)", |vm, cx| vm.zoom(0.5, cx)))
            .child(h_flex().gap_1().child(div().text_xs().text_color(muted).child("Gain")).child(div().w(px(110.)).child(Slider::new(&vm.gain))))
            .child(fit);

        let (start, span, total) = (vm.start, vm.span, vm.duration().max(1e-9));
        let shown_channels = vm.visible_channels();
        let all = vm.set_channels().len();
        let window_text = format!(
            "{:.3}–{:.3} s of {:.1} s · channels {}–{} of {all}",
            start,
            start + span,
            info.duration(),
            vm.first + 1,
            vm.first + shown_channels.len()
        );
        let status = vm.error.clone().map(|e| (true, e)).or_else(|| vm.waiting.then(|| (false, "reading…".to_string())));
        let lanes = vm.visible();
        let scale = vm.scale(cx);
        let gain = vm.gain_value(cx);
        let labels: Vec<String> = shown_channels.iter().map(|&c| info.channels.get(c).map_or_else(|| (c + 1).to_string(), |ch| ch.name.clone())).collect();
        let sites = vm.sites(cx);
        let markers = vm.marker_positions(cx);
        let ticks = vm.ticks();
        let (first, lanes_n) = (vm.first, vm.lanes);

        let traces = Traces::new(lanes, gain, fg).shared_peak(scale.as_ref().map(|s| s.0)).scale_bar(scale.as_ref().map(|s| s.1)).markers(markers, warning.opacity(0.7));
        let label_col = v_flex().w(px(LABELS)).flex_none().children(labels.into_iter().map(|l| div().flex_1().min_h_0().flex().items_center().text_xs().text_color(muted).overflow_hidden().child(l)));
        // Channel scrollbar: where the shown lanes are in the set
        let scrollbar = div().id("preview-channel-scroll").test_support().relative().w(px(8.)).flex_none().h_full().cursor_pointer().child(measure(self.scrollbar.clone())).child(
            canvas(
                |_, _, _| (),
                move |b: Bounds<Pixels>, _, window, _| {
                    window.paint_quad(fill(b, border));
                    if all > 0 {
                        let h = b.size.height.as_f32();
                        let y0 = first as f32 / all as f32 * h;
                        let y1 = ((first + lanes_n).min(all) as f32 / all as f32 * h).max(y0 + 6.0);
                        window.paint_quad(fill(Bounds::new(point(b.origin.x, b.origin.y + px(y0)), size(b.size.width, px(y1 - y0))), muted));
                    }
                },
            )
            .size_full(),
        );
        let main = div()
            .id("preview-traces")
            .test_support()
            .track_focus(&self.focus)
            .flex()
            .flex_row()
            .flex_1()
            .min_h_0()
            .gap_1()
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .on_key_down(cx.listener(Self::on_key))
            .child(label_col)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h_full()
                    .cursor(if matches!(self.drag, Some(Drag::Pan { .. })) { CursorStyle::ClosedHand } else { CursorStyle::OpenHand })
                    .child(measure(self.traces.clone()))
                    .child(traces),
            )
            .child(scrollbar)
            .when(!sites.is_empty(), |this| this.child(ProbeMap::new(sites, muted, accent)));
        let axis = h_flex().h(px(16.)).child(div().w(px(LABELS + 4.)).flex_none()).child(
            div().relative().flex_1().h_full().children(ticks.into_iter().map(|(x, label)| div().absolute().left(relative(x)).top_0().text_xs().text_color(muted).child(label))),
        );
        let overview = h_flex().h(px(14.)).child(div().w(px(LABELS + 4.)).flex_none()).child(
            div().id("preview-overview").test_support().relative().flex_1().h_full().cursor(CursorStyle::ResizeLeftRight).child(measure(self.overview.clone())).child(
                canvas(
                    |_, _, _| (),
                    move |b: Bounds<Pixels>, _, window, _| {
                        let w = b.size.width.as_f32();
                        window.paint_quad(fill(b, border));
                        let x0 = (start / total) as f32 * w;
                        let x1 = (((start + span) / total) as f32 * w).max(x0 + 4.0);
                        let win = Bounds::new(point(b.origin.x + px(x0), b.origin.y), size(px(x1 - x0), b.size.height));
                        window.paint_quad(fill(win, accent.opacity(0.35)));
                        // Edges: grab to zoom
                        for x in [x0, x1 - 2.0] {
                            window.paint_quad(fill(Bounds::new(point(b.origin.x + px(x), b.origin.y), size(px(2.), b.size.height)), accent));
                        }
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
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .child(controls)
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_xs().text_color(muted).child(window_text))
                    .children(status.map(|(err, text)| div().text_xs().text_color(if err { danger } else { muted }).child(text)))
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(muted).child("Drag or swipe: time · wheel: channels · Ctrl+wheel: zoom")),
            )
            .child(main)
            .child(axis)
            .child(overview)
            .child(Muted::new(legend))
            .into_any_element()
    }
}
