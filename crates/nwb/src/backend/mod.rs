//! Storage behind NWB: groups, attributes and typed datasets at HDMF paths.
//!
//! The NWB types only speak this trait; Zarr and HDF5 sit behind it. Attribute conventions follow
//! hdmf-zarr (`_DTYPE`, `_ARRAY_DIMENSIONS`, `_LINKS`, `_REFERENCE`), which the HDF5 backend
//! translates to native links and references.

#[cfg(feature = "hdf5")]
pub mod hdf5;
pub mod zarr;

use std::path::Path;

use serde_json::{Map, Value};

use nc_base::{Result, SampleType};

pub type Attrs = Map<String, Value>;

/// Streams a large dataset in pieces along its first dimension (safe to share between threads).
pub trait RowSink: Send + Sync {
    /// Writes rows `start..start + rows` (the first dimension) from row-major little-endian
    /// bytes of the dataset's sample type.
    fn write_rows(&self, start: u64, rows: u64, bytes: &[u8]) -> Result<()>;
}

pub trait Backend {
    fn group(&self, path: &str, attrs: Attrs) -> Result<()>;
    /// Scalar string dataset.
    fn string(&self, path: &str, value: &str, attrs: Attrs) -> Result<()>;
    /// 1-D string dataset; `dim` names its dimension.
    fn strings(&self, path: &str, values: &[String], dim: &str, attrs: Attrs) -> Result<()>;
    /// Scalar float64 dataset.
    fn f64_scalar(&self, path: &str, value: f64, attrs: Attrs) -> Result<()>;
    /// Float64 dataset of `shape` (row-major values), dimensions named `dims`.
    fn f64s(&self, path: &str, values: &[f64], shape: &[u64], dims: &[&str], attrs: Attrs) -> Result<()>;
    fn i64s(&self, path: &str, values: &[i64], dim: &str, attrs: Attrs) -> Result<()>;
    /// 1-D uint64 dataset (`VectorIndex`: NWB wants an unsigned type).
    fn u64s(&self, path: &str, values: &[u64], dim: &str, attrs: Attrs) -> Result<()>;
    /// Starts a large dataset of `shape` and type `ty`, chunked by `chunk_rows` along dimension 0.
    fn stream(&self, path: &str, shape: &[u64], chunk_rows: u64, ty: SampleType, dims: &[&str], attrs: Attrs) -> Result<Box<dyn RowSink>>;

    /// Called once everything is written (the HDF5 backend creates its object references here).
    fn finish(&self) -> Result<()> {
        Ok(())
    }

    /// Small 1-D float32 dataset.
    fn f32s(&self, path: &str, values: &[f32], dim: &str, attrs: Attrs) -> Result<()> {
        let n = values.len() as u64;
        let sink = self.stream(path, &[n], n.max(1), SampleType::F32, &[dim], attrs)?;
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        if n > 0 { sink.write_rows(0, n, &bytes) } else { Ok(()) }
    }
}

/// This build writes NWB/HDF5 (`.nwb`) as well as Zarr.
pub const HDF5: bool = cfg!(feature = "hdf5");

/// `dest` with the extension of `format`: `name.nwb.zarr` ↔ `name.nwb`.
pub fn with_format(dest: &Path, format: Format) -> std::path::PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = name.strip_suffix(".nwb.zarr").or_else(|| name.strip_suffix(".zarr")).or_else(|| name.strip_suffix(".nwb")).unwrap_or(&name);
    dest.with_file_name(match format {
        Format::Zarr => format!("{stem}.nwb.zarr"),
        Format::Hdf5 => format!("{stem}.nwb"),
    })
}

/// Where `dest` is written: NWB on HDF5 for a `.nwb` file, else a Zarr store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zarr,
    Hdf5,
}

impl Format {
    pub fn of(dest: &Path) -> Self {
        let name = dest.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if name.ends_with(".nwb") && !dest.is_dir() { Format::Hdf5 } else { Format::Zarr }
    }
}

/// The backend for `dest` ([`Format::of`]).
pub fn create(dest: &Path, gzip: Option<u32>, overwrite: bool) -> Result<Box<dyn Backend>> {
    match Format::of(dest) {
        Format::Zarr => Ok(Box::new(zarr::ZarrBackend::create(dest, gzip, overwrite)?)),
        #[cfg(feature = "hdf5")]
        Format::Hdf5 => Ok(Box::new(hdf5::Hdf5Backend::create(dest, gzip, overwrite)?)),
        #[cfg(not(feature = "hdf5"))]
        Format::Hdf5 => Err(nc_base::Error::Unsupported(format!(
            "{} is an NWB/HDF5 file, but this build writes HDF5 only with the `hdf5` (or `hdf5-static`) feature; write a `.nwb.zarr` store instead",
            dest.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_name() {
        assert_eq!(Format::of(Path::new("/x/rec.nwb.zarr")), Format::Zarr);
        assert_eq!(Format::of(Path::new("/x/rec.NWB")), Format::Hdf5);
        assert_eq!(with_format(Path::new("/x/rec.nwb.zarr"), Format::Hdf5), Path::new("/x/rec.nwb"));
        assert_eq!(with_format(Path::new("/x/rec.nwb"), Format::Zarr), Path::new("/x/rec.nwb.zarr"));
        assert_eq!(with_format(Path::new("rec"), Format::Hdf5), Path::new("rec.nwb"));
    }
}
