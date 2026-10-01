# neuro-convert

Pure-Rust, streaming neurophysiology data converter. Reads raw vendor recordings into a neutral session model and converts them into publication-ready **NWB 2.11+ (Zarr v3)** stores without loading multi-hour recordings into memory.

## Usage

```bash
neuro-convert formats                         # List supported input and output formats
neuro-convert inspect <recording> [--json]    # Inspect streams, events, snippets, tables, and warnings
neuro-convert convert <recording> -m meta.yaml -o out.nwb.zarr [--dry-run] [--gzip 1]
neuro-convert validate out.nwb.zarr           # Run structural & schema checks on an NWB store
```

## Supported Formats

| Input | Capabilities |
| :--- | :--- |
| **TDT (Tucker-Davis Technologies)** | Synapse & OpenEx blocks/tanks (`--block`); `TEV` & `SEV` (v0–v3, multi-hour splits) continuous streams, `rawpacked` stores, snippet stores (`snips` + offline `.SortResult` via `--sort`), epocs, scalars, runtime notes, and impedance CSVs |

| Output | Capabilities |
| :--- | :--- |
| **NWB 2.11.0 (Zarr v3)** | `hdmf-zarr`-compatible Zarr v3 hierarchy (`ElectricalSeries`, `TimeSeries`, `SpikeEventSeries`, `Units`, `EventsTable`, `TimeIntervals`, `DeviceModel`), embedded core/HDMF schema specifications, DANDI readiness validation |

## Repository Structure

```text
crates/neuro-convert/
  src/model/      Neutral Session, Recording (chunked + stored reads), events, snippets, tables, metadata
  src/common/     Binary codecs, memory-mapped I/O, text encodings, ISO timestamp helpers
  src/inputs/     Format readers (TDT TSQ/TEV/SEV, epocs, snippets, SortResult, notes, TIN)
  src/metadata/   YAML session & stream metadata parser and validator
  src/outputs/    NWB Zarr v3 planner, neurodata type writers, and structural validator
  specs/          Vendored NWB Core & HDMF YAML schemas (BSD)
  metadata/       Example session metadata YAML templates
  tests/          Integration tests & PyNWB / hdmf-zarr / nwbinspector verification scripts
```

## License

Licensed under the [MIT License](LICENSE).
