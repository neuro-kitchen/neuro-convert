//! Session-level metadata: identification, subject, devices, notes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Session-level metadata. Inputs fill what their files carry; the user's metadata file fills
/// the rest before exporting (NWB requires e.g. `start_time`, `description`, subject species).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// NWB identifier; `None` = a new UUID at export.
    pub identifier: Option<String>,
    /// Session description (NWB requires one).
    pub description: Option<String>,
    /// ISO 8601 local time as recorded (time zone added from user metadata when known).
    pub start_time: Option<String>,
    /// ISO 8601 local time the recording stopped, when recorded.
    pub stop_time: Option<String>,
    /// Experiment or protocol name as recorded.
    pub experiment: Option<String>,
    /// People who ran the session.
    pub experimenters: Vec<String>,
    /// Lab name.
    pub lab: Option<String>,
    /// Institution name.
    pub institution: Option<String>,
    /// The animal or person recorded.
    pub subject: Subject,
    /// Acquisition hardware (amplifiers, probes, headstages).
    pub devices: Vec<Device>,
    /// Free text notes recorded during the session.
    pub notes: Vec<String>,
    /// Anything else worth keeping (software versions, rig names, …).
    pub extra: BTreeMap<String, String>,
}

/// The animal or person recorded.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    /// Subject id.
    pub id: Option<String>,
    /// Latin binomial (`Rattus norvegicus`).
    pub species: Option<String>,
    /// `M`, `F`, `U` or `O`.
    pub sex: Option<String>,
    /// ISO 8601 duration, e.g. `P90D`.
    pub age: Option<String>,
    /// Free text.
    pub description: Option<String>,
}

/// A piece of acquisition hardware.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Device {
    /// Device name, unique in the session.
    pub name: String,
    /// What the device is.
    pub description: String,
    /// Manufacturer (`Tucker-Davis Technologies`, `IMEC`, …).
    pub manufacturer: Option<String>,
    /// Model of the device (e.g. `RZ2`), written as an NWB `DeviceModel` with the manufacturer.
    #[serde(default)]
    pub model: Option<String>,
}
