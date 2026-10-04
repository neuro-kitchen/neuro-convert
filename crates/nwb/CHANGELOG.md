# nc-nwb changelog

Each crate has its own version (its `Cargo.toml`, `nc_nwb::version()`; recorded in every
conversion's report and NWB `source_script`). Bump it, and add an entry here, whenever the crate's
behaviour or API changes — especially anything that changes what a conversion writes.

## 0.1.0 — 2026-10-02
First recorded version.
- NWB 2.11.0 writer: Zarr v3 (hdmf-zarr layout) and, with `hdf5` / `hdf5-static`, HDF5 `.nwb`
  (links, object references, parallel compression with direct chunk writes).
- `VectorIndex` columns are uint64; `Units/spike_times` carries `resolution`;
  `/general/source_script` records program, reader and writer versions.
- Integrity check: xxh3-64 block digests, source vs store, full / sampled / off, for both backends.
