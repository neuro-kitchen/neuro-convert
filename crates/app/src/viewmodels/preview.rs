//! The preview: which stream, which channels (pages of 16 or an electrode group), which time
//! window, gain, and event markers. Data comes from the sampler thread (newest request wins,
//! recent results cached); nothing is fetched while drawing. Listens to: PreviewRequested,
//! RecordingOpened.

use std::sync::Arc;

use gpui_kit::component::slider::{SliderEvent, SliderScale, SliderState};
use gpui_kit::{App, AppContext as _, Context, Entity, Subscription};
use nc_convert::core::{Recording, Session};

use crate::domain::format::{nice_below, si, ticks};
use crate::domain::AppEvent;
use crate::services::sampler::{Key, Lanes, Sampler};
use crate::store::Store;

/// Envelope columns per view (drawn stretched to the panel width).
pub const COLUMNS: usize = 1200;
/// Channels per page.
pub const PAGE: usize = 16;
/// Recent results kept.
const CACHE: usize = 16;

/// A choice in the channel dropdown.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelSet {
    pub label: String,
    pub channels: Vec<usize>,
}

/// Pages of [`PAGE`] channels (1-based labels), then one set per electrode group when the
/// channels belong to more than one group.
pub fn channel_sets(s: &Session, recording: &str, count: usize) -> Vec<ChannelSet> {
    let mut sets: Vec<ChannelSet> = (0..count)
        .step_by(PAGE)
        .map(|a| {
            let b = (a + PAGE).min(count);
            ChannelSet { label: format!("Channels {}–{}", a + 1, b), channels: (a..b).collect() }
        })
        .collect();
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (c, e) in s.channel_electrodes(recording).iter().enumerate() {
        let Some(e) = e else { continue };
        let g = &s.electrodes[*e].group;
        match groups.iter_mut().find(|(n, _)| n == g) {
            Some((_, v)) => v.push(c),
            None => groups.push((g.clone(), vec![c])),
        }
    }
    if groups.len() > 1 {
        sets.extend(groups.into_iter().map(|(g, channels)| ChannelSet { label: format!("Group {g} ({})", channels.len()), channels }));
    }
    sets
}

/// The sample range of a time window, clamped to the recording.
pub fn window(samples: u64, rate: f64, start: f64, span: f64) -> std::ops::Range<u64> {
    let rate = rate.max(1e-9);
    let s0 = ((start.max(0.0) * rate) as u64).min(samples);
    let s1 = (s0 + (span * rate).max(1.0) as u64).min(samples);
    s0..s1
}

/// Largest |value| over all lanes (shared scale).
pub fn peak(lanes: &[Vec<(f32, f32)>]) -> f32 {
    lanes.iter().flatten().filter(|(a, b)| a.is_finite() && b.is_finite()).map(|(a, b)| a.abs().max(b.abs())).fold(0.0, f32::max)
}

/// Positions (0..1) of event onsets in `[start, start + span)`.
pub fn markers(onsets: &[f64], start: f64, span: f64) -> Vec<f32> {
    let from = onsets.partition_point(|t| *t < start);
    onsets[from..].iter().take_while(|t| **t < start + span).map(|t| ((t - start) / span) as f32).take(2000).collect()
}

pub struct PreviewVm {
    store: Entity<Store>,
    sampler: Arc<Sampler>,
    reply: async_channel::Sender<(Key, nc_convert::Result<Lanes>)>,
    pub recording: Option<Arc<dyn Recording>>,
    pub start: f64,
    pub span: f64,
    pub sets: Vec<ChannelSet>,
    pub set: usize,
    pub gain: Entity<SliderState>,
    /// Scale each channel to its own peak instead of one scale for all.
    pub fit_each: bool,
    /// Event series drawn as markers.
    pub marker: Option<String>,
    /// What is drawn (it may lag the wanted view while sampling).
    pub shown: Option<(Key, Lanes)>,
    pub waiting: bool,
    pub error: Option<String>,
    cache: Vec<(Key, Lanes)>,
    _subscriptions: Vec<Subscription>,
}

impl PreviewVm {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let (reply, rx) = async_channel::unbounded();
        cx.spawn(async move |this, cx| {
            while let Ok((key, result)) = rx.recv().await {
                if this.update(cx, |vm, cx| vm.received(key, result, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let gain = cx.new(|_| SliderState::new().min(0.25).max(64.0).scale(SliderScale::Logarithmic).default_value(1.0));
        let subscriptions = vec![
            cx.subscribe(&store, |this, store, event: &AppEvent, cx| {
                if matches!(event, AppEvent::PreviewRequested | AppEvent::RecordingOpened) {
                    let ws = &store.read(cx).ws;
                    let rec = ws.preview.as_ref().and_then(|n| ws.session()?.recording(n).cloned());
                    let name = |r: &Option<Arc<dyn Recording>>| r.as_ref().map(|r| r.info().name.clone());
                    if name(&rec) != name(&this.recording) || *event == AppEvent::RecordingOpened {
                        this.sets = match (&rec, ws.session()) {
                            (Some(r), Some(s)) => channel_sets(s, &r.info().name, r.info().channel_count()),
                            _ => Vec::new(),
                        };
                        if *event == AppEvent::RecordingOpened {
                            this.marker = None;
                        }
                        this.recording = rec;
                        this.start = 0.0;
                        this.set = 0;
                        this.shown = None;
                        this.cache.clear();
                        this.request(cx);
                    }
                }
            }),
            cx.subscribe(&gain, |_, _, _: &SliderEvent, cx| cx.notify()),
        ];
        Self {
            store,
            sampler: Arc::new(Sampler::new()),
            reply,
            recording: None,
            start: 0.0,
            span: 1.0,
            sets: Vec::new(),
            set: 0,
            gain,
            fit_each: false,
            marker: None,
            shown: None,
            waiting: false,
            error: None,
            cache: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn key(&self) -> Option<Key> {
        let i = self.recording.as_ref()?.info();
        let channels = self.sets.get(self.set).map(|s| s.channels.clone()).unwrap_or_default();
        Some(Key { recording: i.name.clone(), samples: window(i.samples, i.sample_rate, self.start, self.span), channels, columns: COLUMNS })
    }

    /// Shows the wanted view from the cache, or asks the sampler for it.
    fn request(&mut self, cx: &mut Context<Self>) {
        let (Some(rec), Some(key)) = (self.recording.clone(), self.key()) else {
            cx.notify();
            return;
        };
        if let Some((_, lanes)) = self.cache.iter().find(|(k, _)| *k == key) {
            self.shown = Some((key, lanes.clone()));
            self.waiting = false;
        } else {
            self.waiting = true;
            self.sampler.request(key, rec, self.reply.clone());
        }
        cx.notify();
    }

    fn received(&mut self, key: Key, result: nc_convert::Result<Lanes>, cx: &mut Context<Self>) {
        match result {
            Ok(lanes) => {
                self.cache.retain(|(k, _)| *k != key);
                self.cache.push((key.clone(), lanes.clone()));
                if self.cache.len() > CACHE {
                    self.cache.remove(0);
                }
                if Some(&key) == self.key().as_ref() {
                    self.shown = Some((key, lanes));
                    self.waiting = false;
                    self.error = None;
                }
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.waiting = false;
            }
        }
        cx.notify();
    }

    pub fn duration(&self) -> f64 {
        self.recording.as_ref().map_or(0.0, |r| r.info().duration())
    }

    /// Included streams, for the stream dropdown.
    pub fn streams(&self, cx: &App) -> Vec<String> {
        let ws = &self.store.read(cx).ws;
        ws.session()
            .map(|s| s.recordings.iter().map(|r| r.info().name.clone()).filter(|n| ws.included(nc_convert::core::ItemKind::Stream, n)).collect())
            .unwrap_or_default()
    }

    pub fn select_stream(vm: &Entity<Self>, name: String, cx: &mut App) {
        let store = vm.read(cx).store.clone();
        store.update(cx, |s, cx| s.apply(cx, |ws| ws.request_preview(&name)));
    }

    pub fn select_set(&mut self, set: usize, cx: &mut Context<Self>) {
        if set < self.sets.len() {
            self.set = set;
            self.request(cx);
        }
    }

    /// Event series of the session, for the markers dropdown.
    pub fn event_names(&self, cx: &App) -> Vec<String> {
        self.store.read(cx).ws.session().map(|s| s.events.iter().map(|e| e.name.clone()).collect()).unwrap_or_default()
    }

    pub fn set_marker(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.marker = name;
        cx.notify();
    }

    /// Marker positions in the visible window.
    pub fn marker_positions(&self, cx: &App) -> Vec<f32> {
        let Some(name) = &self.marker else { return Vec::new() };
        let Some(e) = self.store.read(cx).ws.session().and_then(|s| s.event_series(name)) else { return Vec::new() };
        markers(&e.onsets, self.start, self.span)
    }

    /// Site positions (µm) of the stream's channels, and whether each is shown.
    pub fn sites(&self, cx: &App) -> Vec<([f32; 2], bool)> {
        let (Some(rec), Some(key)) = (&self.recording, self.key()) else { return Vec::new() };
        let Some(s) = self.store.read(cx).ws.session() else { return Vec::new() };
        s.channel_electrodes(&rec.info().name)
            .iter()
            .enumerate()
            .filter_map(|(c, e)| Some(((*e).and_then(|e| s.electrodes[e].position_um).map(|p| [p[0], p[1]])?, key.channels.contains(&c))))
            .collect()
    }

    pub fn gain_value(&self, cx: &App) -> f32 {
        self.gain.read(cx).value().start()
    }

    pub fn set_fit_each(&mut self, on: bool, cx: &mut Context<Self>) {
        self.fit_each = on;
        cx.notify();
    }

    pub fn can_pan(&self, direction: f64) -> bool {
        if direction < 0.0 { self.start > 0.0 } else { self.start + self.span < self.duration() }
    }

    /// Moves by `fraction` of the visible span (negative: earlier).
    pub fn pan(&mut self, fraction: f64, cx: &mut Context<Self>) {
        let max = (self.duration() - self.span).max(0.0);
        self.start = (self.start + fraction * self.span).clamp(0.0, max);
        self.request(cx);
    }

    /// Centers the window on `fraction` of the recording (overview bar).
    pub fn jump(&mut self, fraction: f64, cx: &mut Context<Self>) {
        let max = (self.duration() - self.span).max(0.0);
        self.start = (fraction.clamp(0.0, 1.0) * self.duration() - self.span / 2.0).clamp(0.0, max);
        self.request(cx);
    }

    pub fn zoom(&mut self, factor: f64, cx: &mut Context<Self>) {
        let center = self.start + self.span / 2.0;
        self.span = (self.span * factor).clamp(0.002, self.duration().max(0.002));
        let max = (self.duration() - self.span).max(0.0);
        self.start = (center - self.span / 2.0).clamp(0.0, max);
        self.request(cx);
    }

    /// Mouse wheel: channels; with Ctrl zoom; with Shift pan.
    pub fn wheel(&mut self, dy: f32, ctrl: bool, shift: bool, cx: &mut Context<Self>) {
        if dy == 0.0 {
            return;
        }
        let down = dy < 0.0;
        if ctrl {
            self.zoom(if down { 1.25 } else { 0.8 }, cx);
        } else if shift {
            self.pan(if down { 0.2 } else { -0.2 }, cx);
        } else {
            let next = if down { self.set + 1 } else { self.set.saturating_sub(1) };
            self.select_set(next.min(self.sets.len().saturating_sub(1)), cx);
        }
    }

    /// Time axis ticks: (position 0..1, label).
    pub fn ticks(&self) -> Vec<(f32, String)> {
        let step_label = |t: f64| if self.span < 0.05 { format!("{:.0} ms", t * 1e3) } else { format!("{} s", trim(t)) };
        ticks(self.start, self.start + self.span, 5).into_iter().map(|t| (((t - self.start) / self.span) as f32, step_label(t))).collect()
    }

    /// (peak used for every lane, scale bar value and its label) for the shared scale.
    pub fn scale(&self, cx: &App) -> Option<(f32, f32, String)> {
        let (_, lanes) = self.shown.as_ref()?;
        let p = peak(lanes);
        if self.fit_each || p <= 0.0 {
            return None;
        }
        // A lane's half height shows peak / (0.9 · gain)
        let lane_half = p / (0.9 * self.gain_value(cx));
        let bar = nice_below(f64::from(lane_half)) as f32;
        let unit = self.recording.as_ref().map(|r| r.info().unit.clone()).unwrap_or_default();
        Some((p, bar, si(f64::from(bar), &unit)))
    }
}

fn trim(t: f64) -> String {
    let s = format!("{t:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace::tests::{open, scratch, workspace};

    #[test]
    fn test_window_sets_and_markers() {
        assert_eq!(window(10_000, 1000.0, 9.5, 1.0), 9_500..10_000);
        assert_eq!(window(10_000, 1000.0, -3.0, 0.0001), 0..1);

        let mut ws = workspace(&scratch("preview-vm"));
        open(&mut ws, "session.fake");
        let sets = channel_sets(ws.session().unwrap(), "Wav1", 40);
        assert_eq!(sets.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Channels 1–16", "Channels 17–32", "Channels 33–40"]);
        assert_eq!(sets[2].channels, (32..40).collect::<Vec<_>>());

        assert_eq!(markers(&[0.1, 0.5, 1.2, 1.4, 3.0], 1.0, 1.0), vec![0.2, 0.4]);
        assert_eq!(peak(&[vec![(-2.0, 1.0)], vec![(f32::NAN, f32::NAN), (0.0, 3.0)]]), 3.0);
    }
}
