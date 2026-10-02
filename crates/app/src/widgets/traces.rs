//! Signal drawing. Each lane is one filled band from its per-column min / max (upper edge left to
//! right, lower edge back), at least 1 px tall: one path per lane instead of a shape per column.
//! All lanes share one scale (a scale bar shows it) unless each is fitted to its own peak.
//! Event markers are vertical lines. The probe map draws electrode sites with the visible
//! channels highlighted.

use gpui_kit::{canvas, fill, point, px, size, App, Bounds, Hsla, IntoElement, PathBuilder, Pixels, RenderOnce, Styled as _, Window};

use crate::services::sampler::Lanes;

#[derive(IntoElement)]
pub struct Traces {
    lanes: Option<Lanes>,
    gain: f32,
    /// One peak for every lane (shared scale); `None` fits each lane to its own peak.
    peak: Option<f32>,
    color: Hsla,
    /// Event onsets, as positions 0..1 across the width.
    markers: Vec<f32>,
    marker_color: Hsla,
    /// Scale bar height in data units, drawn at the right of the first lane.
    bar: Option<f32>,
}

impl Traces {
    pub fn new(lanes: Option<Lanes>, gain: f32, color: Hsla) -> Self {
        Self { lanes, gain, peak: None, color, markers: Vec::new(), marker_color: color, bar: None }
    }

    pub fn shared_peak(mut self, peak: Option<f32>) -> Self {
        self.peak = peak;
        self
    }

    pub fn markers(mut self, markers: Vec<f32>, color: Hsla) -> Self {
        self.markers = markers;
        self.marker_color = color;
        self
    }

    pub fn scale_bar(mut self, bar: Option<f32>) -> Self {
        self.bar = bar;
        self
    }
}

/// Pixels per data unit for a lane of height `h`: the peak reaches 45 % of the height.
fn scale(peak: f32, h: f32, gain: f32) -> f32 {
    if peak > 0.0 { 0.45 * h * gain / peak } else { 0.0 }
}

/// Points of one lane's band, in pixels: upper edge then lower edge reversed. `peak` is the
/// shared peak, or `None` for the lane's own. `None` when the lane has no finite values.
pub fn band(lane: &[(f32, f32)], bounds: (f32, f32, f32, f32), gain: f32, peak: Option<f32>) -> Option<Vec<(f32, f32)>> {
    let (x0, top, w, h) = bounds;
    let finite: Vec<(usize, f32, f32)> = lane.iter().enumerate().filter(|(_, (a, b))| a.is_finite() && b.is_finite()).map(|(i, &(a, b))| (i, a, b)).collect();
    if finite.is_empty() || w <= 0.0 {
        return None;
    }
    let own = || finite.iter().map(|&(_, a, b)| a.abs().max(b.abs())).fold(0.0f32, f32::max);
    let k = scale(peak.unwrap_or_else(own), h, gain);
    let mid = top + h / 2.0;
    let x = |i: usize| x0 + w * (i as f32 + 0.5) / lane.len().max(1) as f32;
    let clamp = |y: f32| y.clamp(top, top + h);
    let mut points: Vec<(f32, f32)> = finite.iter().map(|&(i, _, hi)| (x(i), clamp(mid - hi * k - 0.5))).collect();
    points.extend(finite.iter().rev().map(|&(i, lo, _)| (x(i), clamp(mid - lo * k + 0.5))));
    Some(points)
}

impl RenderOnce for Traces {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self { lanes, gain, peak, color, markers, marker_color, bar } = self;
        canvas(
            |_, _, _| (),
            move |bounds: Bounds<Pixels>, _, window, _| {
                let (x0, y0) = (bounds.origin.x.as_f32(), bounds.origin.y.as_f32());
                let (w, h) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
                for m in &markers {
                    let x = x0 + m * w;
                    window.paint_quad(fill(Bounds::new(point(px(x), px(y0)), size(px(1.), px(h))), marker_color));
                }
                let Some(lanes) = lanes else { return };
                if lanes.is_empty() {
                    return;
                }
                let lane_h = h / lanes.len() as f32;
                for (li, lane) in lanes.iter().enumerate() {
                    let Some(points) = band(lane, (x0, y0 + lane_h * li as f32, w, lane_h), gain, peak) else { continue };
                    let pts: Vec<_> = points.iter().map(|&(x, y)| point(px(x), px(y))).collect();
                    let mut path = PathBuilder::fill();
                    path.add_polygon(&pts, true);
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                }
                if let (Some(bar), Some(peak)) = (bar, peak) {
                    let len = bar * scale(peak, lane_h, gain);
                    let mid = y0 + lane_h / 2.0;
                    window.paint_quad(fill(Bounds::new(point(px(x0 + w - 3.), px(mid - len / 2.)), size(px(3.), px(len))), color));
                }
            },
        )
        .size_full()
    }
}

/// Width of the [`ProbeMap`].
pub const PROBE_MAP_WIDTH: f32 = 90.;

#[derive(IntoElement)]
pub struct ProbeMap {
    /// Site positions (µm) and whether the channel is visible.
    sites: Vec<([f32; 2], bool)>,
    muted: Hsla,
    accent: Hsla,
}

impl ProbeMap {
    pub fn new(sites: Vec<([f32; 2], bool)>, muted: Hsla, accent: Hsla) -> Self {
        Self { sites, muted, accent }
    }
}

impl RenderOnce for ProbeMap {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self { sites, muted, accent } = self;
        canvas(
            |_, _, _| (),
            move |bounds: Bounds<Pixels>, _, window, _| {
                if sites.is_empty() {
                    return;
                }
                let (min_x, max_x) = sites.iter().fold((f32::MAX, f32::MIN), |(a, b), (p, _)| (a.min(p[0]), b.max(p[0])));
                let (min_y, max_y) = sites.iter().fold((f32::MAX, f32::MIN), |(a, b), (p, _)| (a.min(p[1]), b.max(p[1])));
                let (w, h) = (bounds.size.width.as_f32() - 8.0, bounds.size.height.as_f32() - 8.0);
                let scale = (w / (max_x - min_x).max(1.0)).min(h / (max_y - min_y).max(1.0));
                for (p, visible) in &sites {
                    // Tip at the bottom: depth grows upward
                    let x = bounds.origin.x.as_f32() + 4.0 + (p[0] - min_x) * scale;
                    let y = bounds.origin.y.as_f32() + 4.0 + h - (p[1] - min_y) * scale;
                    let s = if *visible { 4.0 } else { 2.0 };
                    window.paint_quad(fill(Bounds::new(point(px(x - s / 2.0), px(y - s / 2.0)), size(px(s), px(s))), if *visible { accent } else { muted }));
                }
            },
        )
        .w(px(PROBE_MAP_WIDTH))
        .h_full()
    }
}

#[cfg(test)]
mod tests {
    use super::band;

    #[test]
    fn test_band_geometry() {
        // 4 columns, peak 2 → scale 0.45 * 100 / 2 = 22.5 px per unit; lane top 0, height 100
        let lane = [(-2.0, 2.0), (0.0, 0.0), (f32::NAN, f32::NAN), (-1.0, 1.0)];
        let pts = band(&lane, (0.0, 0.0, 400.0, 100.0), 1.0, None).unwrap();
        assert_eq!(pts.len(), 6, "3 finite columns, upper + lower");
        assert_eq!(pts[0], (50.0, 50.0 - 45.0 - 0.5));
        // A flat column still gets a 1 px band
        let (upper, lower) = (pts[1].1, pts[4].1);
        assert_eq!((pts[1].0, lower - upper), (150.0, 1.0));
        assert!(band(&[(f32::NAN, f32::NAN)], (0.0, 0.0, 10.0, 10.0), 1.0, None).is_none());
        // Shared peak 4: half the size of the lane's own scale
        let shared = band(&lane, (0.0, 0.0, 400.0, 100.0), 1.0, Some(4.0)).unwrap();
        assert_eq!(shared[0], (50.0, 50.0 - 22.5 - 0.5));
    }
}
