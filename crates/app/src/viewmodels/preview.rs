//! The preview: which stream, which channels (all, or an electrode group, scrolled N lanes at a
//! time), which time window, gain, and event markers. Data comes from the sampler thread (newest
//! request wins, recent results cached); nothing is fetched while drawing.
//!
//! Smooth navigation: the sampler is asked for three windows around the view (aligned to a grid
//! of window lengths), so panning and dragging only re-slice what is already there
//! ([`resample`]); a new request goes out when the view leaves the fetched range or the zoom
//! changes. Listens to: PreviewRequested, RecordingOpened.

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
/// Lanes shown at once by default.
pub const LANES: usize = 16;
/// Lane counts offered.
pub const LANE_CHOICES: [usize; 5] = [4, 8, 16, 32, 64];
/// Recent results kept.
const CACHE: usize = 16;
/// Time shown when a stream opens (less for shorter recordings).
pub const SPAN: f64 = 1.0;
/// Shortest time shown.
const MIN_SPAN: f64 = 0.002;

/// The window a stream opens with: [`SPAN`], or the whole recording when it is shorter.
pub fn initial_span(duration: f64) -> f64 {
    SPAN.min(duration).max(MIN_SPAN)
}

/// `0.000–1.000 s of 12.50 s`; milliseconds for recordings under a second (`0.0–30.0 ms of 30.0 ms`).
pub fn window_label(start: f64, span: f64, total: f64) -> String {
    if total < 1.0 {
        format!("{:.1}–{:.1} ms of {:.1} ms", start * 1e3, (start + span) * 1e3, total * 1e3)
    } else {
        format!("{:.3}–{:.3} s of {:.2} s", start, start + span, total)
    }
}

/// A choice in the channel dropdown.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelSet {
    pub label: String,
    pub channels: Vec<usize>,
}

/// All channels, then one set per electrode group when the channels belong to more than one.
pub fn channel_sets(s: &Session, recording: &str, count: usize) -> Vec<ChannelSet> {
    let mut sets = vec![ChannelSet { label: format!("All channels ({count})"), channels: (0..count).collect() }];
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

/// The time range to fetch for a view: three window lengths around it, aligned to a grid of
/// window lengths (so small pans stay inside it).
pub fn fetch_range(start: f64, span: f64) -> (f64, f64) {
    let cell = (start / span).floor();
    (((cell - 1.0) * span).max(0.0), 3.0 * span)
}

/// The visible `columns` of `channels` for `[start, start + span)` cut out of fetched lanes
/// (`key` says what they cover; channels or times not covered are NaN gaps). Works for any
/// fetched resolution, so a zoom shows coarser data until the new data arrives.
pub fn resample(key: &Key, lanes: &[Vec<(f32, f32)>], channels: &[usize], rate: f64, start: f64, span: f64, columns: usize) -> Vec<Vec<(f32, f32)>> {
    let gap = (f32::NAN, f32::NAN);
    let len = (key.samples.end - key.samples.start) as f64;
    channels
        .iter()
        .map(|c| {
            let Some(lane) = key.channels.iter().position(|x| x == c).and_then(|i| lanes.get(i)) else { return vec![gap; columns] };
            let n = lane.len() as f64;
            if n == 0.0 || len <= 0.0 {
                return vec![gap; columns];
            }
            (0..columns)
                .map(|i| {
                    let t0 = start + i as f64 * span / columns as f64;
                    let s0 = t0 * rate - key.samples.start as f64;
                    let s1 = s0 + span / columns as f64 * rate;
                    if s1 <= 0.0 || s0 >= len {
                        return gap;
                    }
                    let c0 = ((s0 / len * n).floor().max(0.0)) as usize;
                    let c1 = ((s1 / len * n).ceil().min(n) as usize).max(c0 + 1).min(lane.len());
                    lane[c0..c1].iter().filter(|(a, b)| a.is_finite() && b.is_finite()).fold(gap, |(lo, hi), &(a, b)| (if lo.is_nan() { a } else { lo.min(a) }, if hi.is_nan() { b } else { hi.max(b) }))
                })
                .collect()
        })
        .collect()
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
    /// First shown position in the set's channel list, and how many lanes are shown.
    pub first: usize,
    pub lanes: usize,
    pub gain: Entity<SliderState>,
    /// Scale each channel to its own peak instead of one scale for all.
    pub fit_each: bool,
    /// Event series drawn as markers.
    pub marker: Option<String>,
    /// The latest fetched data (it may cover only part of the view while sampling).
    pub shown: Option<(Key, Lanes)>,
    /// The last key asked for (a pan inside its range asks nothing).
    requested: Option<Key>,
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
                        this.span = initial_span(rec.as_ref().map_or(SPAN, |r| r.info().duration()));
                        this.recording = rec;
                        this.start = 0.0;
                        this.set = 0;
                        this.first = 0;
                        this.shown = None;
                        this.requested = None;
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
            span: SPAN,
            sets: Vec::new(),
            set: 0,
            first: 0,
            lanes: LANES,
            gain,
            fit_each: false,
            marker: None,
            shown: None,
            requested: None,
            waiting: false,
            error: None,
            cache: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    /// Channels of the selected set.
    pub fn set_channels(&self) -> &[usize] {
        self.sets.get(self.set).map_or(&[], |s| s.channels.as_slice())
    }

    /// The channels on screen, top to bottom.
    pub fn visible_channels(&self) -> Vec<usize> {
        self.set_channels().iter().skip(self.first).take(self.lanes).copied().collect()
    }

    /// What to fetch for the current view.
    pub fn key(&self) -> Option<Key> {
        let i = self.recording.as_ref()?.info();
        let (from, len) = fetch_range(self.start, self.span);
        Some(Key { recording: i.name.clone(), samples: window(i.samples, i.sample_rate, from, len), channels: self.visible_channels(), columns: 3 * COLUMNS })
    }

    /// The visible lanes cut out of the fetched data.
    pub fn visible(&self) -> Option<Lanes> {
        let (key, lanes) = self.shown.as_ref()?;
        let rate = self.recording.as_ref()?.info().sample_rate;
        Some(Arc::new(resample(key, lanes, &self.visible_channels(), rate, self.start, self.span, COLUMNS)))
    }

    /// Shows the wanted view from the cache, or asks the sampler for it (once per key).
    fn request(&mut self, cx: &mut Context<Self>) {
        let (Some(rec), Some(key)) = (self.recording.clone(), self.key()) else {
            cx.notify();
            return;
        };
        if self.shown.as_ref().is_some_and(|(k, _)| *k == key) {
            self.waiting = false;
        } else if let Some((_, lanes)) = self.cache.iter().find(|(k, _)| *k == key) {
            self.shown = Some((key.clone(), lanes.clone()));
            self.waiting = false;
        } else if self.requested.as_ref() != Some(&key) {
            self.waiting = true;
            self.sampler.request(key.clone(), rec, self.reply.clone());
        }
        self.requested = Some(key);
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
                    self.waiting = false;
                    self.error = None;
                }
                // Even a superseded answer is closer than nothing (it is re-sliced to the view)
                if self.shown.as_ref().is_none_or(|(k, _)| Some(k) != self.key().as_ref()) {
                    self.shown = Some((key, lanes));
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
            self.first = 0;
            self.request(cx);
        }
    }

    /// Scrolls the lanes by `delta` channels (clamped).
    pub fn scroll_channels(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.set_first(self.first as isize + delta, cx);
    }

    /// Shows channels from position `first` of the set (clamped).
    pub fn set_first(&mut self, first: isize, cx: &mut Context<Self>) {
        let max = self.set_channels().len().saturating_sub(self.lanes);
        let first = first.clamp(0, max as isize) as usize;
        if first != self.first {
            self.first = first;
            self.request(cx);
        }
    }

    pub fn set_lanes(&mut self, lanes: usize, cx: &mut Context<Self>) {
        self.lanes = lanes.max(1);
        self.set_first(self.first as isize, cx);
        self.request(cx);
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
        let (Some(rec), visible) = (&self.recording, self.visible_channels()) else { return Vec::new() };
        let Some(s) = self.store.read(cx).ws.session() else { return Vec::new() };
        s.channel_electrodes(&rec.info().name)
            .iter()
            .enumerate()
            .filter_map(|(c, e)| Some(((*e).and_then(|e| s.electrodes[e].position_um).map(|p| [p[0], p[1]])?, visible.contains(&c))))
            .collect()
    }

    pub fn gain_value(&self, cx: &App) -> f32 {
        self.gain.read(cx).value().start()
    }

    pub fn set_fit_each(&mut self, on: bool, cx: &mut Context<Self>) {
        self.fit_each = on;
        cx.notify();
    }

    /// Shows `[start, start + span)` (clamped to the recording).
    pub fn set_view(&mut self, start: f64, span: f64, cx: &mut Context<Self>) {
        let d = self.duration().max(MIN_SPAN);
        self.span = span.clamp(MIN_SPAN, d);
        self.start = start.clamp(0.0, (d - self.span).max(0.0));
        self.request(cx);
    }

    /// Moves by `fraction` of the visible span (negative: earlier).
    pub fn pan(&mut self, fraction: f64, cx: &mut Context<Self>) {
        self.set_view(self.start + fraction * self.span, self.span, cx);
    }

    /// Zooms by `factor` (> 1: more time) keeping the time under `anchor` (0..1 across the
    /// view) in place.
    pub fn zoom_at(&mut self, factor: f64, anchor: f64, cx: &mut Context<Self>) {
        let at = self.start + anchor.clamp(0.0, 1.0) * self.span;
        let span = (self.span * factor).clamp(MIN_SPAN, self.duration().max(MIN_SPAN));
        self.set_view(at - anchor.clamp(0.0, 1.0) * span, span, cx);
    }

    pub fn zoom(&mut self, factor: f64, cx: &mut Context<Self>) {
        self.zoom_at(factor, 0.5, cx);
    }

    /// Time axis ticks: (position 0..1, label).
    pub fn ticks(&self) -> Vec<(f32, String)> {
        let step_label = |t: f64| if self.span < 0.05 { format!("{:.0} ms", t * 1e3) } else { format!("{} s", trim(t)) };
        ticks(self.start, self.start + self.span, 5).into_iter().map(|t| (((t - self.start) / self.span) as f32, step_label(t))).collect()
    }

    /// (peak used for every lane, scale bar value and its label) for the shared scale.
    pub fn scale(&self, cx: &App) -> Option<(f32, f32, String)> {
        let lanes = self.visible()?;
        let p = peak(&lanes);
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
        assert_eq!(sets.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["All channels (40)"]);
        assert_eq!(sets[0].channels, (0..40).collect::<Vec<_>>());

        assert_eq!(markers(&[0.1, 0.5, 1.2, 1.4, 3.0], 1.0, 1.0), vec![0.2, 0.4]);

        // Short recordings open whole; long ones with one second
        assert_eq!(initial_span(0.03), 0.03);
        assert_eq!(initial_span(10.0), 1.0);
        assert_eq!(initial_span(0.0), MIN_SPAN);
        assert_eq!(window_label(0.0, 0.03, 0.03), "0.0–30.0 ms of 30.0 ms");
        assert_eq!(window_label(2.0, 1.0, 12.5), "2.000–3.000 s of 12.50 s");
        assert_eq!(peak(&[vec![(-2.0, 1.0)], vec![(f32::NAN, f32::NAN), (0.0, 3.0)]]), 3.0);
    }

    #[test]
    fn test_fetch_range_and_resample() {
        // Three windows around the view, on a grid of window lengths
        assert_eq!(fetch_range(5.3, 1.0), (4.0, 3.0));
        assert_eq!(fetch_range(5.9, 1.0), (4.0, 3.0), "small pans stay in the fetched range");
        assert_eq!(fetch_range(0.2, 1.0), (0.0, 3.0));

        // Fetched: channels 3 and 5, samples 0..8 in 4 columns (2 samples each), 1 Hz
        let key = Key { recording: "r".into(), samples: 0..8, channels: vec![3, 5], columns: 4 };
        let lanes = vec![vec![(0.0, 1.0), (1.0, 2.0), (2.0, 3.0), (3.0, 4.0)], vec![(-1.0, 0.0); 4]];
        // Seconds 2..6 in 2 columns: column 0 = samples 2..4 = fetched column 1, column 1 = column 2
        let out = resample(&key, &lanes, &[3, 9], 1.0, 2.0, 4.0, 2);
        assert_eq!(out[0], vec![(1.0, 2.0), (2.0, 3.0)]);
        // Seconds 1..7 in 2 columns: each spans parts of two fetched columns (min / max of both)
        assert_eq!(resample(&key, &lanes, &[3], 1.0, 1.0, 6.0, 2)[0], vec![(0.0, 2.0), (2.0, 4.0)]);
        assert!(out[1].iter().all(|(a, b)| a.is_nan() && b.is_nan()), "channel 9 not fetched: gap");
        // Outside the fetched time: gaps
        let out = resample(&key, &lanes, &[3], 1.0, 10.0, 2.0, 2);
        assert!(out[0][0].0.is_nan());
    }
}
