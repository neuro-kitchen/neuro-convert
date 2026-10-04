//! The error type of every neuro-convert crate.

use std::path::PathBuf;

/// Result with [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Every failure a reader, the writer or the API reports.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Reading or writing `path` failed.
    #[error("{path}: {source}")]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },

    /// A file that claims to be `format` but does not follow it.
    #[error("{format}: {message}")]
    Format {
        /// Format name, e.g. `tdt`, `nwb-zarr`.
        format: &'static str,
        /// What is wrong.
        message: String,
    },

    /// A valid input this build or version cannot handle.
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// No reader detects the path.
    #[error("no reader recognizes {0}")]
    UnknownFormat(PathBuf),

    /// A read asked for a channel the recording does not have.
    #[error("channel {channel} is outside 0..{total}")]
    Channel {
        /// Requested channel index.
        channel: usize,
        /// Channels in the recording.
        total: usize,
    },

    /// A read asked for samples past the end of the recording.
    #[error("sample range {start}..{end} is outside 0..{total}")]
    SampleRange {
        /// First requested sample.
        start: u64,
        /// One past the last requested sample.
        end: u64,
        /// Samples in the recording.
        total: u64,
    },

    /// The output buffer of a read has the wrong length.
    #[error("output buffer holds {actual} values, the request needs {expected}")]
    BufferSize {
        /// Values the request needs.
        expected: usize,
        /// Values the buffer holds.
        actual: usize,
    },

    /// The operation was stopped through its cancel flag.
    #[error("cancelled")]
    Cancelled,
}

impl Error {
    /// [`Error::Io`] for `path`.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io { path: path.into(), source }
    }

    /// [`Error::Format`]: the file is not valid `format`.
    pub fn format(format: &'static str, message: impl Into<String>) -> Self {
        Error::Format { format, message: message.into() }
    }
}
