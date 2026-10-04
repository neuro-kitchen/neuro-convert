# nc-tdt changelog

Each crate has its own version (its `Cargo.toml`, `nc_tdt::version()`; recorded in every
conversion's report and NWB `source_script`). Bump it, and add an entry here, whenever the crate's
behaviour or API changes — especially anything that changes what a conversion writes.

## 0.1.0 — 2026-10-02
First recorded version.
- Synapse / OpenEx blocks and tanks: TEV + SEV (v0–v3) streams, rawpacked, snips (waveforms read
  from the TEV on demand) + offline sorts, epocs, scalars, notes, impedance CSVs.
