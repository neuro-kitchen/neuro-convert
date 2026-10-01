//! nc-convert: the API the CLI and the app use. It knows which readers are compiled in (cargo
//! features), opens a recording with the reader that claims it, and re-exports the model and the
//! NWB writer so callers need this one dependency.
//!
//! Readers outside this repository plug in without a fork:
//! `Registry::builtin().with(MyReader)`.

pub mod registry;

pub use nc_base as base;
pub use nc_core as core;
#[cfg(feature = "nwb")]
pub use nc_nwb as nwb;
#[cfg(feature = "tdt")]
pub use nc_tdt as tdt;

pub use nc_core::{Detection, Error, MetadataFile, OpenOptions, Reader, Result, Session};
pub use registry::{detect, open, Registry};
