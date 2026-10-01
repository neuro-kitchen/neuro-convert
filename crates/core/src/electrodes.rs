//! Electrodes: the physical contacts behind electrical channels and snippets.
//!
//! A reader that knows its hardware (probe geometry, channel maps, impedances) fills these; the
//! user's metadata file adds what the source does not record ([`crate::MetadataFile::apply`]).
//! Writers only read them, so an electrode table always comes from the model.

use serde::{Deserialize, Serialize};

/// Contacts that share a device and an anatomical location (a probe, a shank, an EMG grid).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ElectrodeGroup {
    pub name: String,
    pub description: String,
    /// Anatomical location (`unknown` when not known).
    pub location: String,
    /// Name of one of the session's devices; `None` = the session's first device.
    pub device: Option<String>,
}

/// One channel of a continuous recording: recording name and 0-based channel index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRef {
    pub recording: String,
    pub channel: usize,
}

/// One recording contact.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Electrode {
    /// Contact or channel name (e.g. `HDEG 3`).
    pub name: String,
    /// Name of its [`ElectrodeGroup`].
    pub group: String,
    /// Continuous channels this contact feeds: usually one, several when a source splits a site
    /// into bands (Neuropixels AP + LF); empty for contacts seen only in snippets.
    pub channels: Vec<ChannelRef>,
    /// Anatomical location when it differs from the group's.
    pub location: Option<String>,
    /// Position relative to the probe, in µm (x, y, z).
    pub position_um: Option<[f32; 3]>,
    pub impedance_ohms: Option<f32>,
}
