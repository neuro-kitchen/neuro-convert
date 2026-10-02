//! Content verification: proves that every array with sample data in a written store holds
//! exactly what the reader produced.
//!
//! [`nwb::validate`](crate::validate) only checks structure. Here each array is compared in
//! fixed-size row blocks: the **source side** is read again through the session's readers
//! (`Recording::read_stored` / `read`, in-memory events and snippets) and laid out as the
//! array's dtype, row-major, little-endian; the **store side** is read back through `zarrs`. Both
//! are hashed with xxh3-64 per block. Neither side uses the writer's chunking or copy code, so a
//! chunk written to the wrong place, a transposition or a wrong dtype shows up as a mismatch.
//!
//! The digests ([`Digests`]) go into the conversion report, so a copy of the store can be
//! re-checked later without the source ([`recheck`]).

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;
use zarrs::array::{Array, ArraySubset};
use zarrs::filesystem::FilesystemStore;
use zarrs::storage::ReadableStorageTraits;

use crate::mapping::NwbPlan;
use crate::types::series::{regular_rate, Storage};
use crate::Progress;
use nc_core::{Error, Issue, Recording, Result, SampleType, Session, SnippetSeries};

/// Hash algorithm of the digests (recorded in reports).
pub const ALGORITHM: &str = "xxh3-64";

/// Target bytes per compared block (the block length in rows follows each array's row size).
pub const BLOCK_BYTES: u64 = 8 << 20;

/// Blocks compared per array besides the first and the last at [`VerifyLevel::Sampled`].
pub const SAMPLED_BLOCKS: usize = 8;

/// How much of the written data is compared with the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyLevel {
    /// Structure only (no sample data read back).
    Off,
    /// First, last and [`SAMPLED_BLOCKS`] random blocks of every array.
    Sampled,
    /// Every block (about one more pass over the source and the output).
    #[default]
    Full,
}

impl std::str::FromStr for VerifyLevel {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "off" => Ok(Self::Off),
            "sampled" => Ok(Self::Sampled),
            "full" => Ok(Self::Full),
            _ => Err(format!("verify must be full, sampled or off, got {s:?}")),
        }
    }
}

/// Digests of every verified array, as saved in the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Digests {
    pub algorithm: String,
    pub level: VerifyLevel,
    pub arrays: Vec<ArrayDigest>,
}

impl Digests {
    pub fn mismatched(&self) -> usize {
        self.arrays.iter().filter(|a| !a.mismatched.is_empty()).count()
    }

    /// `verified 12 arrays (full): digests match` or the number that differ.
    pub fn summary(&self) -> String {
        let level = match self.level {
            VerifyLevel::Off => "off",
            VerifyLevel::Sampled => "sampled",
            VerifyLevel::Full => "full",
        };
        match self.mismatched() {
            0 => format!("verified {} arrays ({level}): content matches the source", self.arrays.len()),
            n => format!("verified {} arrays ({level}): {n} differ from the source", self.arrays.len()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArrayDigest {
    /// Array path in the store (`/acquisition/HDEG/data`).
    pub path: String,
    /// Store dtype (`int16`, `float32`, …).
    pub dtype: String,
    pub shape: Vec<u64>,
    /// Rows (first dimension) per block; the last block may be shorter.
    pub block_rows: u64,
    pub blocks: u64,
    /// Indices of the compared blocks; `None` = all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<Vec<u64>>,
    /// Hex digest of each compared block (source side), in `checked` order.
    pub digests: Vec<String>,
    /// Digest over all block digests (only when every block was compared).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    /// Blocks whose store content differs from the source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mismatched: Vec<u64>,
    /// Sample rate of a continuous series (to report mismatches in seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
}

impl ArrayDigest {
    fn block_indices(&self) -> Vec<u64> {
        self.checked.clone().unwrap_or_else(|| (0..self.blocks).collect())
    }

    fn issue(&self, what: &str) -> Option<Issue> {
        if self.mismatched.is_empty() {
            return None;
        }
        let shown: Vec<String> = self.mismatched.iter().take(5).map(|&k| {
            let (r0, r1) = (k * self.block_rows, ((k + 1) * self.block_rows).min(self.shape[0]));
            match self.rate {
                Some(rate) if rate > 0.0 => format!("{:.3}–{:.3} s", r0 as f64 / rate, r1 as f64 / rate),
                _ => format!("rows {r0}–{r1}"),
            }
        }).collect();
        let more = if self.mismatched.len() > 5 { format!(" and {} more", self.mismatched.len() - 5) } else { String::new() };
        Some(Issue::error(format!("{}: content differs from {what} in {} block(s): {}{more}", self.path, self.mismatched.len(), shown.join(", "))))
    }
}

/// Where the expected content of one array comes from.
enum Source<'a> {
    /// Rows `[time, channel]` from a recording, as stored (`native`) or scaled float32.
    Recording { rec: &'a dyn Recording, native: bool },
    /// Little-endian bytes of the whole array, row-major.
    Bytes(Vec<u8>),
    /// Float32 waveforms of these snippets, one per row (read from the source per block).
    Snippets { sn: &'a SnippetSeries, events: Vec<usize> },
}

struct Target<'a> {
    path: String,
    ty: SampleType,
    shape: Vec<u64>,
    /// Values per row (product of the dimensions after the first).
    row_values: u64,
    rate: Option<f64>,
    source: Source<'a>,
}

impl Target<'_> {
    fn rows(&self) -> u64 {
        self.shape.first().copied().unwrap_or(0)
    }

    fn block_rows(&self) -> u64 {
        (BLOCK_BYTES / (self.row_values.max(1) * self.ty.bytes() as u64)).max(1)
    }

    /// Expected bytes of rows `r0..r1`.
    fn expected(&self, r0: u64, r1: u64) -> Result<Vec<u8>> {
        let es = self.ty.bytes();
        match &self.source {
            Source::Bytes(b) => {
                let w = self.row_values as usize * es;
                Ok(b[r0 as usize * w..r1 as usize * w].to_vec())
            }
            Source::Snippets { sn, events } => Ok(f32_bytes(&sn.read(&events[r0 as usize..r1 as usize])?)),
            Source::Recording { rec, native } => {
                let c = self.row_values as usize;
                let n = (r1 - r0) as usize;
                let channels: Vec<usize> = (0..c).collect();
                // Channel-major from the reader …
                let mut src = vec![0u8; n * c * es];
                if *native {
                    if !rec.read_stored(&channels, r0..r1, &mut src)? {
                        return Err(Error::Unsupported(format!("{}: the source no longer serves stored samples", self.path)));
                    }
                } else {
                    let mut floats = vec![0f32; n * c];
                    rec.read(&channels, r0..r1, &mut floats)?;
                    for (d, v) in src.as_chunks_mut::<4>().0.iter_mut().zip(&floats) {
                        *d = v.to_le_bytes();
                    }
                }
                // … to the store's row-major [time, channel]
                let mut out = vec![0u8; src.len()];
                for (ch, column) in src.chunks_exact(n * es).enumerate() {
                    for (t, v) in column.chunks_exact(es).enumerate() {
                        let at = (t * c + ch) * es;
                        out[at..at + es].copy_from_slice(v);
                    }
                }
                Ok(out)
            }
        }
    }
}

fn f64_bytes(v: &[f64]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Every array with sample data that `plan` writes, with its expected content.
fn targets<'a>(session: &'a Session, plan: &NwbPlan) -> Vec<Target<'a>> {
    let mut out = Vec::new();
    for s in &plan.series {
        let rec = session.recordings[s.recording].as_ref();
        let info = rec.info();
        let electrical = s.electrodes.is_some();
        let serves_stored = rec.read_stored(&[], 0..0, &mut []).unwrap_or(false);
        let storage = Storage::choose(info, electrical, s.conversion, serves_stored);
        let cols = info.channel_count() as u64;
        let shape = if electrical || cols != 1 { vec![info.samples, cols] } else { vec![info.samples] };
        out.push(Target {
            path: format!("/acquisition/{}/data", s.name),
            ty: storage.ty,
            shape,
            row_values: cols,
            rate: Some(info.sample_rate),
            source: Source::Recording { rec, native: storage.native },
        });
    }
    for p in &plan.events {
        let e = &session.events[p.event];
        let n = e.len() as u64;
        let f64s = |path: String, shape: Vec<u64>, values: &[f64]| Target {
            row_values: shape[1..].iter().product(),
            path,
            ty: SampleType::F64,
            shape,
            rate: None,
            source: Source::Bytes(f64_bytes(values)),
        };
        if p.table {
            let base = format!("/events/{}", p.name);
            out.push(f64s(format!("{base}/timestamp"), vec![n], &e.onsets));
            if let Some(off) = &e.offsets {
                let durations: Vec<f64> = e.onsets.iter().zip(off).map(|(a, b)| b - a).collect();
                out.push(f64s(format!("{base}/duration"), vec![n], &durations));
            }
            out.push(f64s(format!("{base}/value"), vec![n], &e.values));
        } else {
            let base = format!("/acquisition/{}", p.name);
            let shape = if e.channels > 1 { vec![n, e.channels as u64] } else { vec![n] };
            out.push(f64s(format!("{base}/data"), shape, &e.values));
            if regular_rate(&e.onsets).is_none() {
                out.push(f64s(format!("{base}/timestamps"), vec![n], &e.onsets));
            }
        }
    }
    for p in &plan.snippets {
        let sn = &session.snippets[p.snippet];
        let w = sn.samples_per_snippet;
        for &(channel, _) in &p.rows {
            let events = sn.on_channel(channel);
            let base = format!("/acquisition/{}_ch{channel}", p.name);
            let times: Vec<f64> = events.iter().map(|&i| sn.timestamps[i]).collect();
            let k = events.len() as u64;
            out.push(Target { path: format!("{base}/data"), ty: SampleType::F32, shape: vec![k, 1, w as u64], row_values: w as u64, rate: None, source: Source::Snippets { sn, events } });
            out.push(Target { path: format!("{base}/timestamps"), ty: SampleType::F64, shape: vec![k], row_values: 1, rate: None, source: Source::Bytes(f64_bytes(&times)) });
        }
    }
    out.retain(|t| t.rows() > 0);
    out
}

/// An array opened for reading back.
enum StoredArray {
    Zarr(Box<Array<dyn ReadableStorageTraits>>),
    #[cfg(feature = "hdf5")]
    Hdf5 { shape: Vec<u64> },
}

impl StoredArray {
    fn shape(&self) -> &[u64] {
        match self {
            StoredArray::Zarr(a) => a.shape(),
            #[cfg(feature = "hdf5")]
            StoredArray::Hdf5 { shape } => shape,
        }
    }
}

/// Reads arrays back from a store (Zarr) or file (HDF5).
enum StoreReader {
    Zarr(Arc<dyn ReadableStorageTraits>),
    #[cfg(feature = "hdf5")]
    Hdf5(crate::backend::hdf5::Hdf5Reader),
}

impl StoreReader {
    fn open(path: &Path) -> Result<Self> {
        #[cfg(feature = "hdf5")]
        if crate::backend::Format::of(path) == crate::backend::Format::Hdf5 {
            return Ok(Self::Hdf5(crate::backend::hdf5::Hdf5Reader::open(path)?));
        }
        let store = FilesystemStore::new(path).map_err(|e| Error::format("nwb-zarr", e.to_string()))?;
        Ok(Self::Zarr(Arc::new(store)))
    }

    fn array(&self, path: &str) -> Result<StoredArray> {
        match self {
            Self::Zarr(store) => Array::open(store.clone(), path).map(|a| StoredArray::Zarr(Box::new(a))).map_err(|e| Error::format("nwb-zarr", format!("{path}: {e}"))),
            #[cfg(feature = "hdf5")]
            Self::Hdf5(r) => r.describe(path).map(|(_, shape)| StoredArray::Hdf5 { shape }),
        }
    }

    /// The array's dtype name (Zarr: `_DTYPE`) and shape.
    fn describe(&self, path: &str) -> Result<(String, Vec<u64>)> {
        match self {
            Self::Zarr(_) => match self.array(path)? {
                StoredArray::Zarr(a) => {
                    let dtype = a.attributes().get("_DTYPE").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                    Ok((dtype, a.shape().to_vec()))
                }
                #[cfg(feature = "hdf5")]
                StoredArray::Hdf5 { .. } => unreachable!("a Zarr store opens Zarr arrays"),
            },
            #[cfg(feature = "hdf5")]
            Self::Hdf5(r) => r.describe(path),
        }
    }

    /// Little-endian bytes of rows `r0..r1` of `path`, read as `ty`.
    fn rows(&self, array: &StoredArray, path: &str, ty: SampleType, r0: u64, r1: u64) -> Result<Vec<u8>> {
        let array = match (self, array) {
            (Self::Zarr(_), StoredArray::Zarr(a)) => a,
            #[cfg(feature = "hdf5")]
            (Self::Hdf5(r), _) => return r.rows(path, ty, r0, r1),
            #[cfg(feature = "hdf5")]
            _ => unreachable!("arrays come from their own store"),
        };
        let ranges: Vec<std::ops::Range<u64>> = std::iter::once(r0..r1).chain(array.shape()[1..].iter().map(|&d| 0..d)).collect();
        let subset = ArraySubset::new_with_ranges(&ranges);
        let err = |e: zarrs::array::ArrayError| Error::format("nwb-zarr", format!("{path}: {e}"));
        macro_rules! read {
            ($t:ty) => {
                array.retrieve_array_subset::<Vec<$t>>(&subset).map_err(err)?.iter().flat_map(|v| v.to_le_bytes()).collect()
            };
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

fn hex(h: u64) -> String {
    format!("{h:016x}")
}

/// Blocks to compare out of `blocks` at `level` (sorted, distinct).
fn choose_blocks(blocks: u64, level: VerifyLevel, seed: &mut u64) -> Option<Vec<u64>> {
    if level == VerifyLevel::Full || blocks <= SAMPLED_BLOCKS as u64 + 2 {
        return None;
    }
    let mut picked = vec![0, blocks - 1];
    while picked.len() < SAMPLED_BLOCKS + 2 {
        // splitmix64
        *seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = *seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        let k = (z ^ (z >> 31)) % blocks;
        if !picked.contains(&k) {
            picked.push(k);
        }
    }
    picked.sort_unstable();
    Some(picked)
}

/// A compared block: (array, slot in its `checked` list, block index, digest, equal).
type BlockResult = (usize, usize, u64, u64, bool);

/// What one comparison needs to know about its block.
struct Work {
    target: usize,
    /// Position in the target's `checked` list.
    slot: usize,
    block: u64,
}

/// Compares the store at `dest` with what `plan` was to write from `session`. Returns the
/// digests and one error per array whose content differs (or cannot be read). `progress` counts
/// values compared; `cancel` stops between blocks with [`Error::Cancelled`].
pub fn verify(
    session: &Session,
    plan: &NwbPlan,
    dest: &Path,
    level: VerifyLevel,
    threads: usize,
    cancel: Option<&AtomicBool>,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<(Digests, Vec<Issue>)> {
    let mut digests = Digests { algorithm: ALGORITHM.into(), level, arrays: Vec::new() };
    if level == VerifyLevel::Off {
        return Ok((digests, Vec::new()));
    }
    let started = Instant::now();
    let store = StoreReader::open(dest)?;
    let targets = targets(session, plan);
    let mut issues = Vec::new();
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);

    // Describe every array; shape or dtype mismatches are errors without reading data
    let mut arrays = Vec::new();
    let mut work = Vec::new();
    for (i, t) in targets.iter().enumerate() {
        let opened = store.array(&t.path).and_then(|a| store.describe(&t.path).map(|d| (a, d)));
        let (array, (dtype, shape)) = match opened {
            Ok(x) => x,
            Err(e) => {
                issues.push(Issue::error(format!("{}: cannot be read back ({e})", t.path)));
                arrays.push(None);
                continue;
            }
        };
        if shape != t.shape || dtype != t.ty.name() {
            issues.push(Issue::error(format!("{}: store has {dtype} {shape:?}, expected {} {:?}", t.path, t.ty.name(), t.shape)));
            arrays.push(None);
            continue;
        }
        let block_rows = t.block_rows();
        let blocks = t.rows().div_ceil(block_rows);
        let checked = choose_blocks(blocks, level, &mut seed);
        let indices = checked.clone().unwrap_or_else(|| (0..blocks).collect());
        work.extend(indices.iter().enumerate().map(|(slot, &block)| Work { target: i, slot, block }));
        digests.arrays.push(ArrayDigest {
            path: t.path.clone(),
            dtype,
            shape,
            block_rows,
            blocks,
            digests: vec![String::new(); indices.len()],
            checked,
            digest: None,
            mismatched: Vec::new(),
            rate: t.rate,
        });
        arrays.push(Some((array, digests.arrays.len() - 1)));
    }

    // Compare blocks in parallel
    let total: u64 = work.iter().map(|w| {
        let t = &targets[w.target];
        let br = t.block_rows();
        ((w.block + 1) * br).min(t.rows()).saturating_sub(w.block * br) * t.row_values
    }).sum();
    let done = AtomicU64::new(0);
    let next = AtomicU64::new(0);
    let failed: Mutex<Option<Error>> = Mutex::new(None);
    let results: Mutex<Vec<BlockResult>> = Mutex::new(Vec::with_capacity(work.len()));
    let finished = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !finished.load(Ordering::Relaxed) {
                progress(Progress { done: done.load(Ordering::Relaxed), total, elapsed: started.elapsed() });
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        });
        std::thread::scope(|workers| {
            for _ in 0..threads.max(1) {
                workers.spawn(|| loop {
                    let k = next.fetch_add(1, Ordering::Relaxed) as usize;
                    if k >= work.len() || failed.lock().unwrap().is_some() {
                        break;
                    }
                    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                        *failed.lock().unwrap() = Some(Error::Cancelled);
                        break;
                    }
                    let w = &work[k];
                    let t = &targets[w.target];
                    let (array, a) = arrays[w.target].as_ref().expect("only described arrays get work");
                    let br = t.block_rows();
                    let (r0, r1) = (w.block * br, ((w.block + 1) * br).min(t.rows()));
                    let res = t.expected(r0, r1).and_then(|exp| store.rows(array, &t.path, t.ty, r0, r1).map(|got| (xxh3_64(&exp), exp == got)));
                    match res {
                        Ok((h, same)) => {
                            results.lock().unwrap().push((*a, w.slot, w.block, h, same));
                            done.fetch_add((r1 - r0) * t.row_values, Ordering::Relaxed);
                        }
                        Err(e) => {
                            *failed.lock().unwrap() = Some(e);
                            break;
                        }
                    }
                });
            }
        });
        finished.store(true, Ordering::Relaxed);
    });
    if let Some(e) = failed.into_inner().unwrap() {
        return Err(e);
    }
    progress(Progress { done: total, total, elapsed: started.elapsed() });

    for (a, slot, block, h, same) in results.into_inner().unwrap() {
        let d = &mut digests.arrays[a];
        d.digests[slot] = hex(h);
        if !same {
            d.mismatched.push(block);
        }
    }
    for d in &mut digests.arrays {
        d.mismatched.sort_unstable();
        if d.checked.is_none() {
            d.digest = Some(combine(&d.digests));
        }
        issues.extend(d.issue("the source"));
    }
    Ok((digests, issues))
}

/// Digest over block digests (hex strings, in block order).
fn combine(blocks: &[String]) -> String {
    let bytes: Vec<u8> = blocks.iter().flat_map(|h| u64::from_str_radix(h, 16).unwrap_or(0).to_le_bytes()).collect();
    hex(xxh3_64(&bytes))
}

/// Re-checks a store against digests saved in a report (no source needed): every recorded block
/// is read back on `threads` threads and hashed. Returns the digests with `mismatched` filled,
/// and their issues.
pub fn recheck(dest: &Path, saved: &Digests, threads: usize) -> Result<(Digests, Vec<Issue>)> {
    if saved.algorithm != ALGORITHM {
        return Err(Error::Unsupported(format!("digest algorithm {:?} (this build uses {ALGORITHM})", saved.algorithm)));
    }
    let store = StoreReader::open(dest)?;
    let mut out = saved.clone();
    let mut issues = Vec::new();
    let mut arrays = Vec::new();
    let mut work = Vec::new();
    for (i, d) in out.arrays.iter_mut().enumerate() {
        d.mismatched.clear();
        let ty = SampleType::from_name(&d.dtype).ok_or_else(|| Error::Unsupported(format!("{}: dtype {}", d.path, d.dtype)))?;
        match store.array(&d.path) {
            Ok(a) if a.shape() == d.shape.as_slice() => {
                work.extend(d.block_indices().into_iter().enumerate().map(|(slot, block)| Work { target: i, slot, block }));
                arrays.push(Some((a, ty)));
            }
            Ok(a) => {
                issues.push(Issue::error(format!("{}: shape {:?}, the report says {:?}", d.path, a.shape(), d.shape)));
                arrays.push(None);
            }
            Err(e) => {
                issues.push(Issue::error(format!("{}: cannot be read ({e})", d.path)));
                arrays.push(None);
            }
        }
    }
    let next = AtomicU64::new(0);
    let failed: Mutex<Option<Error>> = Mutex::new(None);
    let bad: Mutex<Vec<(usize, u64)>> = Mutex::new(Vec::new());
    let saved_arrays = &out.arrays;
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| loop {
                let k = next.fetch_add(1, Ordering::Relaxed) as usize;
                if k >= work.len() || failed.lock().unwrap().is_some() {
                    break;
                }
                let w = &work[k];
                let d = &saved_arrays[w.target];
                let (array, ty) = arrays[w.target].as_ref().expect("only readable arrays get work");
                let (r0, r1) = (w.block * d.block_rows, ((w.block + 1) * d.block_rows).min(d.shape[0]));
                match store.rows(array, &d.path, *ty, r0, r1) {
                    Ok(bytes) => {
                        if d.digests.get(w.slot).map(String::as_str) != Some(hex(xxh3_64(&bytes)).as_str()) {
                            bad.lock().unwrap().push((w.target, w.block));
                        }
                    }
                    Err(e) => {
                        *failed.lock().unwrap() = Some(e);
                        break;
                    }
                }
            });
        }
    });
    if let Some(e) = failed.into_inner().unwrap() {
        return Err(e);
    }
    for (i, block) in bad.into_inner().unwrap() {
        out.arrays[i].mismatched.push(block);
    }
    for d in &mut out.arrays {
        d.mismatched.sort_unstable();
        issues.extend(d.issue("the report"));
    }
    Ok((out, issues))
}

#[cfg(test)]
mod tests {
    use super::{choose_blocks, VerifyLevel, SAMPLED_BLOCKS};

    #[test]
    fn test_choose_blocks() {
        let mut seed = 7;
        assert_eq!(choose_blocks(100, VerifyLevel::Full, &mut seed), None);
        assert_eq!(choose_blocks(5, VerifyLevel::Sampled, &mut seed), None, "few blocks: all");
        let picked = choose_blocks(100, VerifyLevel::Sampled, &mut seed).unwrap();
        assert_eq!(picked.len(), SAMPLED_BLOCKS + 2);
        assert_eq!((picked[0], *picked.last().unwrap()), (0, 99), "first and last always");
        assert!(picked.windows(2).all(|w| w[0] < w[1]), "sorted and distinct");
    }
}
