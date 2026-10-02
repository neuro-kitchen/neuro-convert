# nc-openephys changelog

Each crate has its own version (its `Cargo.toml`, `nc_openephys::version()`; recorded in every
conversion's report and NWB `source_script`). Bump it, and add an entry here, whenever the crate's
behaviour or API changes — especially anything that changes what a conversion writes.

## 0.1.0 — 2026-10-02
First recorded version.
- Binary format (GUI 0.4.4 – 0.6+) and legacy `.continuous` (gaps read as zeros, as neo; TTL,
  messages, `.spikes`; one container per acquisition start).
