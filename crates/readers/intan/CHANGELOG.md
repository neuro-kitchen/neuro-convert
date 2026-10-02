# nc-intan changelog

Each crate has its own version (its `Cargo.toml`, `nc_intan::version()`; recorded in every
conversion's report and NWB `source_script`). Bump it, and add an entry here, whenever the crate's
behaviour or API changes — especially anything that changes what a conversion writes.

## 0.1.0 — 2026-10-02
First recorded version.
- RHD / RHS: traditional files, one file per signal type, one file per channel; all streams,
  digital lines as events, electrodes per port with impedances.
