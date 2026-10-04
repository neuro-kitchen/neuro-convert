# nc-convert changelog

Each crate has its own version (its `Cargo.toml`, `nc_convert::version()`; recorded in every
conversion's report and NWB `source_script`). Bump it, and add an entry here, whenever the crate's
behaviour or API changes — especially anything that changes what a conversion writes.

## 0.1.0 — 2026-10-02
First recorded version.
- `Registry` (built-in readers by feature; `with` for others), `Job` (open → plan → write → verify,
  cancel, `Report`), `versions()` (every crate in the build), `preview::envelope`.
- Reports list the program and crate versions; a cancelled HDF5 file is removed like a Zarr store.
