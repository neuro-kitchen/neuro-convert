//! Neuropixels probe description from imec metadata: channel gains (IMRO table) and site
//! positions (geometry map).
//!
//! - Gains: NP1.0-family probes (3A and types 0, 1020, 1030, 1100, 1120–1123, 1200, 1300) set AP
//!   and LF gain per channel in `~imroTbl` (entry fields 3 and 4). Newer metadata states
//!   `imChan0apGain` / `imChan0lfGain`; NP2.0 types 21 / 24 use a fixed AP gain of 80 and type
//!   2013 of 100 (as `readSGLX.py`). Anything else has no known gain.
//! - Positions: `~snsGeomMap` `(part,shanks,shank pitch,shank width)(shank:x:z:used)…` in µm
//!   (x within its shank). Older files only have `~snsShankMap` `(shanks,cols,rows)(shank:col:
//!   row:used)…`, turned into µm with the probe's site layout (SGLXMetaToCoords):
//!   NP1.0 x = col·32 + (27 on even rows, 11 on odd rows), z = row·20; NP2.0 x = col·32 + 27,
//!   z = row·15, shanks 250 µm apart.

use crate::meta::Meta;

/// NP1.0-family probe types whose IMRO entries carry per-channel AP / LF gains.
const NP1_TYPES: [u32; 10] = [0, 1020, 1030, 1100, 1120, 1121, 1122, 1123, 1200, 1300];

/// Gains of a probe's channels, indexed by probe channel (0-based, as in the IMRO table).
#[derive(Debug, Clone, PartialEq)]
pub enum Gains {
    PerChannel { ap: Vec<f64>, lf: Vec<f64> },
    Uniform { ap: f64, lf: Option<f64> },
    Unknown,
}

impl Gains {
    pub fn from_meta(meta: &Meta) -> Self {
        let np1 = meta.is_3a() || meta.probe_type().is_some_and(|t| NP1_TYPES.contains(&t));
        if np1 && let Some((_, rows)) = meta.table("imroTbl", ',', ' ') {
            let field = |i: usize| rows.iter().map(|r| r.get(i).and_then(|v| v.parse::<f64>().ok())).collect::<Option<Vec<f64>>>();
            if let (Some(ap), Some(lf)) = (field(3), field(4)) {
                return Gains::PerChannel { ap, lf };
            }
        }
        if let Some(ap) = meta.f64("imChan0apGain") {
            return Gains::Uniform { ap, lf: meta.f64("imChan0lfGain").filter(|g| *g > 0.0) };
        }
        match meta.probe_type() {
            Some(21 | 24) => Gains::Uniform { ap: 80.0, lf: None },
            Some(2013) => Gains::Uniform { ap: 100.0, lf: None },
            _ => Gains::Unknown,
        }
    }

    /// Gain of probe channel `chan` in the AP (`lf == false`) or LF band.
    pub fn get(&self, chan: usize, lf: bool) -> Option<f64> {
        let g = match self {
            Gains::PerChannel { ap, lf: l } => if lf { l.get(chan).copied() } else { ap.get(chan).copied() },
            Gains::Uniform { ap, lf: l } => if lf { *l } else { Some(*ap) },
            Gains::Unknown => None,
        };
        g.filter(|g| *g > 0.0)
    }
}

/// One site of the geometry map, in file order of the neural channels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Site {
    pub shank: usize,
    /// µm, across the probe (shank offset included).
    pub x: f32,
    /// µm, along the shank from the tip.
    pub z: f32,
    /// `false` for reference / disconnected sites.
    pub used: bool,
}

/// Site layout used to place `~snsShankMap` columns and rows.
struct Layout {
    even_x: f32,
    odd_x: f32,
    col_pitch: f32,
    row_pitch: f32,
    shank_pitch: f32,
}

const NP1_LAYOUT: Layout = Layout { even_x: 27.0, odd_x: 11.0, col_pitch: 32.0, row_pitch: 20.0, shank_pitch: 0.0 };
const NP2_LAYOUT: Layout = Layout { even_x: 27.0, odd_x: 27.0, col_pitch: 32.0, row_pitch: 15.0, shank_pitch: 250.0 };

/// Sites of the file's neural channels, or `None` when the metadata cannot place them.
pub fn sites(meta: &Meta) -> Option<Vec<Site>> {
    let num = |s: &str| s.parse::<f32>().ok();
    if let Some((head, rows)) = meta.table("snsGeomMap", ',', ':') {
        let pitch = head.get(2).and_then(|v| num(v))?;
        return rows
            .iter()
            .map(|r| {
                let shank = r.first()?.parse::<usize>().ok()?;
                Some(Site { shank, x: shank as f32 * pitch + num(r.get(1)?)?, z: num(r.get(2)?)?, used: r.get(3)? != "0" })
            })
            .collect();
    }
    let layout = if meta.is_3a() || meta.probe_type().is_some_and(|t| NP1_TYPES.contains(&t)) {
        NP1_LAYOUT
    } else if matches!(meta.probe_type(), Some(21 | 24)) {
        NP2_LAYOUT
    } else {
        return None;
    };
    let (_, rows) = meta.table("snsShankMap", ',', ':')?;
    rows.iter()
        .map(|r| {
            let (shank, col, row) = (r.first()?.parse::<usize>().ok()?, num(r.get(1)?)?, r.get(2)?.parse::<u32>().ok()?);
            let offset = if row % 2 == 0 { layout.even_x } else { layout.odd_x };
            Some(Site {
                shank,
                x: shank as f32 * layout.shank_pitch + col * layout.col_pitch + offset,
                z: row as f32 * layout.row_pitch,
                used: r.get(3)? != "0",
            })
        })
        .collect()
}

/// Human-readable probe model: `imDatPrb_pn` (e.g. `NP1010`), else `3A` / `type <n>`.
pub fn model(meta: &Meta) -> String {
    if let Some(pn) = meta.get("imDatPrb_pn") {
        return pn.to_string();
    }
    match (meta.is_3a(), meta.probe_type()) {
        (true, _) => format!("3A (option {})", meta.get("imProbeOpt").unwrap_or("?")),
        (_, Some(t)) => format!("type {t}"),
        _ => "unknown".into(),
    }
}

/// Probe serial number (`imDatPrb_sn`, or `imProbeSN` for 3A).
pub fn serial(meta: &Meta) -> Option<&str> {
    meta.get("imDatPrb_sn").or_else(|| meta.get("imProbeSN"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::tests::NP1_3A;

    #[test]
    fn test_np1_gains_and_shank_map() {
        let m = Meta::parse(NP1_3A);
        let g = Gains::from_meta(&m);
        assert_eq!((g.get(0, false), g.get(2, false), g.get(2, true), g.get(9, false)), (Some(500.0), Some(250.0), Some(250.0), None));
        let s = sites(&m).unwrap();
        // (col,row): (0,0) (1,0) (0,1) (1,1, unused)
        let xz: Vec<(f32, f32)> = s.iter().map(|s| (s.x, s.z)).collect();
        assert_eq!(xz, vec![(27.0, 0.0), (59.0, 0.0), (11.0, 20.0), (43.0, 20.0)]);
        assert!(!s[3].used);
        assert_eq!(model(&m), "3A (option 3)");
    }

    #[test]
    fn test_geom_map_and_np2_gain() {
        let m = Meta::parse("typeThis=imec\nnSavedChans=3\nimDatPrb_type=24\n~snsGeomMap=(NP2010,4,250,70)(0:27:0:1)(2:59:15:1)\n");
        let s = sites(&m).unwrap();
        assert_eq!((s[1].shank, s[1].x, s[1].z), (2, 559.0, 15.0));
        assert_eq!(Gains::from_meta(&m), Gains::Uniform { ap: 80.0, lf: None });
        assert_eq!(Gains::from_meta(&Meta::parse("imDatPrb_type=9999\n")), Gains::Unknown);
    }
}
