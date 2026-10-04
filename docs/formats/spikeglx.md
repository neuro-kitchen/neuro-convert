# SpikeGLX (Neuropixels imec probes, NI-DAQ, OneBox)

Reader: `crates/readers/spikeglx` (`nc-spikeglx`). References: SpikeGLX metadata help,
`readSGLX.py` (SpikeGLX_Datafile_Tools), SGLXMetaToCoords, the IMRO table reference
(billkarsh.github.io/SpikeGLX/help/imroTables).

## Files
| File | Role | Module |
|---|---|---|
| `<run>_g<gate>_t<trig>.imec<N>.ap.bin` / `.lf.bin` | probe data: int16, all saved channels interleaved per sample | `bin.rs` |
| `….nidq.bin` | NI-DAQ data (MN, MA, XA analog, DW digital words) | `bin.rs` |
| `….obx<N>.obx.bin` | OneBox data (XA analog, XD digital word, SY sync) | `bin.rs` |
| `.meta` (one per `.bin`) | `key=value`; `~` tables: `imroTbl`, `snsChanMap`, `snsShankMap`, `snsGeomMap` | `meta.rs` |

- A run is one gate (`<run>_g<gate>`): all its triggers (`_t0`, `_t1`, …; CatGT's `_tcat`)
  and streams open together. A path to a `.bin` / `.meta` opens its gate (from a probe folder,
  its siblings too). A folder opens the gate found in it and two levels below (run folder
  `<run>_g0/`, probe folders `<run>_g0_imec0/`); several gates need `--block <run>_g<gate>`.
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
| OneBox XA | `obx<N>` | volts (`obAiRangeMax / obMaxInt`) |
| OneBox XD / SY | `obx<N>.digital` / `obx<N>.sync` | raw 16-bit words |

With several triggers every name gets the trigger: `imec0.ap.t0`, `imec0.ap.t1`, … (one series
per trigger window; electrodes are shared).

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

## Time
- Every recording starts at `firstSample / rate` (time since acquisition start, on its own
  clock) minus the run's earliest; trigger windows keep their gaps.
- **Sync alignment.** The sync pulse (a square wave, `syncSourcePeriod` s): imec and OneBox bit 6
  of `SY`; NI a digital line (`syncNiChanType=0`, line `syncNiChan`) or an analog channel above
  `syncNiThresh` V (`syncNiChanType=1`). Rising edges are found by sampling every 10 ms and
  bisecting each change (a few thousand reads even for an hour of AP data). Per trigger, each
  stream with ≥ 2 edges is fitted (least squares, edges matched within a quarter period) to the
  reference: the first probe, else NI, else OneBox. The fit sets the stream's start time and
  sample rate (`rate / scale`); a note gives offset, drift (ppm) and largest residual. Streams
  without a pulse keep their own clock (warning).
- **TTL events.** Every NI DW / OneBox XD line that changes becomes `<stream> TTL <line>` (line =
  word × 16 + bit; NI's sync line too), high periods as interval events, on the reference clock,
  joined across triggers. Full scan of the digital column (NI / OneBox files are small).

## Session
- `start_time`: earliest `fileCreateTime` (local time, no zone: set `session.timezone`).
- `experiment`: the run name; `userNotes` → notes (once per stream); alignment notes;
  `firstSample` (per stream, band and trigger), `appVersion`, triggers → extras.

## Not yet
Imec sync bits as events (used for alignment only); OneBox with probes attached (no test data).

## Verification
- `crates/readers/spikeglx/tests/real_run.rs`: IBL `data/raw/spikeglx/imec_385_100s` (3A, 384 AP + sync,
  100 s) through `nc_core::testkit::check_reader`, plus pinned values.
- `tools/python/compare spikeglx`: values against SpikeGLX's own conversion rule
  (`imChan0apGain` when present) and electrode positions against probeinterface (0.4.0:
  identical geometry, x offset 11 µm NP1 / 27 µm NP2). OK on IBL 3A, Noise4Sam (NP1 PRB_1_4),
  NP2010 (type 24), NP2013.
- neo's GIN test data (`tools/python/fetch_gin.py spikeglx/<set>`): DigitalChannelTest — all 163
  TTL high periods identical to neo's ON / OFF events (through NWB); multi_trigger_multi_gate —
  trigger and stream offsets identical to neo's segment `t_start` differences; OneBox
  run_with_only_adc read and verified. Unit tests: synthetic sync alignment (50 ppm, 3 ms
  recovered within 0.2 ms), TTL scan, triggers.
- Converted to NWB and read back with pynwb 4.2.0 / hdmf-zarr 0.14.0: int16 data identical to the
  `.bin`, volts identical to SpikeGLX's conversion, 384 electrodes with positions.
