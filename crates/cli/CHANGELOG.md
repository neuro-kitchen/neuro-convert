# nc-cli changelog

Each crate has its own version (its `Cargo.toml`, `nc_cli::version()`; recorded in every
conversion's report and NWB `source_script`). Bump it, and add an entry here, whenever the crate's
behaviour or API changes — especially anything that changes what a conversion writes.

## 0.1.0 — 2026-10-02
First recorded version.
- `formats` (with reader and crate versions), `inspect` (shows the reader and its version),
  `convert` (`-o name.nwb.zarr`, or `name.nwb` in HDF5 builds), `validate`, `verify`; `--version`.
