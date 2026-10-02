//! The NumPy `.npy` files Open Ephys writes next to its data: 1-D little-endian int64 / float64 /
//! int16 / uint64 arrays and fixed-width byte strings. Only what those files need: version 1–3
//! headers, C order, no pickles.

use std::path::Path;

use nc_base::{Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Npy {
    /// `descr` of the header, e.g. `<i8`, `<f8`, `|S513`.
    pub descr: String,
    /// Elements (first dimension; Open Ephys arrays are 1-D, or N × 1).
    pub len: usize,
    /// Bytes per element.
    pub item: usize,
    data: Vec<u8>,
}

fn err(path: &Path, msg: impl Into<String>) -> Error {
    Error::format("npy", format!("{}: {}", path.display(), msg.into()))
}

impl Npy {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
        Self::parse(&bytes).map_err(|m| err(path, m))
    }

    pub fn parse(bytes: &[u8]) -> std::result::Result<Self, String> {
        if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
            return Err("not a .npy file".into());
        }
        let (len_bytes, start) = if bytes[6] == 1 { (2, 8) } else { (4, 8) };
        let header_len = if len_bytes == 2 { u16::from_le_bytes([bytes[8], bytes[9]]) as usize } else { u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize };
        let header_start = start + len_bytes;
        let header = std::str::from_utf8(bytes.get(header_start..header_start + header_len).ok_or("header ends early")?).map_err(|_| "header is not text")?;
        let field = |key: &str| -> Option<&str> {
            let at = header.find(&format!("'{key}'"))? + key.len() + 2;
            let rest = header[at..].trim_start().strip_prefix(':')?.trim_start();
            Some(rest)
        };
        let descr = field("descr").and_then(|r| r.strip_prefix('\'')).and_then(|r| r.split('\'').next()).ok_or("no descr")?.to_string();
        if field("fortran_order").is_some_and(|r| r.starts_with("True")) {
            return Err("Fortran order is not supported".into());
        }
        let shape = field("shape").and_then(|r| r.strip_prefix('(')).and_then(|r| r.split(')').next()).ok_or("no shape")?;
        let dims: Vec<usize> = shape.split(',').map(str::trim).filter(|s| !s.is_empty()).map(|s| s.parse().map_err(|_| format!("bad shape {shape:?}"))).collect::<std::result::Result<_, _>>()?;
        let len = dims.first().copied().unwrap_or(1);
        let item_count: usize = dims.iter().product::<usize>().max(if dims.is_empty() { 1 } else { 0 });
        let item = descr[2..].parse::<usize>().map_err(|_| format!("unsupported dtype {descr:?}"))?;
        if descr.starts_with('>') {
            return Err(format!("big-endian dtype {descr:?}"));
        }
        let data = bytes[header_start + header_len..].to_vec();
        if data.len() < item_count * item {
            return Err(format!("{} bytes of data for {item_count} × {item}", data.len()));
        }
        // N × 1 arrays read as N elements
        let item = if dims.len() == 2 { item * dims[1] } else { item };
        Ok(Self { descr, len, item, data })
    }

    fn element(&self, i: usize) -> &[u8] {
        &self.data[i * self.item..(i + 1) * self.item]
    }

    /// Integer values (`i1`–`i8`, `u1`–`u8`) as i64.
    pub fn ints(&self) -> std::result::Result<Vec<i64>, String> {
        let kind = &self.descr[1..2];
        if kind != "i" && kind != "u" {
            return Err(format!("expected integers, found {}", self.descr));
        }
        let size: usize = self.descr[2..].parse().unwrap_or(0);
        Ok((0..self.len)
            .map(|i| {
                let b = &self.element(i)[..size];
                let mut buf = [0u8; 8];
                buf[..size].copy_from_slice(b);
                // Sign-extend signed types
                if kind == "i" && b[size - 1] & 0x80 != 0 {
                    buf[size..].fill(0xFF);
                }
                i64::from_le_bytes(buf)
            })
            .collect())
    }

    /// Float values (`f4`, `f8`).
    pub fn floats(&self) -> std::result::Result<Vec<f64>, String> {
        match &self.descr[1..] {
            "f8" => Ok((0..self.len).map(|i| f64::from_le_bytes(self.element(i).try_into().unwrap())).collect()),
            "f4" => Ok((0..self.len).map(|i| f32::from_le_bytes(self.element(i).try_into().unwrap()) as f64).collect()),
            d => Err(format!("expected floats, found {d}")),
        }
    }

    /// Fixed-width byte strings (`|S…`), trailing NULs removed.
    pub fn strings(&self) -> std::result::Result<Vec<String>, String> {
        if !self.descr.contains('S') {
            return Err(format!("expected byte strings, found {}", self.descr));
        }
        Ok((0..self.len).map(|i| String::from_utf8_lossy(self.element(i)).trim_end_matches('\0').to_string()).collect())
    }
}

#[cfg(test)]
pub mod tests {
    use super::Npy;

    /// A version 1 `.npy` image.
    pub fn npy(descr: &str, shape: &str, data: &[u8]) -> Vec<u8> {
        let mut header = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': ({shape}), }}");
        while (10 + header.len() + 1) % 64 != 0 {
            header.push(' ');
        }
        header.push('\n');
        let mut b = b"\x93NUMPY\x01\x00".to_vec();
        b.extend((header.len() as u16).to_le_bytes());
        b.extend(header.as_bytes());
        b.extend(data);
        b
    }

    #[test]
    fn test_parse() {
        let ints: Vec<u8> = [5i64, -3, 7].iter().flat_map(|v| v.to_le_bytes()).collect();
        let a = Npy::parse(&npy("<i8", "3,", &ints)).unwrap();
        assert_eq!(a.ints().unwrap(), vec![5, -3, 7]);
        assert!(a.floats().is_err());
        let states: Vec<u8> = [-1i16, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(Npy::parse(&npy("<i2", "2,", &states)).unwrap().ints().unwrap(), vec![-1, 2]);
        let f: Vec<u8> = [0.5f64].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(Npy::parse(&npy("<f8", "1,", &f)).unwrap().floats().unwrap(), vec![0.5]);
        assert_eq!(Npy::parse(&npy("|S4", "2,", b"ab\0\0cdef")).unwrap().strings().unwrap(), vec!["ab", "cdef"]);
        assert_eq!(Npy::parse(&npy("<i8", "0,", &[])).unwrap().len, 0);
        assert!(Npy::parse(b"nope").is_err());
    }
}
