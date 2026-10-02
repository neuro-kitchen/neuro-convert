# nc-neuralynx changelog

Each crate has its own version (its `Cargo.toml`, `nc_neuralynx::version()`; recorded in every
conversion's report and NWB `source_script`). Bump it, and add an entry here, whenever the crate's
behaviour or API changes — especially anything that changes what a conversion writes.

## 0.1.0 — 2026-10-02
First recorded version.
- Cheetah / Pegasus / BML / Neuraview: `.ncs` streams as neo groups them, gaps as parts, the
  sample rate measured from record timestamps (not the stated rate); `.nse` / `.nst` / `.ntt`
  spikes per wire; `.nev` events per (id, TTL).
