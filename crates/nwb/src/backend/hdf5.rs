//! NWB on HDF5 (`.nwb`), the layout pynwb / HDMF write (feature `hdf5`; `hdf5-static` builds the
//! HDF5 library from source instead of linking the system's).
//!
//! The NWB types speak hdmf-zarr's conventions; here they become native HDF5:
//! - `_LINKS` attributes → soft links in the group;
//! - `{"_REFERENCE": {"path": …}}` attribute values, the root's `.specloc` and string datasets
//!   marked `_DTYPE: object_reference` → object references (created in [`Backend::finish`], once
//!   every target exists);
//! - `_DTYPE` / `_ARRAY_DIMENSIONS` are dropped (HDF5 types and shapes carry them);
//! - strings are variable-length UTF-8; large datasets are chunked along the first dimension and
//!   deflate-compressed at the store's gzip level.

use std::path::Path;
use std::sync::Mutex;

use hdf5_metno as h5;
use h5::types::VarLenUnicode;
use h5::{Group, Hyperslab, Location, SliceOrIndex};
use serde_json::Value;

use super::{Attrs, Backend, RowSink};
use nc_base::{Error, Result, SampleType};

fn err(context: &str, e: impl std::fmt::Display) -> Error {
    Error::format("nwb-hdf5", format!("{context}: {e}"))
}

fn text(s: &str) -> Result<VarLenUnicode> {
    s.replace('\0', "").parse().map_err(|e| err("string", e))
}

/// An object reference to create when the file is complete.
enum Pending {
    /// Attribute `name` on `owner` → `target`.
    Attr { owner: String, name: String, target: String },
    /// 1-D dataset `path` of references to `targets`, with `attrs`.
    Dataset { path: String, targets: Vec<String>, attrs: Attrs },
}

pub struct Hdf5Backend {
    file: h5::File,
    gzip: Option<u32>,
    pending: Mutex<Vec<Pending>>,
}

impl Hdf5Backend {
    /// Creates `path` (which must not exist unless `overwrite`).
    pub fn create(path: &Path, gzip: Option<u32>, overwrite: bool) -> Result<Self> {
        if path.exists() {
            if !overwrite {
                return Err(Error::Unsupported(format!("{} already exists (use --overwrite)", path.display())));
            }
            if path.is_dir() {
                std::fs::remove_dir_all(path).map_err(|e| Error::io(path, e))?;
            } else {
                std::fs::remove_file(path).map_err(|e| Error::io(path, e))?;
            }
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let file = h5::File::create(path).map_err(|e| err(&path.display().to_string(), e))?;
        Ok(Self { file, gzip, pending: Mutex::new(Vec::new()) })
    }

    /// The group at `path`, created (with any missing parents) when absent.
    fn ensure_group(&self, path: &str) -> Result<Group> {
        let path = path.trim_end_matches('/');
        if path.is_empty() {
            return self.file.as_group().map_err(|e| err("/", e));
        }
        if let Ok(g) = self.file.group(path) {
            return Ok(g);
        }
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        let p = self.ensure_group(parent)?;
        p.create_group(name).map_err(|e| err(path, e))
    }

    /// The parent group of `path` (created when absent) and the last name.
    fn parent<'a>(&self, path: &'a str) -> Result<(Group, &'a str)> {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        Ok((self.ensure_group(parent)?, name))
    }

    /// Writes `attrs` on `loc` (`owner` is its path, for deferred references and links).
    fn attrs(&self, loc: &Location, owner: &str, attrs: &Attrs, group: Option<&Group>) -> Result<()> {
        for (name, value) in attrs {
            match (name.as_str(), value) {
                ("_DTYPE" | "_ARRAY_DIMENSIONS", _) => {}
                ("_LINKS", Value::Array(links)) => {
                    let Some(g) = group else { continue };
                    for l in links {
                        if let (Some(path), Some(link)) = (l.get("path").and_then(Value::as_str), l.get("name").and_then(Value::as_str)) {
                            g.link_soft(path, link).map_err(|e| err(&format!("{owner}/{link}"), e))?;
                        }
                    }
                }
                (".specloc", Value::String(target)) => {
                    let target = if target.starts_with('/') { target.clone() } else { format!("/{target}") };
                    self.pending.lock().unwrap().push(Pending::Attr { owner: owner.into(), name: name.clone(), target });
                }
                (_, Value::Object(o)) if o.contains_key("_REFERENCE") => {
                    let target = o["_REFERENCE"].get("path").and_then(Value::as_str).unwrap_or("/").to_string();
                    self.pending.lock().unwrap().push(Pending::Attr { owner: owner.into(), name: name.clone(), target });
                }
                (_, v) => write_attr(loc, name, v).map_err(|e| err(&format!("{owner}@{name}"), e))?,
            }
        }
        Ok(())
    }

    fn location(&self, path: &str) -> Result<Location> {
        let p = path.trim_end_matches('/');
        if p.is_empty() {
            return Ok((**self.file).clone());
        }
        if let Ok(g) = self.file.group(p) {
            return Ok((*g).clone());
        }
        self.file.dataset(p).map(|d| (**d).clone()).map_err(|e| err(path, e))
    }
}

fn write_attr(loc: &Location, name: &str, v: &Value) -> h5::Result<()> {
    match v {
        Value::String(s) => {
            let t: VarLenUnicode = s.replace('\0', "").parse().map_err(|e| h5::Error::from(format!("{e}")))?;
            loc.new_attr::<VarLenUnicode>().create(name)?.write_scalar(&t)
        }
        Value::Bool(b) => loc.new_attr::<bool>().create(name)?.write_scalar(b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => loc.new_attr::<i64>().create(name)?.write_scalar(&i),
            None => loc.new_attr::<f64>().create(name)?.write_scalar(&n.as_f64().unwrap_or(f64::NAN)),
        },
        Value::Array(items) if items.iter().all(Value::is_string) => {
            let v: Vec<VarLenUnicode> = items.iter().filter_map(|x| x.as_str()?.replace('\0', "").parse().ok()).collect();
            loc.new_attr::<VarLenUnicode>().shape([v.len()]).create(name)?.write_raw(&v)
        }
        Value::Array(items) if items.iter().all(|x| x.as_i64().is_some()) => {
            let v: Vec<i64> = items.iter().filter_map(Value::as_i64).collect();
            loc.new_attr::<i64>().shape([v.len()]).create(name)?.write_raw(&v)
        }
        Value::Array(items) => {
            let v: Vec<f64> = items.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect();
            loc.new_attr::<f64>().shape([v.len()]).create(name)?.write_raw(&v)
        }
        Value::Null | Value::Object(_) => Ok(()),
    }
}

impl Backend for Hdf5Backend {
    fn group(&self, path: &str, attrs: Attrs) -> Result<()> {
        let g = self.ensure_group(path)?;
        let owner = if path.trim_end_matches('/').is_empty() { "/" } else { path };
        self.attrs(&g, owner, &attrs, Some(&g))
    }

    fn string(&self, path: &str, value: &str, attrs: Attrs) -> Result<()> {
        let (parent, name) = self.parent(path)?;
        let ds = parent.new_dataset::<VarLenUnicode>().shape(()).create(name).map_err(|e| err(path, e))?;
        ds.write_scalar(&text(value)?).map_err(|e| err(path, e))?;
        self.attrs(&ds, path, &attrs, None)
    }

    fn strings(&self, path: &str, values: &[String], _dim: &str, attrs: Attrs) -> Result<()> {
        if attrs.get("_DTYPE").and_then(Value::as_str) == Some("object_reference") {
            self.pending.lock().unwrap().push(Pending::Dataset { path: path.into(), targets: values.to_vec(), attrs });
            return Ok(());
        }
        let (parent, name) = self.parent(path)?;
        let v: Vec<VarLenUnicode> = values.iter().map(|s| text(s)).collect::<Result<_>>()?;
        let ds = parent.new_dataset::<VarLenUnicode>().shape([v.len()]).create(name).map_err(|e| err(path, e))?;
        if !v.is_empty() {
            ds.write_raw(&v).map_err(|e| err(path, e))?;
        }
        self.attrs(&ds, path, &attrs, None)
    }

    fn f64_scalar(&self, path: &str, value: f64, attrs: Attrs) -> Result<()> {
        let (parent, name) = self.parent(path)?;
        let ds = parent.new_dataset::<f64>().shape(()).create(name).map_err(|e| err(path, e))?;
        ds.write_scalar(&value).map_err(|e| err(path, e))?;
        self.attrs(&ds, path, &attrs, None)
    }

    fn f64s(&self, path: &str, values: &[f64], shape: &[u64], _dims: &[&str], attrs: Attrs) -> Result<()> {
        let (parent, name) = self.parent(path)?;
        let shape: Vec<usize> = shape.iter().map(|&s| s as usize).collect();
        let ds = parent.new_dataset::<f64>().shape(shape).create(name).map_err(|e| err(path, e))?;
        if !values.is_empty() {
            ds.write_raw(values).map_err(|e| err(path, e))?;
        }
        self.attrs(&ds, path, &attrs, None)
    }

    fn i64s(&self, path: &str, values: &[i64], _dim: &str, attrs: Attrs) -> Result<()> {
        let (parent, name) = self.parent(path)?;
        let ds = parent.new_dataset::<i64>().shape([values.len()]).create(name).map_err(|e| err(path, e))?;
        if !values.is_empty() {
            ds.write_raw(values).map_err(|e| err(path, e))?;
        }
        self.attrs(&ds, path, &attrs, None)
    }

    fn u64s(&self, path: &str, values: &[u64], _dim: &str, attrs: Attrs) -> Result<()> {
        let (parent, name) = self.parent(path)?;
        let ds = parent.new_dataset::<u64>().shape([values.len()]).create(name).map_err(|e| err(path, e))?;
        if !values.is_empty() {
            ds.write_raw(values).map_err(|e| err(path, e))?;
        }
        self.attrs(&ds, path, &attrs, None)
    }

    fn stream(&self, path: &str, shape: &[u64], chunk_rows: u64, ty: SampleType, _dims: &[&str], attrs: Attrs) -> Result<Box<dyn RowSink>> {
        let (parent, name) = self.parent(path)?;
        let dims: Vec<usize> = shape.iter().map(|&s| s as usize).collect();
        let mut chunk = dims.clone();
        chunk[0] = (chunk_rows as usize).clamp(1, dims[0].max(1));
        let empty = dims.contains(&0);
        macro_rules! create {
            ($t:ty) => {{
                let b = parent.new_dataset::<$t>().shape(dims.clone());
                // Chunking (and so compression) needs a non-empty shape
                let b = if empty { b } else { b.chunk(chunk.clone()) };
                let b = match (self.gzip, empty) {
                    (Some(level), false) => b.deflate(level as u8),
                    _ => b,
                };
                b.create(name).map_err(|e| err(path, e))?
            }};
        }
        let ds = match ty {
            SampleType::I8 => create!(i8),
            SampleType::I16 => create!(i16),
            SampleType::U16 => create!(u16),
            SampleType::I32 => create!(i32),
            SampleType::I64 => create!(i64),
            SampleType::F32 => create!(f32),
            SampleType::F64 => create!(f64),
        };
        self.attrs(&ds, path, &attrs, None)?;
        let chunk_rows = if empty { 0 } else { chunk[0] };
        Ok(Box::new(Hdf5Rows { ds, ty, rest: dims[1..].to_vec(), rows: dims[0], chunk_rows, gzip: if empty { None } else { self.gzip }, path: path.to_string() }))
    }

    /// Creates the object references, now that every target exists.
    fn finish(&self) -> Result<()> {
        let pending = std::mem::take(&mut *self.pending.lock().unwrap());
        for p in pending {
            match p {
                Pending::Attr { owner, name, target } => {
                    let loc = self.location(&owner)?;
                    let r: h5::ObjectReference1 = self.file.reference(&target).map_err(|e| err(&target, e))?;
                    loc.new_attr::<h5::ObjectReference1>().create(name.as_str()).and_then(|a| a.write_scalar(&r)).map_err(|e| err(&format!("{owner}@{name}"), e))?;
                }
                Pending::Dataset { path, targets, attrs } => {
                    let refs: Vec<h5::ObjectReference1> = targets.iter().map(|t| self.file.reference(t).map_err(|e| err(t, e))).collect::<Result<_>>()?;
                    let (parent, name) = self.parent(&path)?;
                    let ds = parent.new_dataset::<h5::ObjectReference1>().shape([refs.len()]).create(name).map_err(|e| err(&path, e))?;
                    if !refs.is_empty() {
                        ds.write_raw(&refs).map_err(|e| err(&path, e))?;
                    }
                    self.attrs(&ds, &path, &attrs, None)?;
                }
            }
        }
        self.file.flush().map_err(|e| err("flush", e))
    }
}

struct Hdf5Rows {
    ds: h5::Dataset,
    ty: SampleType,
    /// Sizes of the dimensions after the first (written whole).
    rest: Vec<usize>,
    /// Rows of the dataset and per chunk (0: not chunked).
    rows: usize,
    chunk_rows: usize,
    gzip: Option<u32>,
    path: String,
}

impl Hdf5Rows {
    /// Writes one whole chunk with `H5Dwrite_chunk`: compressed here (in the caller's thread, so
    /// chunks compress in parallel) as the deflate filter stores it (a zlib stream), then handed
    /// to HDF5 under its lock. The last chunk is padded to the chunk size, as HDF5 stores it.
    fn write_chunk(&self, start: usize, bytes: &[u8]) -> Result<()> {
        let row_bytes = self.rest.iter().product::<usize>() * self.ty.bytes();
        let full = self.chunk_rows * row_bytes;
        let mut data = bytes.to_vec();
        data.resize(full, 0);
        let payload = match self.gzip {
            Some(level) => {
                use std::io::Write;
                let mut z = flate2::write::ZlibEncoder::new(Vec::with_capacity(full / 2), flate2::Compression::new(level));
                z.write_all(&data).and_then(|_| z.finish()).map_err(|e| err(&self.path, e))?
            }
            None => data,
        };
        let offset: Vec<u64> = std::iter::once(start as u64).chain(self.rest.iter().map(|_| 0)).collect();
        let id = self.ds.id();
        let status = h5::sync::sync(|| unsafe {
            hdf5_metno_sys::h5d::H5Dwrite_chunk(id, hdf5_metno_sys::h5p::H5P_DEFAULT, 0, offset.as_ptr(), payload.len(), payload.as_ptr().cast())
        });
        if status < 0 {
            return Err(err(&self.path, format!("H5Dwrite_chunk failed at row {start}")));
        }
        Ok(())
    }
}

impl RowSink for Hdf5Rows {
    fn write_rows(&self, start: u64, rows: u64, bytes: &[u8]) -> Result<()> {
        let (s, n) = (start as usize, rows as usize);
        // Whole chunks (the writers copy chunk by chunk): direct, compressed in parallel
        if self.chunk_rows > 0 && s % self.chunk_rows == 0 && (n == self.chunk_rows || s + n == self.rows) {
            return self.write_chunk(s, bytes);
        }
        let mut slab: Vec<SliceOrIndex> = vec![SliceOrIndex::from(start as usize..(start + rows) as usize)];
        slab.extend(self.rest.iter().map(|&d| SliceOrIndex::from(0..d)));
        let shape: Vec<usize> = std::iter::once(rows as usize).chain(self.rest.iter().copied()).collect();
        macro_rules! write {
            ($t:ty, $n:expr) => {{
                let values: Vec<$t> = bytes.chunks_exact($n).map(|b| <$t>::from_le_bytes(b.try_into().unwrap())).collect();
                let view = ndarray::ArrayViewD::from_shape(shape.clone(), &values).map_err(|e| err(&self.path, e))?;
                self.ds.write_slice(view, Hyperslab::from(slab.clone())).map_err(|e| err(&self.path, e))
            }};
        }
        match self.ty {
            SampleType::I8 => write!(i8, 1),
            SampleType::I16 => write!(i16, 2),
            SampleType::U16 => write!(u16, 2),
            SampleType::I32 => write!(i32, 4),
            SampleType::I64 => write!(i64, 8),
            SampleType::F32 => write!(f32, 4),
            SampleType::F64 => write!(f64, 8),
        }
    }
}

/// Reads arrays back for the integrity check. Chunked datasets are read chunk by chunk straight
/// from the file (HDF5 only says where each chunk is) and inflated in the caller's thread, so
/// blocks verify in parallel; others go through HDF5.
pub struct Hdf5Reader {
    file: h5::File,
    raw: std::fs::File,
}

impl Hdf5Reader {
    pub fn open(path: &Path) -> Result<Self> {
        let file = h5::File::open(path).map_err(|e| err(&path.display().to_string(), e))?;
        let raw = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
        Ok(Self { file, raw })
    }

    /// Rows `r0..r1` of a chunked dataset from its stored chunks (`None`: not chunked).
    fn chunked_rows(&self, ds: &h5::Dataset, path: &str, ty: SampleType, r0: u64, r1: u64) -> Result<Option<Vec<u8>>> {
        use std::os::unix::fs::FileExt;
        let Some(chunk) = ds.chunk() else { return Ok(None) };
        let shape = ds.shape();
        // Only chunks spanning every dimension after the first (as written here)
        if chunk[1..] != shape[1..] {
            return Ok(None);
        }
        let deflate = ds.filters().iter().any(|f| matches!(f, h5::filters::Filter::Deflate(_)));
        let row_bytes = shape[1..].iter().product::<usize>() * ty.bytes();
        let cr = chunk[0] as u64;
        let mut out = Vec::with_capacity((r1 - r0) as usize * row_bytes);
        let id = ds.id();
        for c in r0 / cr..r1.div_ceil(cr) {
            let offset: Vec<u64> = std::iter::once(c * cr).chain(shape[1..].iter().map(|_| 0)).collect();
            let (mut mask, mut addr, mut size) = (0u32, 0u64, 0u64);
            let status = h5::sync::sync(|| unsafe { hdf5_metno_sys::h5d::H5Dget_chunk_info_by_coord(id, offset.as_ptr(), &mut mask, &mut addr, &mut size) });
            if status < 0 {
                return Err(err(path, format!("no chunk information for row {}", c * cr)));
            }
            let full = cr as usize * row_bytes;
            let data = if addr == u64::MAX || size == 0 {
                vec![0u8; full]
            } else {
                let mut stored = vec![0u8; size as usize];
                self.raw.read_exact_at(&mut stored, addr).map_err(|e| err(path, e))?;
                if deflate && mask & 1 == 0 {
                    use std::io::Read;
                    let mut d = Vec::with_capacity(full);
                    flate2::read::ZlibDecoder::new(&stored[..]).read_to_end(&mut d).map_err(|e| err(path, e))?;
                    d
                } else {
                    stored
                }
            };
            // The rows of this chunk inside r0..r1
            let (a, b) = (r0.max(c * cr) - c * cr, r1.min((c + 1) * cr) - c * cr);
            let slice = data.get(a as usize * row_bytes..b as usize * row_bytes).ok_or_else(|| err(path, "chunk shorter than expected"))?;
            out.extend_from_slice(slice);
        }
        Ok(Some(out))
    }

    /// (dtype name, shape) of the dataset at `path`.
    pub fn describe(&self, path: &str) -> Result<(String, Vec<u64>)> {
        let ds = self.file.dataset(path).map_err(|e| err(path, e))?;
        let d = ds.dtype().and_then(|t| t.to_descriptor()).map_err(|e| err(path, e))?;
        use h5::types::{FloatSize, IntSize, TypeDescriptor as T};
        let name = match d {
            T::Integer(IntSize::U1) => "int8",
            T::Integer(IntSize::U2) => "int16",
            T::Integer(IntSize::U4) => "int32",
            T::Integer(IntSize::U8) => "int64",
            T::Unsigned(IntSize::U2) => "uint16",
            T::Float(FloatSize::U4) => "float32",
            T::Float(FloatSize::U8) => "float64",
            _ => "?",
        };
        Ok((name.to_string(), ds.shape().iter().map(|&s| s as u64).collect()))
    }

    /// Little-endian bytes of rows `r0..r1` of `path`, read as `ty`.
    pub fn rows(&self, path: &str, ty: SampleType, r0: u64, r1: u64) -> Result<Vec<u8>> {
        let ds = self.file.dataset(path).map_err(|e| err(path, e))?;
        if let Some(bytes) = self.chunked_rows(&ds, path, ty, r0, r1)? {
            return Ok(bytes);
        }
        let shape = ds.shape();
        let mut slab: Vec<SliceOrIndex> = vec![SliceOrIndex::from(r0 as usize..r1 as usize)];
        slab.extend(shape[1..].iter().map(|&d| SliceOrIndex::from(0..d)));
        macro_rules! read {
            ($t:ty) => {{
                let a: ndarray::ArrayD<$t> = ds.read_slice(Hyperslab::from(slab)).map_err(|e| err(path, e))?;
                a.iter().flat_map(|v| v.to_le_bytes()).collect()
            }};
        }
        Ok(match ty {
            SampleType::I8 => read!(i8),
            SampleType::I16 => read!(i16),
            SampleType::U16 => read!(u16),
            SampleType::I32 => read!(i32),
            SampleType::I64 => read!(i64),
            SampleType::F32 => read!(f32),
            SampleType::F64 => read!(f64),
        })
    }
}

/// Structural checks of an NWB/HDF5 file: the root is an `NWBFile`, the required datasets exist,
/// the cached specifications are referenced, and every link resolves. (The Zarr checks are more
/// thorough; read an HDF5 file back with pynwb for the rest.)
pub fn validate(path: &Path) -> Result<Vec<nc_core::Issue>> {
    use nc_core::Issue;
    let file = h5::File::open(path).map_err(|e| err(&path.display().to_string(), e))?;
    let mut issues = Vec::new();
    let kind = file.attr("neurodata_type").and_then(|a| a.read_scalar::<VarLenUnicode>()).map(|v| v.to_string());
    if kind.as_deref().ok() != Some("NWBFile") {
        issues.push(Issue::error("root is not an NWBFile".to_string()));
    }
    for required in ["session_description", "identifier", "session_start_time", "timestamps_reference_time", "file_create_date"] {
        if file.dataset(required).is_err() {
            issues.push(Issue::error(format!("/{required} is missing")));
        }
    }
    let spec = file.attr(".specloc").and_then(|a| a.read_scalar::<h5::ObjectReference1>()).and_then(|r| file.dereference(&r));
    if spec.is_err() || file.group("specifications").is_err() {
        issues.push(Issue::warning("no cached specifications (/specifications)".to_string()));
    }
    // Every soft link under the electrode groups and devices resolves
    for group in ["general/extracellular_ephys", "general/devices"] {
        let Ok(g) = file.group(group) else { continue };
        for name in g.member_names().unwrap_or_default() {
            let Ok(sub) = g.group(&name) else { continue };
            for link in sub.member_names().unwrap_or_default() {
                if sub.group(&link).is_err() && sub.dataset(&link).is_err() {
                    issues.push(Issue::error(format!("/{group}/{name}/{link} does not resolve")));
                }
            }
        }
    }
    Ok(issues)
}
