//! nc-template: a working reader for a made-up format, to copy for a real one
//! (docs/readers/README.md walks through it).
//!
//! The made-up format, `*.example`: a 32-byte header — magic `EXAMPLE1`, uint32 channel count,
//! float64 sample rate (Hz), float64 volts per bit, uint32 reserved — then int16 samples,
//! channels interleaved per sample. An optional `<name>.events.csv` next to it holds
//! `seconds,label` lines (TTL markers).
//!
//! Replace each part with your format's: detection, header parsing, the `Recording`, events,
//! electrodes, metadata. Keep the shape: memory-map the data, read only what is asked, push
//! recoverable problems to `provenance.warnings`.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nc_base::mapped::MappedFile;
use nc_core::{
    check_read, Calibration, ChannelInfo, ChannelRef, Detection, Device, Electrode, ElectrodeGroup, Error, EventSeries, MemoryOrder, OpenOptions,
    Provenance, Reader, Recording, RecordingInfo, Result, SampleType, Session, SignalKind,
};

/// This crate's version (`nc-template`, from its `Cargo.toml`): recorded in every conversion's
/// provenance and report, so a problem in a file can be traced to the code that wrote it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// [`VERSION`].
pub fn version() -> &'static str {
    VERSION
}

const MAGIC: &[u8; 8] = b"EXAMPLE1";
const HEADER: usize = 32;

pub struct Template;

/// The parsed header.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Header {
    pub channels: usize,
    pub rate: f64,
    pub volts_per_bit: f64,
}

impl Header {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < HEADER || &b[..8] != MAGIC {
            return Err(Error::format("template", "not an EXAMPLE1 file"));
        }
        let channels = u32::from_le_bytes(b[8..12].try_into().expect("4 bytes")) as usize;
        let rate = f64::from_le_bytes(b[12..20].try_into().expect("8 bytes"));
        let volts_per_bit = f64::from_le_bytes(b[20..28].try_into().expect("8 bytes"));
        if channels == 0 || rate.is_nan() || rate <= 0.0 {
            return Err(Error::format("template", format!("bad header: {channels} channels at {rate} Hz")));
        }
        Ok(Self { channels, rate, volts_per_bit })
    }
}

impl Reader for Template {
    fn name(&self) -> &'static str {
        "template"
    }

    fn version(&self) -> &'static str {
        VERSION
    }

    // `maturity()` stays the default (experimental) until the reference comparison passes; then
    // return `nc_core::Maturity::Verified`.

    fn description(&self) -> &'static str {
        "Made-up EXAMPLE1 recording (the reader template)"
    }

    fn opens(&self) -> &'static str {
        "An .example file"
    }

    fn versions(&self) -> &'static [&'static str] {
        &["EXAMPLE1: int16 interleaved channels, optional events CSV"]
    }

    /// Cheap: the header only, never the data.
    fn detect(&self, path: &Path) -> Option<Detection> {
        use std::io::Read;
        let mut b = [0u8; HEADER];
        std::fs::File::open(path).ok()?.read_exact(&mut b).ok()?;
        Header::parse(&b).ok()?;
        Some(Detection { format: "template", version: Some("EXAMPLE1".into()), confidence: 0.95 })
    }

    fn open(&self, path: &Path, options: &OpenOptions) -> Result<Session> {
        let file = Arc::new(MappedFile::open(path)?);
        let header = Header::parse(file.bytes())?;
        let mut s = Session::default();
        let mut prov = Provenance::new("template");
        prov.version = Some("EXAMPLE1".into());
        prov.add_file(path);

        let row = 2 * header.channels;
        let body = file.bytes().len() - HEADER;
        if !body.is_multiple_of(row) {
            // Recoverable: a warning, not an error
            prov.warnings.push(format!("{}: ends with a partial sample", path.display()));
        }
        let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "signal".into());

        // One stream of electrode channels, int16 kept with a gain to volts
        if options.wants(&name) {
            let channels: Vec<ChannelInfo> = (0..header.channels).map(|c| ChannelInfo { name: format!("ch{c}"), gain: header.volts_per_bit, offset: 0.0 }).collect();
            let info = RecordingInfo {
                name: name.clone(),
                description: "EXAMPLE1 channels".into(),
                channels,
                samples: (body / row) as u64,
                sample_rate: header.rate,
                start_time: 0.0,
                unit: "V".into(),
                calibration: Calibration::Known,
                kind: SignalKind::Electrical,
                stored_as: SampleType::I16,
                order: MemoryOrder::TimeMajor,
                storage: "example".into(),
                metadata: Default::default(),
            };
            // Electrodes: one per channel, in one group on one device (what the hardware tells)
            s.metadata.devices.push(Device { name: "example-amp".into(), description: "EXAMPLE1 amplifier".into(), manufacturer: None, model: None });
            s.electrode_groups.push(ElectrodeGroup { name: "array".into(), description: "EXAMPLE1 channels".into(), location: "unknown".into(), device: Some("example-amp".into()) });
            for c in 0..header.channels {
                s.electrodes.push(Electrode { name: format!("ch{c}"), group: "array".into(), channels: vec![ChannelRef { recording: name.clone(), channel: c }], ..Default::default() });
            }
            s.recordings.push(Arc::new(ExampleRecording { info, file, columns: header.channels }));
        }

        // Optional events sidecar: `seconds,label` lines
        let csv: PathBuf = path.with_extension("events.csv");
        if let Ok(text) = std::fs::read_to_string(&csv) {
            prov.add_file(&csv);
            let mut onsets = Vec::new();
            let mut labels = Vec::new();
            for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
                match line.split_once(',').and_then(|(t, l)| Some((t.trim().parse::<f64>().ok()?, l.trim().to_string()))) {
                    Some((t, l)) => {
                        onsets.push(t);
                        labels.push(l);
                    }
                    None => prov.warnings.push(format!("{}: line {} is not `seconds,label`", csv.display(), i + 1)),
                }
            }
            if !onsets.is_empty() && options.wants("markers") {
                let n = onsets.len();
                s.events.push(EventSeries { name: "markers".into(), description: "Markers from the events CSV".into(), onsets, offsets: None, values: vec![1.0; n], channels: 1, labels });
            }
        }

        s.metadata.experiment = Some(name);
        s.provenance = prov;
        Ok(s)
    }
}

/// int16 samples, channels interleaved per sample, after the header.
pub struct ExampleRecording {
    info: RecordingInfo,
    file: Arc<MappedFile>,
    columns: usize,
}

impl ExampleRecording {
    /// Calls `f(i, t, bytes)` for each requested channel `i` and sample offset `t`, row by row so
    /// the file is read in order.
    fn each(&self, channels: &[usize], samples: Range<u64>, mut f: impl FnMut(usize, usize, [u8; 2])) {
        let b = self.file.bytes();
        for (t, s) in (samples.start as usize..samples.end as usize).enumerate() {
            let base = HEADER + s * self.columns * 2;
            for (i, &c) in channels.iter().enumerate() {
                let at = base + c * 2;
                f(i, t, [b[at], b[at + 1]]);
            }
        }
    }
}

impl Recording for ExampleRecording {
    fn info(&self) -> &RecordingInfo {
        &self.info
    }

    /// float32, channel-major, scaled by each channel's gain and offset.
    fn read(&self, channels: &[usize], samples: Range<u64>, out: &mut [f32]) -> Result<()> {
        let n = check_read(&self.info, channels, &samples, out.len())?;
        let scale: Vec<(f64, f64)> = channels.iter().map(|&c| (self.info.channels[c].gain, self.info.channels[c].offset)).collect();
        self.each(channels, samples, |i, t, v| out[i * n + t] = (i16::from_le_bytes(v) as f64 * scale[i].0 + scale[i].1) as f32);
        Ok(())
    }

    /// The stored int16, little-endian, channel-major (lets NWB keep the integers).
    fn read_stored(&self, channels: &[usize], samples: Range<u64>, out: &mut [u8]) -> Result<bool> {
        let n = check_read(&self.info, channels, &samples, out.len() / 2)?;
        if out.len() != channels.len() * n * 2 {
            return Err(Error::BufferSize { expected: channels.len() * n * 2, actual: out.len() });
        }
        self.each(channels, samples, |i, t, v| {
            let at = (i * n + t) * 2;
            out[at..at + 2].copy_from_slice(&v);
        });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a fixture: `channels` × `samples`, value = channel × 100 + sample.
    fn fixture(dir: &Path, channels: u32, samples: usize) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("rec.example");
        let mut b = MAGIC.to_vec();
        b.extend(channels.to_le_bytes());
        b.extend(1000.0f64.to_le_bytes());
        b.extend(1e-6f64.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        for t in 0..samples {
            for c in 0..channels as usize {
                b.extend(((c * 100 + t) as i16).to_le_bytes());
            }
        }
        std::fs::write(&path, b).unwrap();
        std::fs::write(dir.join("rec.events.csv"), "0.5,start\n1.5,stop\n").unwrap();
        path
    }

    #[test]
    fn test_reads_the_fixture() {
        let dir = std::env::temp_dir().join(format!("nc-template-{}", std::process::id()));
        let path = fixture(&dir, 3, 2000);
        // Detection, Session::validate and read consistency at the start, middle and end
        let s = nc_core::testkit::check_reader(&Template, &path, &OpenOptions::default());
        let rec = s.recording("rec").unwrap();
        assert_eq!((rec.info().samples, rec.info().channel_count()), (2000, 3));
        let mut out = vec![0.0; 2];
        rec.read(&[2], 10..12, &mut out).unwrap();
        assert_eq!(out, vec![(210.0 * 1e-6) as f32, (211.0 * 1e-6) as f32]);
        assert_eq!(s.electrodes.len(), 3);
        assert_eq!(s.event_series("markers").unwrap().labels, vec!["start", "stop"]);
        assert!(Template.detect(&dir.join("rec.events.csv")).is_none());
    }
}
