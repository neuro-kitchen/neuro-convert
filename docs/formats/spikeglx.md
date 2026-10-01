# SpikeGLX (Neuropixels imec probes, NI-DAQ)

Reader: `crates/readers/spikeglx` (`nc-spikeglx`). References: SpikeGLX metadata help,
`readSGLX.py` (SpikeGLX_Datafile_Tools), SGLXMetaToCoords, the IMRO table reference
(billkarsh.github.io/SpikeGLX/help/imroTables).

## Files
| File | Role | Module |
|---|---|---|
| `<run>_g<gate>_t<trig>.imec<N>.ap.bin` / `.lf.bin` | probe data: int16, all saved channels interleaved per sample | `bin.rs` |
| `….nidq.bin` | NI-DAQ data (MN, MA, XA analog, DW digital words) | `bin.rs` |
| `.meta` (one per `.bin`) | `key=value`; `~` tables: `imroTbl`, `snsChanMap`, `snsShankMap`, `snsGeomMap` | `meta.rs` |

- A path to a `.bin` / `.meta` opens every file of its run in that folder (e.g. AP + LF). A folder
  opens the run found in it and two levels below (run folder `<run>_g0/`, probe folders
  `<run>_g0_imec0/`); several runs need `--block <run>`.
- 3A writes `.imec.` (read as `imec0`). Renamed files (IBL: `imec_385_100s.ap.bin`) keep only
  the band; the stream type then comes from `typeThis`.

## Recordings
| Source | Recording | Kind / unit |
|---|---|---|
| imec AP channels | `<probe>.ap` | electrical, volts |
| imec LF channels | `<probe>.lf` | electrical, volts |
| imec sync (SY) | `<probe>.<band>.sync` | raw 16-bit word |
| nidq MN + MA + XA | `nidq` | volts |
| nidq DW | `nidq.digital` | raw 16-bit words |

Saved channel order follows `snsSaveChanSubset` (`all`, indices and `a:b` ranges); the counts
`snsApLfSy` / `snsMnMaXaDw` split the columns into types. Samples stay int16; each channel's gain
is its volts per bit:

- imec: `imAiRangeMax / imMaxInt / channel gain` (`imMaxInt` = 512 when absent: 3A / NP1.0).
  Gains: NP1.0 family (3A; types 0, 1020, 1030, 1100, 1120–1123, 1200, 1300) per channel from
  `imroTbl` entries (fields 3 = AP, 4 = LF); else `imChan0apGain` / `imChan0lfGain`; else 80
  (types 21, 24) or 100 (type 2013). Unknown gain → `a.u.` and `Calibration::Unknown`.
- nidq: `niAiRangeMax / niMaxInt / gain` (`niMaxInt` = 32768 when absent), gain `niMNGain` (MN),
  `niMAGain` (MA), 1 (XA). DW words are not scaled.

## Electrodes
One electrode per probe channel, shared by the AP and LF recordings (`Electrode::channels` holds
both), grouped per probe (`imec0`) or per shank (`imec0_shank<n>`), on device `imec0`
(manufacturer IMEC, model `imDatPrb_pn`, or `3A (option n)`). Positions (`rel_x`, `rel_y` in µm):
- `~snsGeomMap` `(part,shanks,shank pitch,width)(shank:x:z:used)…`: x + shank · pitch, z;
- older `~snsShankMap` `(shanks,cols,rows)(shank:col:row:used)…`: NP1.0 x = col·32 + 27 (even
  rows) / 11 (odd rows), z = row·20; NP2.0 x = col·32 + 27, z = row·15, shanks 250 µm apart.

x is measured from the shank edge, as SpikeGLX does; probeinterface measures from the leftmost
site, so its x values are 11 µm smaller for NP1.0 (same geometry). Reference sites
(`used = 0`) are kept as electrodes.

## Session
- `start_time`: earliest `fileCreateTime` (local time, no zone: set `session.timezone`).
- `experiment`: the run name; `userNotes` → notes; `firstSample` and `appVersion` → extras.
- Every stream starts at 0 s. Streams are not aligned with the sync channel yet (a warning says
  so when a run has several streams).

## Not yet
OneBox (`obx`) streams; TTL events from sync / digital lines; sync-based alignment; multi-trigger
runs (`_t0`, `_t1`, … are separate runs today); `catgt`-processed files.

## Verification
- `crates/readers/spikeglx/tests/real_run.rs`: IBL `data/ibl/imec_385_100s` (3A, 384 AP + sync,
  100 s) through `nc_core::testkit::check_reader`, plus pinned values.
- `tools/python/compare_spikeglx.py`: values against SpikeGLX's own conversion rule and electrode
  positions against probeinterface (0.4.0: identical geometry, x offset 11 µm).
- Converted to NWB and read back with pynwb 4.2.0 / hdmf-zarr 0.14.0: int16 data identical to the
  `.bin`, volts identical to SpikeGLX's conversion, 384 electrodes with positions.
