# nc-base changelog

Each crate has its own version (its `Cargo.toml`, `nc_base::version()`; recorded in every
conversion's report and NWB `source_script`). Bump it, and add an entry here, whenever the crate's
behaviour or API changes — especially anything that changes what a conversion writes.

## 0.1.0 — 2026-10-02
First recorded version.
- Errors, sample types, little-endian decoding, memory-mapped files (`MappedFile`, with `Debug`),
  Latin-1 / cp437 text, ISO-8601 times.
