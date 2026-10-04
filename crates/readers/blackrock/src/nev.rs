//! `.nev`: spike waveforms, digital / serial input words and comments, as fixed-size packets.
//!
//! 336-byte header (`NEURALEV` 2.x / `BREVENTS` 3.0: spec, flags, header and packet sizes,
//! timestamp resolution, UTC time origin, application, comment, extended-header count), then
//! 32-byte extended headers (`NEUEVWAV`: per electrode, digitization factor in nV per bit,
//! waveform bytes per sample and width; `NEUEVLBL`: electrode labels), then packets: timestamp
//! (uint32; 3.0 uint64), uint16 packet id, payload. Packet id 0 = digital / serial input (insertion
//! reason, digital word), 1–2048 = a spike on that electrode (unit class, waveform), 0xFFFF = a
//! comment (2.3+). A timestamp going back by more than a second, or a "critical load restart"
//! comment, starts a new clock epoch (neo's segments); smaller steps back (PTP jitter across hubs)
//! stay in the epoch.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use nc_base::mapped::MappedFile;
use nc_core::{Error, Result, Waveforms};

/// Waveform settings of one electrode (`NEUEVWAV`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wave {
    /// nV per bit.
    pub digitization: u16,
    /// Bytes per waveform sample (1, 2 or 4).
    pub bytes: u8,
    /// Samples per waveform (0: fill the packet).
    pub width: u16,
    /// Front-end bank (1 = A, …).
    pub connector: u8,
    /// Pin on the bank.
    pub pin: u8,
}

/// A packet's place in the file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Packet {
    /// Timestamp in ticks of the file's resolution.
    pub timestamp: u64,
    /// Packet id: 0 digital / serial input, 1–2048 electrode spikes, 0xFFFF comments, …
    pub id: u16,
    /// Byte offset of the packet.
    pub offset: usize,
    /// Clock epoch (0 until the first clock reset).
    pub epoch: usize,
}

/// A `.nev` file: header, extended headers and the index of its packets, memory-mapped.
#[derive(Debug)]
pub struct Nev {
    /// The mapped file.
    pub file: Arc<MappedFile>,
    /// File spec (`2.3`, `3.0`).
    pub spec: String,
    /// Timestamp ticks per second.
    pub timestamp_resolution: u64,
    /// UTC time origin `[year, month, weekday, day, hour, minute, second, ms]`.
    pub origin: [u16; 8],
    /// Application that wrote the file.
    pub application: String,
    /// Header comment.
    pub comment: String,
    /// Waveforms are int16 whatever `NEUEVWAV` says (header flag bit 0).
    pub all_int16: bool,
    /// Bytes per packet.
    pub packet_bytes: usize,
    /// Waveform settings per electrode id.
    pub waves: BTreeMap<u16, Wave>,
    /// Electrode labels (`NEUEVLBL`) per id.
    pub labels: BTreeMap<u16, String>,
    /// Every packet, in file order.
    pub packets: Vec<Packet>,
    /// Number of clock epochs.
    pub epochs: usize,
    /// Problems met while indexing.
    pub warnings: Vec<String>,
}

fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

impl Nev {
    /// Bytes of a packet timestamp (8 from spec 3.0, else 4).
    pub fn ts_bytes(&self) -> usize {
        if self.spec.starts_with('3') { 8 } else { 4 }
    }

    /// Maps the file at `path`, parses its headers and indexes its packets.
    pub fn open(path: &Path) -> Result<Self> {
        let file = Arc::new(MappedFile::open(path)?);
        let mapped = file.clone();
        let b = mapped.bytes();
        let bad = |m: &str| Error::format("blackrock", format!("{}: {m}", path.display()));
        if b.len() < 336 || (&b[..8] != b"NEURALEV" && &b[..8] != b"BREVENTS") {
            return Err(bad("not a NEV file"));
        }
        let spec = format!("{}.{}", b[8], b[9]);
        let flags = u16_at(b, 10);
        let header = u32_at(b, 12) as usize;
        let packet_bytes = u32_at(b, 16) as usize;
        let timestamp_resolution = u32_at(b, 20) as u64;
        let mut origin = [0u16; 8];
        for (k, o) in origin.iter_mut().enumerate() {
            *o = u16_at(b, 28 + 2 * k);
        }
        let ext = u32_at(b, 332) as usize;
        if packet_bytes < 8 || header > b.len() {
            return Err(bad("bad header sizes"));
        }
        let mut waves = BTreeMap::new();
        let mut labels = BTreeMap::new();
        let has_width = spec.as_str() >= "2.2";
        for k in 0..ext {
            let at = 336 + 32 * k;
            if at + 32 > b.len() {
                break;
            }
            match &b[at..at + 8] {
                b"NEUEVWAV" => {
                    let id = u16_at(b, at + 8);
                    waves.insert(id, Wave { digitization: u16_at(b, at + 12), bytes: b[at + 21], width: if has_width { u16_at(b, at + 22) } else { 0 }, connector: b[at + 10], pin: b[at + 11] });
                }
                b"NEUEVLBL" => {
                    labels.insert(u16_at(b, at + 8), text(&b[at + 10..at + 26]));
                }
                _ => {}
            }
        }
        let mut nev = Self {
            file: file.clone(),
            spec,
            timestamp_resolution,
            origin,
            application: text(&b[44..76]),
            comment: text(&b[76..332]),
            all_int16: flags & 1 == 1,
            packet_bytes,
            waves,
            labels,
            packets: Vec::new(),
            epochs: 1,
            warnings: Vec::new(),
        };
        // Packets; a timestamp going backwards (or a restart comment) starts a new epoch
        let tsb = nev.ts_bytes();
        let n = (b.len() - header) / packet_bytes;
        let mut epoch = 0;
        let mut last = 0u64;
        let resets_possible = nev.spec.as_str() >= "2.3";
        let second = timestamp_resolution.max(1);
        let mut others: BTreeMap<u16, usize> = BTreeMap::new();
        for k in 0..n {
            let at = header + k * packet_bytes;
            let ts = if tsb == 8 { u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes")) } else { u32_at(b, at) as u64 };
            let id = u16_at(b, at + tsb);
            let handled = id <= 2048 || id == 0xFFFF;
            if !handled {
                *others.entry(id).or_default() += 1;
                continue;
            }
            let restart = id == 0xFFFF && nev.comment_text(at).starts_with("critical load restart");
            if resets_possible && k > 0 && (ts + second < last || restart) && !nev.packets.is_empty() {
                epoch += 1;
                last = ts;
            }
            last = last.max(ts);
            if restart {
                continue;
            }
            nev.packets.push(Packet { timestamp: ts, id, offset: at, epoch });
        }
        nev.epochs = epoch + 1;
        if !others.is_empty() {
            let list: Vec<String> = others.iter().map(|(id, n)| format!("{n} × id {id:#06x}")).collect();
            nev.warnings.push(format!("NEV packets not read (video sync, tracking, buttons, configuration, system): {}", list.join(", ")));
        }
        Ok(nev)
    }

    /// The comment of the comment packet at `at` (ANSI or UTF-16).
    pub fn comment_text(&self, at: usize) -> String {
        let b = self.file.bytes();
        let tsb = self.ts_bytes();
        let start = at + tsb + 2 + 6;
        let end = at + self.packet_bytes;
        if start >= end || end > b.len() {
            return String::new();
        }
        let raw = &b[start..end];
        if b[at + tsb + 2] == 1 {
            let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).take_while(|&u| u != 0).collect();
            String::from_utf16_lossy(&units).trim().to_string()
        } else {
            text(raw)
        }
    }

    /// (insertion reason, digital word) of a packet with id 0.
    pub fn digital(&self, at: usize) -> (u8, u16) {
        let b = self.file.bytes();
        let p = at + self.ts_bytes() + 2;
        (b[p], u16_at(b, p + 2))
    }

    /// Unit class of a spike packet.
    pub fn unit(&self, at: usize) -> u8 {
        self.file.bytes()[at + self.ts_bytes() + 2]
    }

    /// (bytes per sample, samples) of electrode `id`'s waveforms.
    pub fn waveform_shape(&self, id: u16) -> (usize, usize) {
        let room = self.packet_bytes - self.ts_bytes() - 4;
        let w = self.waves.get(&id);
        let bytes = if self.all_int16 { 2 } else { w.map_or(2, |w| match w.bytes { 0 | 1 => 1, 4 => 4, _ => 2 }) };
        let width = w.map(|w| w.width as usize).filter(|&n| n > 0 && n * bytes <= room).unwrap_or(room / bytes);
        (bytes, width)
    }
}

/// Spike waveforms of a NEV, read from their packets.
pub struct NevWaveforms {
    /// The file the packets are in.
    pub nev: Arc<Nev>,
    /// (packet offset, bytes per sample, volts per bit) per snippet.
    pub spikes: Vec<(usize, u8, f64)>,
    /// Samples per waveform.
    pub samples: usize,
}

impl std::fmt::Debug for NevWaveforms {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NevWaveforms").field("spikes", &self.spikes.len()).field("samples", &self.samples).finish()
    }
}

impl Waveforms for NevWaveforms {
    fn count(&self) -> usize {
        self.spikes.len()
    }

    fn read(&self, snippets: &[usize], out: &mut [f32]) -> Result<()> {
        let b = self.nev.file.bytes();
        let start = self.nev.ts_bytes() + 4;
        for (o, &i) in out.chunks_exact_mut(self.samples.max(1)).zip(snippets) {
            let &(at, bytes, scale) = self.spikes.get(i).ok_or_else(|| Error::format("blackrock", format!("spike {i} out of range")))?;
            let w = at + start;
            for (k, v) in o.iter_mut().enumerate() {
                let p = w + k * bytes as usize;
                let raw = match bytes {
                    1 => b[p] as i8 as f64,
                    4 => i32::from_le_bytes(b[p..p + 4].try_into().expect("4 bytes")) as f64,
                    _ => i16::from_le_bytes([b[p], b[p + 1]]) as f64,
                };
                *v = (raw * scale) as f32;
            }
        }
        Ok(())
    }
}
