# nc-blackrock changelog

Each crate has its own version (its `Cargo.toml`, `nc_blackrock::version()`; recorded in every
conversion's report and NWB `source_script`). Bump it, and add an entry here, whenever the crate's
behaviour or API changes — especially anything that changes what a conversion writes.

## 0.1.0 — 2026-10-02
First recorded version.
- NSx / NEV file spec 2.1 – 3.0 incl. PTP: pauses as parts, clock resets (> 1 s back) as
  segments, spikes, digital / serial input, comments.
