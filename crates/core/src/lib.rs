//! nc-core: the neutral data model. Readers turn source files into a [`Session`]; writers turn a
//! `Session` into an output format. This crate is the only contract between the two, so a new
//! reader depends on `nc-core` (and `nc-base`) and nothing else.
//!
//! - [`Session`]: everything one recording produced (continuous recordings, events, snippets,
//!   tables, metadata, provenance).
//! - [`Recording`]: a continuous multi-channel signal read in bounded chunks, so hours-long files
//!   never sit in memory.
//! - [`Reader`]: what a format crate implements to be detected and opened.
//! - [`MetadataFile`]: the user's YAML with what the source files do not record.

pub mod events;
pub mod issue;
pub mod memory;
pub mod metadata;
pub mod metadata_file;
pub mod options;
pub mod probe;
pub mod provenance;
pub mod reader;
pub mod recording;
pub mod session;
pub mod snippets;
pub mod table;

pub use events::EventSeries;
pub use issue::{Issue, Level};
pub use memory::MemoryRecording;
pub use metadata::{Device, SessionMetadata, Subject};
pub use metadata_file::{ElectrodeGroupSpec, ImpedanceSpec, ItemSpec, MetadataFile, SnippetSpec, StreamSpec, StreamType};
pub use nc_base::{Error, MemoryOrder, Result, SampleType};
pub use options::OpenOptions;
pub use probe::{Contact, ProbeGeometry};
pub use provenance::{Provenance, SourceFile};
pub use reader::{Detection, Reader};
pub use recording::{check_read, ChannelInfo, Recording, RecordingInfo, SignalKind};
pub use session::Session;
pub use snippets::SnippetSeries;
pub use table::Table;
