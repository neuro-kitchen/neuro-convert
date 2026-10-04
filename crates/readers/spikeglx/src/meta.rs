//! The `.meta` sidecar: `key=value` lines; keys starting with `~` hold tables
//! (`~imroTbl`, `~snsChanMap`, `~snsShankMap`, `~snsGeomMap`) and are stored without the `~`.
//!
//! References: SpikeGLX metadata help and `readSGLX.py` (SpikeGLX_Datafile_Tools).

use std::collections::BTreeMap;
use std::path::Path;

use nc_base::{Error, Result};

/// Which acquisition stream a file belongs to (`typeThis`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    /// Neuropixels probe (AP and/or LF band + sync).
    Imec,
    /// National Instruments DAQ (MN, MA, XA analog + DW digital words).
    Nidq,
    /// OneBox analog / digital (not read yet).
    Obx,
}

/// A parsed `.meta` file: `key=value` lines (`~` of table keys dropped).
#[derive(Debug, Clone, Default)]
pub struct Meta {
    /// Every key and its raw value.
    pub fields: BTreeMap<String, String>,
}

impl Meta {
    /// Parses `.meta` text.
    pub fn parse(text: &str) -> Self {
        let fields = text
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim().trim_start_matches('~').to_string(), v.trim().to_string()))
            .collect();
        Self { fields }
    }

    /// Reads and parses the file at `path`; fails when it has no `nSavedChans`.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        let meta = Self::parse(&text);
        if !meta.fields.contains_key("nSavedChans") {
            return Err(Error::format("spikeglx", format!("{}: not a SpikeGLX .meta (no nSavedChans)", path.display())));
        }
        Ok(meta)
    }

    /// Value of `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    /// Value of `key` as a number.
    pub fn f64(&self, key: &str) -> Option<f64> {
        self.get(key)?.parse().ok()
    }

    fn require(&self, key: &str) -> Result<&str> {
        self.get(key).ok_or_else(|| Error::format("spikeglx", format!("meta has no {key}")))
    }

    fn require_f64(&self, key: &str) -> Result<f64> {
        self.require(key)?.parse().map_err(|_| Error::format("spikeglx", format!("meta {key} is not a number")))
    }

    /// `typeThis`: imec, nidq or obx.
    pub fn stream_type(&self) -> Result<StreamType> {
        match self.require("typeThis")? {
            "imec" => Ok(StreamType::Imec),
            "nidq" => Ok(StreamType::Nidq),
            "obx" => Ok(StreamType::Obx),
            other => Err(Error::Unsupported(format!("SpikeGLX stream type {other:?}"))),
        }
    }

    /// Sample rate of this stream (`imSampRate`, `niSampRate`, `obSampRate`).
    pub fn sample_rate(&self) -> Result<f64> {
        match self.stream_type()? {
            StreamType::Imec => self.require_f64("imSampRate"),
            StreamType::Nidq => self.require_f64("niSampRate"),
            StreamType::Obx => self.require_f64("obSampRate"),
        }
    }

    /// `nSavedChans`: columns per sample in the `.bin`.
    pub fn saved_channels(&self) -> Result<usize> {
        self.require("nSavedChans")?.parse().map_err(|_| Error::format("spikeglx", "nSavedChans is not a number"))
    }

    /// Original (acquisition) index of every saved channel, in file column order
    /// (`snsSaveChanSubset`: `all` or comma-separated indices and inclusive `a:b` ranges).
    pub fn original_channels(&self) -> Result<Vec<usize>> {
        let n = self.saved_channels()?;
        let subset = self.get("snsSaveChanSubset").unwrap_or("all");
        if subset == "all" {
            return Ok((0..n).collect());
        }
        let bad = || Error::format("spikeglx", format!("snsSaveChanSubset {subset:?} is malformed"));
        let mut out = Vec::new();
        for part in subset.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once(':') {
                Some((a, b)) => {
                    let (a, b): (usize, usize) = (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?);
                    out.extend(a..=b);
                }
                None => out.push(part.parse().map_err(|_| bad())?),
            }
        }
        if out.len() != n {
            return Err(Error::format("spikeglx", format!("snsSaveChanSubset lists {} channels, nSavedChans is {n}", out.len())));
        }
        Ok(out)
    }

    /// Comma-separated integer counts (`snsApLfSy`, `acqApLfSy`, `snsMnMaXaDw`, …).
    pub fn counts(&self, key: &str) -> Result<Vec<usize>> {
        let v = self.require(key)?;
        v.split(',').map(|x| x.trim().parse().map_err(|_| Error::format("spikeglx", format!("{key} {v:?} is malformed")))).collect()
    }

    /// Parenthesized table `(header)(entry)(entry)…` → header fields and entry fields, split on
    /// `sep_header` / `sep_entry` (e.g. `,` and ` ` for `imroTbl`, `,` and `:` for maps).
    pub fn table(&self, key: &str, sep_header: char, sep_entry: char) -> Option<(Vec<String>, Vec<Vec<String>>)> {
        let text = self.get(key)?;
        let mut groups = text.split(')').map(|g| g.trim_start_matches('(')).filter(|g| !g.is_empty());
        let header = groups.next()?.split(sep_header).map(|s| s.trim().to_string()).collect();
        let entries = groups.map(|g| g.split(sep_entry).map(|s| s.trim().to_string()).collect()).collect();
        Some((header, entries))
    }

    /// Volts per ADC unit before channel gain: `imAiRangeMax / imMaxInt` (512 when absent, as
    /// for 3A / NP1.0) or `niAiRangeMax / niMaxInt` (32768 when absent).
    pub fn volts_per_bit(&self) -> Result<f64> {
        let (range, max_int) = match self.stream_type()? {
            StreamType::Imec => (self.require_f64("imAiRangeMax")?, self.f64("imMaxInt").unwrap_or(512.0)),
            StreamType::Nidq => (self.require_f64("niAiRangeMax")?, self.f64("niMaxInt").unwrap_or(32768.0)),
            StreamType::Obx => (self.require_f64("obAiRangeMax")?, self.f64("obMaxInt").unwrap_or(32768.0)),
        };
        Ok(range / max_int)
    }

    /// Probe type: `imDatPrb_type` (e.g. 0 = NP1.0, 21 / 24 = NP2.0); `None` for 3A
    /// (`imProbeOpt` instead).
    pub fn probe_type(&self) -> Option<u32> {
        self.get("imDatPrb_type")?.parse().ok()
    }

    /// Phase 3A metadata (no `imDatPrb_type`, probe option in `imProbeOpt`).
    pub fn is_3a(&self) -> bool {
        self.get("imDatPrb_type").is_none() && self.get("imProbeOpt").is_some()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const NP1_3A: &str = "acqApLfSy=4,4,1\nappVersion=20180829\nfileCreateTime=2019-05-07T17:24:02\nfileSizeBytes=50\n\
fileTimeSecs=0.0001\nfirstSample=84149244\nimAiRangeMax=0.6\nimAiRangeMin=-0.6\nimProbeOpt=3\nimProbeSN=641251510\n\
imSampRate=30000\nnSavedChans=5\nsnsApLfSy=4,0,1\nsnsSaveChanSubset=0:3,8\ntypeThis=imec\n\
~imroTbl=(641251510,3,4)(0 0 0 500 250)(1 0 0 500 250)(2 0 0 250 250)(3 0 0 500 250)\n\
~snsChanMap=(4,4,1)(AP0;0:0)(AP1;1:1)(AP2;2:2)(AP3;3:3)(SY0;8:8)\n\
~snsShankMap=(1,2,480)(0:0:0:1)(0:1:0:1)(0:0:1:1)(0:1:1:0)\n";

    #[test]
    fn test_parse_meta() {
        let m = Meta::parse(NP1_3A);
        assert_eq!(m.stream_type().unwrap(), StreamType::Imec);
        assert_eq!(m.sample_rate().unwrap(), 30000.0);
        assert_eq!(m.original_channels().unwrap(), vec![0, 1, 2, 3, 8]);
        assert_eq!(m.counts("snsApLfSy").unwrap(), vec![4, 0, 1]);
        assert!(m.is_3a() && m.probe_type().is_none());
        assert_eq!(m.volts_per_bit().unwrap(), 0.6 / 512.0);
        let (head, rows) = m.table("imroTbl", ',', ' ').unwrap();
        assert_eq!((head.len(), rows.len(), rows[2][3].as_str()), (3, 4, "250"));
        let (head, rows) = m.table("snsShankMap", ',', ':').unwrap();
        assert_eq!((head, rows[3].clone()), (vec!["1".into(), "2".into(), "480".into()], vec!["0".into(), "1".into(), "1".into(), "0".into()]));
    }
}
