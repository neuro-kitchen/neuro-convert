# nc-core changelog

Each crate has its own version (its `Cargo.toml`, `nc_core::version()`; recorded in every
conversion's report and NWB `source_script`). Bump it, and add an entry here, whenever the crate's
behaviour or API changes — especially anything that changes what a conversion writes.

## 0.1.0 — 2026-10-02
First recorded version.
- Neutral model: `Session`, `Recording`, `EventSeries`, `SnippetSeries` (waveforms streamed through
  `Waveforms`, read per block), electrodes and groups, tables, metadata, `Provenance` (`reader` =
  `<name> <version>`, set by the registry).
- `Reader` trait, including `version()`; YAML `MetadataFile` + `apply`; `Session::validate`;
  `testkit::check_reader`.
