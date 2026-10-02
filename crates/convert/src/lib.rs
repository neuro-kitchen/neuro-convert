//! nc-convert: the API the CLI and the app use. It knows which readers are compiled in (cargo
//! features), opens a recording with the reader that claims it, and runs a conversion as a
//! [`Job`] (open → plan → write → verify, with progress and cancellation). It re-exports the
//! model and the NWB writer so callers need this one dependency.
//!
//! Readers outside this repository plug in without a fork:
//! `Registry::builtin().with(MyReader)`.

#[cfg(feature = "nwb")]
pub mod job;
pub mod preview;
pub mod registry;
#[cfg(feature = "nwb")]
pub mod sources;

pub use nc_base as base;
pub use nc_core as core;
#[cfg(feature = "nwb")]
pub use nc_nwb as nwb;
#[cfg(feature = "intan")]
pub use nc_intan as intan;
#[cfg(feature = "openephys")]
pub use nc_openephys as openephys;
#[cfg(feature = "spikeglx")]
pub use nc_spikeglx as spikeglx;
#[cfg(feature = "tdt")]
pub use nc_tdt as tdt;

#[cfg(feature = "nwb")]
pub use job::{CancelToken, Event, Job, Report, Stage};
pub use nc_core::{Detection, Error, MetadataFile, OpenOptions, Reader, Result, Session};
pub use registry::{detect, open, Registry};
