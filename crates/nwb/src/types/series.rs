//! `ElectricalSeries` / `TimeSeries` in `/acquisition`.
//!
//! Continuous data is streamed: worker threads each take a chunk of rows, read it from the
//! source recording (channel-major), transpose it to NWB's `[time, channel]` layout and write
//! it. Memory stays at a few chunks regardless of recording length.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::json;

use super::{attrs, typed_with};
use crate::backend::{Attrs, Backend};
use crate::mapping::{EventPlan, SeriesPlan};
use crate::types::electrodes::TABLE_PATH;
use nc_core::{Error, EventSeries, Recording, RecordingInfo, Result, SampleType};

/// How a series' samples are stored: the dataset type and the factors that bring them to `unit`.
#[derive(Debug, Clone, PartialEq)]
pub struct Storage {
    pub ty: SampleType,
    /// `conversion` attribute of `data`.
    pub conversion: f64,
    /// Per-channel factors (`ElectricalSeries.channel_conversion`), applied on top of `conversion`.
    pub channel_conversion: Option<Vec<f32>>,
    /// Copy `read_stored` bytes (`true`) or scaled `read` values as float32 (`false`).
    pub native: bool,
}

impl Storage {
    /// Sources that serve their stored bytes keep them when the channel scaling can be expressed
    /// in NWB: integers (half the size of float32 or less) and float64 (no precision lost), with a
    /// shared gain folded into `conversion`, or per-channel gains as `channel_conversion`
    /// (electrical series only). Everything else is written as scaled float32.
    pub fn choose(info: &RecordingInfo, electrical: bool, conversion: f64, serves_stored: bool) -> Self {
        let float32 = Storage { ty: SampleType::F32, conversion, channel_conversion: None, native: false };
        if info.stored_as == SampleType::F32 || !serves_stored || info.channels.is_empty() || info.channels.iter().any(|c| c.offset != 0.0) {
            return float32;
        }
        let g = info.channels[0].gain;
        if info.channels.iter().all(|c| c.gain == g) {
            return Storage { ty: info.stored_as, conversion: conversion * g, channel_conversion: None, native: true };
        }
        if electrical {
            let gains = info.channels.iter().map(|c| c.gain as f32).collect();
            return Storage { ty: info.stored_as, conversion, channel_conversion: Some(gains), native: true };
        }
        float32
    }
}

/// Writes a continuous series; `done` counts samples written (all channels) for progress, and
/// `cancel` (when set) stops the copy between chunks.
pub fn write_continuous(
    b: &dyn Backend,
    plan: &SeriesPlan,
    rec: &dyn Recording,
    chunks: crate::ChunkPolicy,
    threads: usize,
    done: &AtomicU64,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    let info = rec.info();
    let path = format!("/acquisition/{}", plan.name);
    let electrical = plan.electrodes.is_some();
    let neurodata_type = if electrical { "ElectricalSeries" } else { "TimeSeries" };
    b.group(&path, typed_with("core", neurodata_type, &[("description", json!(plan.description)), ("comments", json!("no comments"))]))?;

    let (rows, cols) = (info.samples, info.channel_count() as u64);
    let storage = Storage::choose(info, electrical, plan.conversion, rec.read_stored(&[], 0..0, &mut []).unwrap_or(false));
    let data_attrs = attrs(&[
        ("unit", json!(plan.unit)),
        ("conversion", json!(storage.conversion)),
        ("offset", json!(0.0)),
        ("resolution", json!(-1.0)),
    ]);
    let (shape, dims): (Vec<u64>, Vec<&str>) = if electrical {
        (vec![rows, cols], vec!["num_times", "num_channels"])
    } else if cols == 1 {
        (vec![rows], vec!["num_times"])
    } else {
        (vec![rows, cols], vec!["num_times", "num_DIM2"])
    };
    let (native, ty) = (storage.native, storage.ty);
    let chunk = chunks.rows(info.sample_rate, cols as usize, ty.bytes());
    let sink = b.stream(&format!("{path}/data"), &shape, chunk, ty, &dims, data_attrs)?;

    if let Some(rows) = &plan.electrodes {
        let region: Vec<i64> = rows.iter().map(|&r| r as i64).collect();
        let reference = json!({ "_REFERENCE": { "source": ".", "path": TABLE_PATH } });
        let a = typed_with("hdmf-common", "DynamicTableRegion", &[("description", json!("electrodes of this series")), ("table", reference)]);
        b.i64s(&format!("{path}/electrodes"), &region, "num_rows", a)?;
    }
    if let Some(factors) = &storage.channel_conversion {
        let a = attrs(&[("axis", json!(1))]);
        b.f32s(&format!("{path}/channel_conversion"), factors, "num_channels", a)?;
    }
    b.f64_scalar(&format!("{path}/starting_time"), info.start_time, attrs(&[("rate", json!(info.sample_rate)), ("unit", json!("seconds"))]))?;

    // Parallel chunked copy
    let chunks = rows.div_ceil(chunk);
    let next = AtomicU64::new(0);
    let failed: Mutex<Option<Error>> = Mutex::new(None);
    let channels: Vec<usize> = (0..cols as usize).collect();
    let es = ty.bytes();
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                let (mut floats, mut src, mut dst) = (Vec::<f32>::new(), Vec::<u8>::new(), Vec::<u8>::new());
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    if k >= chunks || failed.lock().unwrap().is_some() {
                        break;
                    }
                    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                        *failed.lock().unwrap() = Some(Error::Cancelled);
                        break;
                    }
                    let (s0, s1) = (k * chunk, ((k + 1) * chunk).min(rows));
                    let n = (s1 - s0) as usize;
                    let c = channels.len();
                    src.resize(n * c * es, 0);
                    let res = if native {
                        rec.read_stored(&channels, s0..s1, &mut src).map(|_| ())
                    } else {
                        floats.resize(n * c, 0.0);
                        rec.read(&channels, s0..s1, &mut floats).map(|()| {
                            for (d, v) in src.chunks_exact_mut(4).zip(&floats) {
                                d.copy_from_slice(&v.to_le_bytes());
                            }
                        })
                    };
                    let res = res.and_then(|()| {
                        let out: &[u8] = if c == 1 {
                            &src
                        } else {
                            // channel-major → time-major, element by element
                            dst.resize(src.len(), 0);
                            for ch in 0..c {
                                for t in 0..n {
                                    let (from, to) = ((ch * n + t) * es, (t * c + ch) * es);
                                    dst[to..to + es].copy_from_slice(&src[from..from + es]);
                                }
                            }
                            &dst
                        };
                        sink.write_rows(s0, s1 - s0, out)
                    });
                    match res {
                        Ok(()) => {
                            done.fetch_add((s1 - s0) * cols, Ordering::Relaxed);
                        }
                        Err(e) => {
                            *failed.lock().unwrap() = Some(e);
                            break;
                        }
                    }
                }
            });
        }
    });
    match failed.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// `(start, rate)` when `times` are evenly spaced (within 0.01 % of the mean interval).
pub fn regular_rate(times: &[f64]) -> Option<(f64, f64)> {
    if times.len() < 3 {
        return None;
    }
    let mean = (times[times.len() - 1] - times[0]) / (times.len() - 1) as f64;
    if mean <= 0.0 {
        return None;
    }
    let regular = times.windows(2).all(|w| ((w[1] - w[0]) - mean).abs() <= mean * 1e-4);
    regular.then(|| (times[0], 1.0 / mean))
}

/// Multi-channel scalars: a `TimeSeries` (`[event, channel]`) with explicit timestamps, or
/// `starting_time` + `rate` when the events are evenly spaced.
pub fn write_events(b: &dyn Backend, plan: &EventPlan, e: &EventSeries) -> Result<()> {
    let path = format!("/acquisition/{}", plan.name);
    b.group(&path, typed_with("core", "TimeSeries", &[("description", json!(plan.description)), ("comments", json!("no comments"))]))?;
    let data_attrs = attrs(&[("unit", json!("a.u.")), ("conversion", json!(1.0)), ("offset", json!(0.0)), ("resolution", json!(-1.0))]);
    let n = e.len() as u64;
    if e.channels > 1 {
        b.f64s(&format!("{path}/data"), &e.values, &[n, e.channels as u64], &["num_times", "num_DIM2"], data_attrs)?;
    } else {
        b.f64s(&format!("{path}/data"), &e.values, &[n], &["num_times"], data_attrs)?;
    }
    if let Some((start, rate)) = regular_rate(&e.onsets) {
        return b.f64_scalar(&format!("{path}/starting_time"), start, attrs(&[("rate", json!(rate)), ("unit", json!("seconds"))]));
    }
    let ts_attrs: Attrs = attrs(&[("interval", json!(1)), ("unit", json!("seconds"))]);
    b.f64s(&format!("{path}/timestamps"), &e.onsets, &[n], &["num_times"], ts_attrs)
}

#[cfg(test)]
mod tests {
    use super::{regular_rate, Storage};
    use nc_core::{ChannelInfo, MemoryRecording, Recording, SampleType};

    #[test]
    fn test_storage_choice() {
        let mut info = MemoryRecording::new("r", vec![0.0; 4], 2, 1.0, "V").unwrap().info().clone();
        info.stored_as = SampleType::I16;
        info.channels = vec![ChannelInfo { gain: 2.0, ..ChannelInfo::unity("a") }, ChannelInfo { gain: 2.0, ..ChannelInfo::unity("b") }];
        // Shared gain: int16 kept, gain folded into conversion
        let s = Storage::choose(&info, false, 0.5, true);
        assert_eq!((s.ty, s.conversion, s.native, s.channel_conversion), (SampleType::I16, 1.0, true, None));
        // Per-channel gains: channel_conversion on electrical series, float32 otherwise
        info.channels[1].gain = 4.0;
        let e = Storage::choose(&info, true, 1.0, true);
        assert_eq!((e.ty, e.conversion, e.channel_conversion), (SampleType::I16, 1.0, Some(vec![2.0, 4.0])));
        assert_eq!(Storage::choose(&info, false, 1.0, true).ty, SampleType::F32);
        // No stored bytes, or an offset: float32
        assert!(!Storage::choose(&info, true, 1.0, false).native);
        info.channels[0].offset = 1.0;
        assert!(!Storage::choose(&info, true, 1.0, true).native);
    }

    #[test]
    fn test_regular_rate() {
        let ticks: Vec<f64> = (0..10).map(|i| 4.096e-5 + i as f64 * 1.000_002_56).collect();
        let (start, rate) = regular_rate(&ticks).unwrap();
        assert!((start - 4.096e-5).abs() < 1e-12 && (rate - 1.0 / 1.000_002_56).abs() < 1e-9);
        assert!(regular_rate(&[0.0, 1.0, 3.0]).is_none());
        assert!(regular_rate(&[0.0, 1.0]).is_none());
    }
}
