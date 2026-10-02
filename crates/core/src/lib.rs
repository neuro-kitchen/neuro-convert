//! nc-core: the neutral data model. Readers turn source files into a [`Session`]; writers turn a
//! `Session` into an output format. This crate is the only contract between the two, so a new
//! reader depends on `nc-core` (and `nc-base`) and nothing else.
//!
//! - [`Session`]: everything one recording produced (continuous recordings, events, snippets,
//!   electrodes, tables, metadata, provenance). [`Session::validate`] checks its invariants.
//! - [`Recording`]: a continuous multi-channel signal read in bounded chunks, so hours-long files
//!   never sit in memory.
//! - [`Reader`]: what a format crate implements to be detected and opened; the `testkit` feature
//!   provides the conformance checks every reader runs.
//! - [`MetadataFile`]: the user's YAML with what the source files do not record;
//!   [`MetadataFile::apply`] merges its electrode declarations into a session.
//!
//! Writers read typed fields only; `metadata` maps on recordings and sessions are reader extras
//! for reports.

pub mod apply;
pub mod electrodes;
pub mod events;
pub mod issue;
pub mod memory;
pub mod metadata;
pub mod metadata_file;
pub mod options;
pub mod provenance;
pub mod reader;
pub mod recording;
pub mod session;
pub mod snippets;
pub mod table;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod validate;

pub use electrodes::{ChannelRef, Electrode, ElectrodeGroup};
pub use events::EventSeries;
pub use issue::{Issue, Level, Target};
pub use memory::MemoryRecording;
pub use metadata::{Device, SessionMetadata, Subject};
pub use metadata_file::{ElectrodeGroupSpec, ImpedanceSpec, ItemKind, ItemSpec, MetadataFile, SnippetSpec, StreamSpec, StreamType};
pub use nc_base::{Error, MemoryOrder, Result, SampleType};
pub use options::OpenOptions;
pub use provenance::{Checksum, Provenance, SourceFile};
pub use reader::{Detection, Reader};
pub use recording::{check_read, Calibration, ChannelInfo, Recording, RecordingInfo, SignalKind};
pub use session::Session;
pub use snippets::SnippetSeries;
pub use table::Table;
