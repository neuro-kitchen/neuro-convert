# Intan RHD / RHS (`nc-intan`)

Intan Technologies acquisition systems: **RHD** (RHD2000 USB interface board, RHD Recording
Controller) and **RHS** (Stimulation / Recording Controller). Written by Intan RHX (≥ 3.0) and the
older Intan Recording / Stimulation programs.

## What to open
| Layout | Select | Files |
|---|---|---|
| Traditional | the `.rhd` / `.rhs` file (a folder with one) | header + data blocks of 60 (RHD < 2.0) or 128 samples |
| One file per signal type | the folder (or its `info.rhd` / `info.rhs`) | `time.dat`, `amplifier.dat`, `auxiliary.dat`, `supply.dat`, `analogin.dat`, `digitalin.dat`, `digitalout.dat`; RHS: `dcamplifier.dat`, `stim.dat`, `analogout.dat` |
| One file per channel | the folder (or its `info.*`) | `time.dat`, `amp-A-000.dat`, `aux-A-AUX1.dat`, `vdd-…`, `board-ANALOG-IN-1.dat`, `board-DIGITAL-IN-01.dat`; RHS: `dc-A-000.dat`, `stim-A-000.dat`, `board-ANALOG-OUT-1.dat` |

A folder with several traditional files (RHX splits long recordings every N minutes) offers each
as a container (`--block <file stem>`); joining them is not done yet.

## Header
Magic `0xC6912702` (RHD) / `0xD69127AC` (RHS), version, sample rate, DSP / bandwidth settings,
notch mode, impedance test frequencies, three notes (UTF-16 `QString`s), then RHD: temperature
sensor count (≥ 1.1), board mode (≥ 1.3), reference channel (≥ 2.0); RHS: settle / recovery
modes, stimulation step size, recovery settings, notes, DC-amplifier-saved flag, board mode,
reference. Then signal groups (ports A–H, board) with their channels: native / custom name,
native order (the bit of a digital line), signal type, enabled, impedance magnitude / phase.
Field order as in Intan's `importrhdutilities.py` / `importrhsutilities.py` and neo's
`IntanRawIO`.

Traditional data block (each field: every channel's samples of the block back to back):
- RHD: timestamps (int32 × B; uint32 before 1.2), amplifier (uint16 × B per channel), auxiliary
  (× B/4), supply (× 1), temperature (int16 × 1 per sensor), board ADC (× B), digital in (one
  word × B, if any), digital out.
- RHS: timestamps, amplifier, DC amplifier (if saved), stimulation (one per amplifier channel),
  board ADC, board DAC, digital in, digital out.

## Streams and scales (Intan's readers)
| Stream | Stored | Value |
|---|---|---|
| `amplifier` | uint16 offset by 32768 (traditional) / int16 (RHX) | 0.195 µV per step (kept int16, `conversion` = 0.195e-6) |
| `aux` (RHD) | uint16 | 37.4 µV per step; ¼ rate in traditional files, full rate in RHX files |
| `supply` (RHD) | uint16 | 74.8 µV per step; once per block |
| `temperature` (RHD) | int16 | 0.01 °C per step (neo uses 0.001); once per block |
| `analog_in` | uint16 | RHD board mode 0: 50.354 µV; mode 1: 152.59 µV × (raw − 32768); mode 13 and RHS: 312.5 µV × (raw − 32768) |
| `analog_out` (RHS) | uint16 | 312.5 µV × (raw − 32768) |
| `dc_amplifier` (RHS) | uint16 | −19.23 mV × (raw − 512) (neo uses +19.23 mV) |
| `stim` (RHS) | uint16 word | bits 0–7 magnitude, bit 8 negative → × stimulation step (A); compliance / recovery / settle flags (bits 15–13) not kept |

`(raw − 32768)` streams are stored as int16 (`raw ^ 0x8000`), so NWB keeps 16-bit data with a
`conversion`. DC amplifier and stimulation are written as float32.

Digital inputs / outputs: one event series per line (high periods: onset, offset, value 1); lines
that never go high are left out. Traditional and per-type files hold one 16-bit word per sample
(bit = native order); per-channel files hold 0 / 1 per sample.

## Electrodes and metadata
- One electrode per amplifier channel (custom name), one electrode group per headstage port
  (`port A`, location `unknown`), a device per headstage; impedances from the header when measured.
  RHS DC-amplifier channels link to the same electrodes. Probe geometry / wiring: later (M8).
- Start time from an RHX name ending in `_YYMMDD_HHMMSS` (file stem or folder); otherwise none (set
  `session.start_time`). Notes → session notes; version, bandwidth, DSP cutoff, reference, layout →
  `extra`.
- Timestamps are checked to count up by one; gaps are reported as warnings (not filled).

## Verification
`tools/python/compare_intan.py` converts each recording to NWB, reads it back with pynwb and
compares every value of every stream with neo's `IntanRawIO`, and the digital onsets with neo's
rising edges. Checked 2026-10-01 (neo from PyPI) on the GIN test files
`intan_rhd_test_1.rhd` (RHD 1.5), `intan_rhs_test_1.rhs` (RHS 1.0),
`intan_fps_test_231117_052500`, `intan_fpc_test_231117_052630` (RHD 3.3) and
`intan_fps_rhs_test_240329_091536` (RHS 3.3): all streams equal to rounding (DC amplifier with the
opposite sign, see above), digital onsets equal.
Test data: https://gin.g-node.org/NeuralEnsemble/ephy_testing_data/src/master/intan → `data/raw/intan/`.

Not yet: joining time-split files, notch filtering on request, stimulation flags, files with
disabled amplifier saving in the per-channel layout (missing files are skipped with a warning).
