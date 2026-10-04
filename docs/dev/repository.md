# Repository layout

```text
crates/
  base/          nc-base        primitives, no domain knowledge
  core/          nc-core        neutral data model, Reader trait, metadata file
  readers/
    tdt/         nc-tdt         one crate per input format
    spikeglx/    nc-spikeglx
    intan/       nc-intan
    openephys/   nc-openephys
    blackrock/   nc-blackrock
    neuralynx/   nc-neuralynx
  nwb/           nc-nwb         NWB writer, Zarr and HDF5 backends, checks
  convert/       nc-convert     API for the CLI and the app
  cli/           nc-cli         binary `neuro-convert`
  app/           nc-app         binary `neuro-convert-app`
docs/            this book (Markdown; mdbook build docs)
templates/reader/  a working reader for a made-up format, to copy
metadata/        metadata template and examples
tools/python/    reference comparisons, NWB read-back, test data download
scripts/         release.sh
packaging/       Linux desktop entry
data/            local test recordings (git-ignored)
```

## Dependencies

```text
   nc-cli      nc-app
       \        /
       nc-convert
       /        \
  nc-<format>   nc-nwb
       \        /
        nc-core
           |
        nc-base
```

Arrows only point down. A reader depends on `nc-core` and `nc-base` only, never on `nc-nwb`. The
NWB writer never depends on a reader. `nc-convert` is the only crate that knows both.

## Where things are

| Crate | File | What |
|---|---|---|
| `nc-base` | `error.rs` | `Error`, `Result` |
| | `sample.rs`, `codec.rs` | `SampleType`, little-endian decoding |
| | `mapped.rs` | memory-mapped files |
| | `text.rs`, `time.rs` | cp437 / Latin-1 text, ISO 8601 times |
| `nc-core` | `reader.rs` | `Reader`, `Detection`, `Maturity` |
| | `session.rs`, `recording.rs` | `Session`, `Recording`, `RecordingInfo` |
| | `events.rs`, `snippets.rs`, `electrodes.rs`, `table.rs`, `metadata.rs`, `provenance.rs` | the rest of the model |
| | `metadata_file.rs`, `apply.rs` | the YAML metadata file and how it merges into a session |
| | `validate.rs`, `issue.rs` | `Session::validate`, `Issue` (error / warning) |
| | `testkit.rs` | `check_reader`: the conformance test every reader runs (feature `testkit`) |
| `nc-<format>` | `lib.rs` | the `Reader` implementation; other files per file type of the format |
| `nc-nwb` | `mapping.rs` | `Session` + metadata → `NwbPlan` |
| | `types/` | one writer per NWB type |
| | `backend/` | `Backend` trait; `zarr.rs`, `hdf5.rs` |
| | `integrity.rs` | content check after writing, `verify` from a report |
| | `validate.rs` | structural checks (`neuro-convert validate`) |
| | `schema.rs`, `specs/` | NWB 2.11 schema, vendored and cached in every file |
| `nc-convert` | `registry.rs` | `Registry`: readers compiled in, detection, opening |
| | `job.rs` | `Job`: open → plan → write → verify, progress, cancel, `Report` |
| | `sources.rs` | source file checksums (SpikeGLX `fileSHA1`) |
| | `preview.rs` | min / max envelopes for signal previews |
| | `versions.rs` | every crate's version |
| `nc-cli` | `commands/` | one file per command |
| `nc-app` | see [`crates/app/README.md`](https://github.com/neuro-kitchen/neuro-convert/blob/main/crates/app/README.md) | MVVM layout: `domain/`, `services/`, `viewmodels/`, `widgets/`, `views/` |

## Tests

| Where | What |
|---|---|
| `crates/*/src/**` (`#[cfg(test)]`) | unit tests on synthetic fixtures written by the test |
| `crates/readers/<format>/tests/` | real recordings from `data/raw/`; skip when absent |
| `crates/nwb/tests/write.rs` | small NWB stores in `target/nwb-test/` |
| `tools/python/compare/` | each reader against its reference reader, through NWB |

Details: [Testing](testing.md).
