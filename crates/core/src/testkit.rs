//! Conformance checks for readers (feature `testkit`). A reader crate runs [`check_reader`] on
//! each of its fixtures:
//!
//! ```ignore
//! let session = nc_core::testkit::check_reader(&MyReader, &fixture, &OpenOptions::default());
//! ```
//!
//! It panics with a description of the first broken rule: the reader must claim its own files,
//! produce a session that passes [`Session::validate`], and serve consistent chunked reads.

use std::ops::Range;
use std::path::Path;

use nc_base::codec::decode_into;

use crate::issue::Level;
use crate::options::OpenOptions;
use crate::reader::Reader;
use crate::recording::Recording;
use crate::session::Session;
use crate::Error;

/// Samples per window read by [`check_recording`].
const WINDOW: u64 = 4096;

/// Detects and opens `path` with `reader`, then checks the session and every recording.
pub fn check_reader(reader: &dyn Reader, path: &Path, options: &OpenOptions) -> Session {
    let name = reader.name();
    let d = reader.detect(path).unwrap_or_else(|| panic!("{name}: does not detect its own fixture {}", path.display()));
    assert_eq!(d.format, name, "{name}: detection names another format");
    assert!(d.confidence > 0.0 && d.confidence <= 1.0, "{name}: confidence {} outside (0, 1]", d.confidence);

    let s = reader.open(path, options).unwrap_or_else(|e| panic!("{name}: cannot open {}: {e}", path.display()));
    assert_eq!(s.provenance.format, name, "{name}: provenance.format must be the reader name");
    let errors: Vec<String> = s.validate().into_iter().filter(|i| i.level == Level::Error).map(|i| i.message).collect();
    assert!(errors.is_empty(), "{name}: session breaks the model's invariants:\n  {}", errors.join("\n  "));
    for r in &s.recordings {
        check_recording(r.as_ref());
    }
    s
}

/// Reads windows at the start, middle and end of `rec` and checks that:
/// split reads equal one read, channel order is honored, `read_stored` (when supported) decodes
/// to the same values, and out-of-range requests fail with the right error.
pub fn check_recording(rec: &dyn Recording) {
    let info = rec.info();
    let (name, n, channels) = (&info.name, info.samples, info.channel_count());
    let all: Vec<usize> = (0..channels).collect();
    let win = n.min(WINDOW);
    let starts = [0, (n / 2).saturating_sub(win / 2), n - win];

    for s0 in starts {
        let range = s0..s0 + win;
        let whole = read(rec, &all, range.clone());
        // The same window in two reads, split off the packet / file grid
        let k = s0 + win / 3;
        let (a, b) = (read(rec, &all, s0..k), read(rec, &all, k..s0 + win));
        for c in 0..channels {
            let (w, first) = (&whole[c * win as usize..(c + 1) * win as usize], (k - s0) as usize);
            assert!(
                same(&w[..first], &a[c * first..(c + 1) * first]) && same(&w[first..], &b[c * (win as usize - first)..(c + 1) * (win as usize - first)]),
                "{name}: split read of {range:?} differs on channel {c}"
            );
        }
        // Channels in reverse order come back in that order
        let rev: Vec<usize> = all.iter().rev().copied().collect();
        let reversed = read(rec, &rev, range.clone());
        for (i, &c) in rev.iter().enumerate() {
            let w = win as usize;
            assert!(same(&reversed[i * w..(i + 1) * w], &whole[c * w..(c + 1) * w]), "{name}: channel order not honored ({range:?})");
        }
        check_stored(rec, &all, range, &whole);
    }

    // Out-of-range requests
    let mut one = vec![0.0f32; 1];
    assert!(matches!(rec.read(&[channels], 0..1.min(n), &mut one[..1.min(n) as usize]), Err(Error::Channel { .. })), "{name}: channel {channels} must be rejected");
    assert!(matches!(rec.read(&[0], n..n + 1, &mut one), Err(Error::SampleRange { .. })), "{name}: samples past the end must be rejected");
    if n >= 2 {
        assert!(matches!(rec.read(&[0], 0..2, &mut one), Err(Error::BufferSize { .. })), "{name}: a short buffer must be rejected");
    }
}

/// When the source serves stored bytes, decoding them with the channel scaling must give `read`.
fn check_stored(rec: &dyn Recording, channels: &[usize], range: Range<u64>, read_values: &[f32]) {
    let info = rec.info();
    let bps = info.stored_as.bytes();
    let n = (range.end - range.start) as usize;
    let mut raw = vec![0u8; channels.len() * n * bps];
    match rec.read_stored(channels, range.clone(), &mut raw) {
        Ok(false) => {}
        Ok(true) => {
            let mut decoded = vec![0.0f32; n];
            for (i, &c) in channels.iter().enumerate() {
                let ch = &info.channels[c];
                decode_into(info.stored_as, &raw[i * n * bps..(i + 1) * n * bps], &mut decoded, ch.gain, ch.offset);
                assert!(same(&decoded, &read_values[i * n..(i + 1) * n]), "{}: read_stored and read disagree on channel {c} ({range:?})", info.name);
            }
        }
        Err(e) => panic!("{}: read_stored failed on {range:?}: {e}", info.name),
    }
}

fn read(rec: &dyn Recording, channels: &[usize], range: Range<u64>) -> Vec<f32> {
    let mut out = vec![0.0f32; channels.len() * (range.end - range.start) as usize];
    rec.read(channels, range.clone(), &mut out).unwrap_or_else(|e| panic!("{}: read {range:?} failed: {e}", rec.info().name));
    out
}

/// Equal values, NaN equal to NaN.
fn same(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y || (x.is_nan() && y.is_nan()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{Detection, MemoryRecording};

    struct Fixture;

    impl Reader for Fixture {
        fn name(&self) -> &'static str {
            "fixture"
        }
        fn version(&self) -> &'static str {
            "0.0.0"
        }
        fn description(&self) -> &'static str {
            "in-memory test reader"
        }
        fn versions(&self) -> &'static [&'static str] {
            &[]
        }
        fn detect(&self, _: &Path) -> Option<Detection> {
            Some(Detection { format: "fixture", version: None, confidence: 1.0 })
        }
        fn open(&self, _: &Path, _: &OpenOptions) -> crate::Result<Session> {
            let mut s = Session::default();
            let data: Vec<f32> = (0..3 * 10_000).map(|v| v as f32).collect();
            s.recordings.push(Arc::new(MemoryRecording::new("R", data, 3, 1000.0, "V").unwrap()));
            s.provenance.format = "fixture".into();
            Ok(s)
        }
    }

    #[test]
    fn test_memory_reader_conforms() {
        let s = check_reader(&Fixture, Path::new("."), &OpenOptions::default());
        assert_eq!(s.recordings.len(), 1);
    }
}
