//! The RHD / RHS header: global settings, notes, then signal groups (ports and the board) with
//! their channels. In a traditional `.rhd` / `.rhs` file it is followed by the data blocks; in
//! the RHX "one file per signal type / per channel" layouts it is the whole `info.rhd` /
//! `info.rhs`.
//!
//! Field order follows Intan's own readers (`importrhdutilities.py`, `importrhsutilities.py`)
//! and neo's `IntanRawIO`: version-dependent fields in RHD (temperature sensors ≥ 1.1, board
//! mode ≥ 1.3, reference channel ≥ 2.0); RHS adds settle / recovery / stimulation settings and
//! the DC-amplifier flag.

use nc_base::{Error, Result};

/// First 4 bytes of an RHD file.
pub const RHD_MAGIC: u32 = 0xC691_2702;
/// First 4 bytes of an RHS file.
pub const RHS_MAGIC: u32 = 0xD691_27AC;

/// RHD or RHS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// RHD2000 / RHD USB interface / Recording Controller.
    Rhd,
    /// RHS2000 Stimulation / Recording Controller.
    Rhs,
}

/// What a channel carries (the header's `signal_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Headstage amplifier (electrode) channel.
    Amplifier,
    /// RHD: headstage auxiliary input (accelerometer…), sampled at a quarter of the rate.
    Aux,
    /// RHD: headstage supply voltage, once per block.
    Supply,
    /// Board analog inputs.
    AnalogIn,
    /// RHS: board analog outputs.
    AnalogOut,
    /// Board digital inputs.
    DigitalIn,
    /// Board digital outputs.
    DigitalOut,
}

/// One enabled channel of the header.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    /// `A-000`, `A-AUX1`, `ANALOG-IN-1`, `DIGITAL-IN-01`.
    pub native: String,
    /// User-given name (often the same as `native`).
    pub custom: String,
    /// Index within its signal group; the bit of a digital channel.
    pub native_order: u16,
    /// What it carries.
    pub signal: Signal,
    /// Port of the signal group (`A`…`H`) or `board`.
    pub group: String,
    /// Ω at the impedance test frequency (0 when not measured).
    pub impedance_ohms: f32,
}

/// A parsed RHD / RHS header.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    /// RHD or RHS.
    pub kind: Kind,
    /// File format version (major, minor).
    pub version: (i16, i16),
    /// Amplifier sample rate (Hz).
    pub sample_rate: f64,
    /// The three user notes.
    pub notes: Vec<String>,
    /// RHD: temperature sensor channels saved per block.
    pub temp_sensors: usize,
    /// RHD: decides the board ADC scale (0, 1 or 13).
    pub board_mode: i16,
    /// Reference channel name (≥ 2.0; empty when not recorded).
    pub reference: String,
    /// RHS: DC amplifier data saved next to the amplifier data.
    pub dc_saved: bool,
    /// RHS: amperes per stimulation step.
    pub stim_step: f64,
    /// Filter settings, for the record.
    pub bandwidth: (f32, f32),
    /// DSP high-pass cutoff (Hz), when enabled.
    pub dsp_cutoff: Option<f32>,
    /// Enabled channels, in header order.
    pub channels: Vec<Channel>,
    /// Bytes of the header (data blocks start here in a traditional file).
    pub size: usize,
}

impl Header {
    /// Samples per data block of the traditional layout.
    pub fn block(&self) -> usize {
        match self.kind {
            Kind::Rhd if self.version.0 < 2 => 60,
            _ => 128,
        }
    }

    /// Channels carrying `signal`.
    pub fn of(&self, signal: Signal) -> impl Iterator<Item = &Channel> {
        self.channels.iter().filter(move |c| c.signal == signal)
    }

    /// Number of channels carrying `signal`.
    pub fn count(&self, signal: Signal) -> usize {
        self.of(signal).count()
    }

    /// RHD files before 1.2 store unsigned timestamps.
    pub fn signed_timestamps(&self) -> bool {
        !(self.kind == Kind::Rhd && self.version < (1, 2))
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len()).ok_or_else(|| Error::format("intan", "the header ends early"))?;
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    /// Qt `QString`: byte length (0xFFFFFFFF = null) then UTF-16LE.
    fn qstring(&mut self) -> Result<String> {
        let n = self.u32()?;
        if n == 0xFFFF_FFFF || n == 0 {
            return Ok(String::new());
        }
        if n % 2 != 0 || n > 1 << 20 {
            return Err(Error::format("intan", format!("bad string length {n} in the header")));
        }
        let units: Vec<u16> = self.take(n as usize)?.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
        Ok(String::from_utf16_lossy(&units))
    }
}

/// The kind of file from its first four bytes.
pub fn kind_of(bytes: &[u8]) -> Option<Kind> {
    match u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) {
        RHD_MAGIC => Some(Kind::Rhd),
        RHS_MAGIC => Some(Kind::Rhs),
        _ => None,
    }
}

/// Parses the header at the start of `bytes`.
pub fn parse(bytes: &[u8]) -> Result<Header> {
    let kind = kind_of(bytes).ok_or_else(|| Error::format("intan", "not an RHD / RHS file (magic number)"))?;
    let mut c = Cursor { bytes, at: 4 };
    let version = (c.i16()?, c.i16()?);
    let sample_rate = c.f32()? as f64;
    let dsp_enabled = c.i16()? != 0;
    let dsp = c.f32()?;
    let lower = c.f32()?;
    if kind == Kind::Rhs {
        c.f32()?; // actual lower settle bandwidth
    }
    let upper = c.f32()?;
    c.take(4 * if kind == Kind::Rhs { 4 } else { 3 })?; // desired DSP cutoff / bandwidths
    c.i16()?; // notch filter mode
    c.take(8)?; // desired / actual impedance test frequency
    let (mut stim_step, mut dc_saved, mut temp_sensors, mut board_mode, mut reference) = (0.0, false, 0, 0, String::new());
    if kind == Kind::Rhs {
        c.i16()?; // amp settle mode
        c.i16()?; // charge recovery mode
        stim_step = c.f32()? as f64;
        c.take(8)?; // recovery current limit, target voltage
    }
    let notes: Vec<String> = (0..3).map(|_| c.qstring()).collect::<Result<Vec<_>>>()?.into_iter().filter(|n| !n.trim().is_empty()).collect();
    match kind {
        Kind::Rhs => {
            dc_saved = c.i16()? != 0;
            board_mode = c.i16()?;
            reference = c.qstring()?;
        }
        Kind::Rhd => {
            if version >= (1, 1) {
                temp_sensors = c.i16()?.max(0) as usize;
            }
            if version >= (1, 3) {
                board_mode = c.i16()?;
            }
            if version >= (2, 0) {
                reference = c.qstring()?;
            }
        }
    }
    let groups = c.i16()?.max(0);
    let mut channels = Vec::new();
    for _ in 0..groups {
        let _name = c.qstring()?;
        let prefix = c.qstring()?;
        let enabled = c.i16()? != 0;
        let count = c.i16()?.max(0);
        c.i16()?; // amplified channels
        if !enabled {
            continue;
        }
        for _ in 0..count {
            let native = c.qstring()?;
            let custom = c.qstring()?;
            let native_order = c.i16()?;
            c.i16()?; // custom order
            let signal_type = c.i16()?;
            let enabled = c.i16()? != 0;
            c.i16()?; // chip channel
            if kind == Kind::Rhs {
                c.i16()?; // command stream
            }
            c.i16()?; // board stream
            c.take(8)?; // spike scope trigger mode, threshold, digital channel, edge
            let impedance_ohms = c.f32()?;
            c.f32()?; // impedance phase
            let signal = match (kind, signal_type) {
                (_, 0) => Signal::Amplifier,
                (Kind::Rhd, 1) => Signal::Aux,
                (Kind::Rhd, 2) => Signal::Supply,
                (_, 3) => Signal::AnalogIn,
                (Kind::Rhd, 4) | (Kind::Rhs, 5) => Signal::DigitalIn,
                (Kind::Rhd, 5) | (Kind::Rhs, 6) => Signal::DigitalOut,
                (Kind::Rhs, 4) => Signal::AnalogOut,
                (_, t) => return Err(Error::format("intan", format!("channel {native}: unknown signal type {t}"))),
            };
            if enabled {
                let group = if prefix.is_empty() { "board".to_string() } else { prefix.clone() };
                channels.push(Channel { native, custom, native_order: native_order.max(0) as u16, signal, group, impedance_ohms });
            }
        }
    }
    Ok(Header {
        kind,
        version,
        sample_rate,
        notes,
        temp_sensors,
        board_mode,
        reference,
        dc_saved,
        stim_step,
        bandwidth: (lower, upper),
        dsp_cutoff: dsp_enabled.then_some(dsp),
        channels,
        size: c.at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal RHD 3.0 header with one port (2 amplifier channels) and a board digital input.
    pub fn rhd_header(amplifiers: usize) -> Vec<u8> {
        let mut b = Vec::new();
        let q = |b: &mut Vec<u8>, s: &str| {
            let u: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
            b.extend((u.len() as u32).to_le_bytes());
            b.extend(u);
        };
        b.extend(RHD_MAGIC.to_le_bytes());
        b.extend(3i16.to_le_bytes());
        b.extend(0i16.to_le_bytes());
        b.extend(20_000f32.to_le_bytes());
        b.extend(1i16.to_le_bytes()); // dsp enabled
        b.extend(1f32.to_le_bytes());
        b.extend(0.1f32.to_le_bytes());
        b.extend(7500f32.to_le_bytes());
        b.extend([0u8; 12]);
        b.extend(0i16.to_le_bytes());
        b.extend([0u8; 8]);
        q(&mut b, "a note");
        q(&mut b, "");
        b.extend(0xFFFF_FFFFu32.to_le_bytes());
        b.extend(0i16.to_le_bytes()); // temp sensors
        b.extend(0i16.to_le_bytes()); // board mode
        q(&mut b, "Hardware");
        b.extend(2i16.to_le_bytes()); // groups
        let channel = |b: &mut Vec<u8>, name: &str, order: i16, ty: i16| {
            q(b, name);
            q(b, name);
            b.extend(order.to_le_bytes());
            b.extend(order.to_le_bytes());
            b.extend(ty.to_le_bytes());
            b.extend(1i16.to_le_bytes());
            b.extend([0u8; 4]);
            b.extend([0u8; 8]);
            b.extend(150_000f32.to_le_bytes());
            b.extend(0f32.to_le_bytes());
        };
        q(&mut b, "Port A");
        q(&mut b, "A");
        b.extend(1i16.to_le_bytes());
        b.extend((amplifiers as i16).to_le_bytes());
        b.extend((amplifiers as i16).to_le_bytes());
        for i in 0..amplifiers {
            channel(&mut b, &format!("A-{i:03}"), i as i16, 0);
        }
        q(&mut b, "Board Digital Inputs");
        q(&mut b, "DIN");
        b.extend(1i16.to_le_bytes());
        b.extend(1i16.to_le_bytes());
        b.extend(0i16.to_le_bytes());
        channel(&mut b, "DIGITAL-IN-01", 0, 4);
        b
    }

    #[test]
    fn test_parse_rhd() {
        let bytes = rhd_header(2);
        let h = parse(&bytes).unwrap();
        assert_eq!((h.kind, h.version, h.sample_rate, h.block()), (Kind::Rhd, (3, 0), 20_000.0, 128));
        assert_eq!(h.notes, vec!["a note"]);
        assert_eq!(h.reference, "Hardware");
        assert_eq!(h.count(Signal::Amplifier), 2);
        assert_eq!(h.channels[1].native, "A-001");
        assert_eq!(h.channels[0].group, "A");
        assert_eq!(h.channels[2].signal, Signal::DigitalIn);
        assert_eq!(h.size, bytes.len());
        assert!(parse(&bytes[..bytes.len() - 3]).is_err(), "truncated");
        assert!(parse(&[0u8; 8]).is_err());
    }
}
