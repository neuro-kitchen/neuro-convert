//! nc-base: primitives shared by every neuro-convert crate. No neuroscience and no output format
//! knowledge lives here: errors, sample types, little-endian decoding, memory-mapped files, text
//! from legacy acquisition software and ISO 8601 time helpers.

#![warn(missing_docs)]

/// This crate's version (`nc-base`, from its `Cargo.toml`): recorded in every conversion's
/// provenance and report, so a problem in a file can be traced to the code that wrote it.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// [`VERSION`].
pub fn version() -> &'static str {
    VERSION
}

pub mod codec;
pub mod error;
pub mod mapped;
pub mod sample;
pub mod text;
pub mod time;

pub use error::{Error, Result};
pub use sample::{MemoryOrder, SampleType};
