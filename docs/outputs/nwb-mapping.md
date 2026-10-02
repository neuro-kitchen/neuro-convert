# NWB output

Writer: `crates/nwb` (`nc-nwb`). Target: NWB core 2.11.0 (hdmf-common 1.10.0, hdmf-experimental
0.6.0), stored as a Zarr v3 store in the layout hdmf-zarr 0.14 writes, so pynwb / hdmf-zarr (≥ 0.14)
and DANDI read it. The schema is cached under `/specifications` (vendored in `crates/nwb/specs/`).

## Planning: `nc_nwb::plan(session, metadata, id)`
1. `MetadataFile::apply` merges the metadata file's electrode declarations into the session:
   groups, one electrode per channel of streams declared `electrical` with an `electrode_group`,
   impedances from the group's table, and the electrode of each snippet channel. Electrodes and
   positions supplied by the reader are kept (only their group is reassigned).
2. `Session::validate` checks the model's invariants (lengths, references, time order).
3. `resolve` decides names, units and inclusion from the metadata file; electrodes come only from
   the session. Writers read typed model fields, never reader-specific `metadata` keys.

## Mapping (driven by the session and the metadata file, never by source names)
| Session item | NWB | Notes |
|---|---|---|
| Recording, `type: electrical`, or no `type` and every channel has an electrode | `/acquisition/<name>` `ElectricalSeries` | data `[time, channel]`, unit volts, `conversion` from metadata; `electrodes` region = each channel's electrode row |
| Recording, `type: timeseries` (or no electrodes) | `/acquisition/<name>` `TimeSeries` | unit from metadata (default `a.u.`) |
| Data type | as stored (integers, float64) when the source serves raw bytes, channel offsets are 0 and the gains can be expressed: one shared gain → folded into `conversion`; per-channel gains → `channel_conversion` (electrical series only); else float32 of the scaled values | final value = stored × gain × `streams.<name>.conversion`; e.g. Neuropixels int16 with `conversion` = 2.34375e-06 V/bit |
| `RecordingInfo.calibration = Unknown` | — | plan warning until `streams.<name>.conversion` is set (e.g. TDT int16 stores with a Synapse scale) |
| Recording, `include: false` | — | listed under "skipped" in the plan |
| `Session::electrode_groups` | `/general/extracellular_ephys/<group>` | linked to the group's device (default: the session's first device) |
| `Session::electrodes` | `/general/extracellular_ephys/electrodes` | one row per electrode in session order (an electrode feeding several recordings, e.g. Neuropixels AP + LF, is one row referenced by both series); columns `location` (electrode's, else group's), `group`, `group_name`, `channel_name`, plus `imp` (ohms, NaN = not measured) when any electrode has one and `rel_x, rel_y, rel_z` (µm) when any has a position |
| Events (epocs, marks, notes) | `/events/<name>` `EventsTable` (NWB 2.10+) | `timestamp`, `duration` (offset − onset, when offsets exist), `value`, `annotation` (labels such as notes) |
| Multi-channel scalars | `/acquisition/<name>` `TimeSeries` `[event, channel]` | timestamps, or `starting_time` + `rate` when evenly spaced (±0.01 %) |
| Snippets with electrodes (`SnippetSeries::electrodes`) | `/acquisition/<name>_ch<c>` `SpikeEventSeries` | one series per channel, data `[event, 1, sample]`, `electrodes` = that channel's electrode; with `snippets.<name>.electrode_group`, channel c is the c-th channel of the group's first recording (or a new contact when the group has no recording); without electrodes: not written (plan warning) |
| Sorted snippets (non-zero sort codes) | `/units` `Units` | one unit per channel × sort code: `spike_times`, `electrodes`, `waveform_mean` (volts), `source_store`, `source_channel`, `sort_code` |
| Tables (e.g. impedance CSVs) | `/analysis/<name>` `DynamicTable` | numeric columns float64, others text |
| Devices | `/general/devices/<name>` | from the source + electrode groups |
| Session metadata | root + `/general` | description, identifier (UUID if absent), start time with zone, lab, … |

## Checks before writing
Errors (nothing is written): missing `session.description`, start time without a zone, an
`electrode_group` that is not declared, an electrical stream with channels lacking electrodes,
snippet channels outside their group, duplicate output names, any `Session::validate` error.
Warnings: DANDI-required fields missing (`subject.species`, `subject.age`), unknown stream keys,
uncalibrated streams without a conversion, snippets without electrodes.

## `neuro-convert validate <store>`
Structural checks without Python: root type and required datasets, start time zone, cached schema,
electrodes table column lengths and group references, electrode-group → device links, every
series has data + unit + (rate or matching timestamps), electrical series reference valid electrode
rows matching their channel count, interval tables have equal columns and no negative durations.

## Content verification (`nc_nwb::integrity`, `Job::write`)
Structure checks never read samples, so after writing every array with sample data is compared with
the source:
- **What:** continuous series `data`, event `timestamp` / `duration` / `value` (or scalar series
  `data` / `timestamps`), snippet `data` / `timestamps`.
- **How:** in blocks of ~8 MB of rows (independent of the store's chunking). The source side is read
  again through the reader (`read_stored`, or `read` as float32) and laid out as the store's dtype,
  row-major `[time, channel]`, little-endian; the store side is read back through `zarrs`. Neither
  side uses the writer's copy code, so misplaced or swapped chunks, transpositions and dtype
  errors are caught. Each block is hashed with **xxh3-64**; equal bytes are required.
- **Levels** (`NwbOptions.verify`, CLI `--verify`): `full` (CLI default; every block), `sampled`
  (app default; first, last and 8 random blocks per array), `off` (structure only).
- **Source checksums:** at `full`, files whose format records a checksum (SpikeGLX `fileSHA1` in the
  `.meta`) are hashed (SHA-1) before writing; a mismatch stops the conversion (CLI
  `--skip-source-check` to convert anyway). TDT records none.
- **Report:** `digests` (algorithm, level, per array: dtype, shape, rows per block, block digests,
  overall digest when full, mismatched blocks) and `source_checks`. A mismatch is a verification
  error, so `convert` fails.
- **Later:** `neuro-convert verify <store> [--report <report.json>]` re-reads the recorded blocks of
  a copy (e.g. after an upload) and compares them with the report, without the source.

Timings (2026-10-01, 12 threads, release, gzip 1): TDT 47 min block (2.78 G samples, 28 arrays):
write 32 s, full verify 24 s, sampled < 2 s; `verify` from the report 12 s. IBL 100 s
Neuropixels: write 10.5 s, full verify ~5 s; SHA-1 of the 2.3 GB `.bin` ~5 s.

## Test stores
`crates/nwb/tests/write.rs` writes small stores to `target/nwb-test/`;
`tools/python/validate_nwb.py` reads `small.nwb.zarr` back with pynwb + hdmf-zarr (exact values)
and runs nwbinspector. Checked 2026-10-01 with pynwb 4.2.0, hdmf-zarr 0.14.0, zarr 3.4.0,
nwbinspector 0.7.2: exact values OK; nwbinspector's compression checks still crash on every Zarr v3
array (12 ignored), and `check_units_table_duration` crashes on hdmf-zarr arrays (fancy indexing) —
both upstream issues.
