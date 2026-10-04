# Architecture

```text
 source files ──Reader::open──▶ Session ──nwb::plan(+ MetadataFile)──▶ NwbPlan ──nwb::write──▶ Backend ──▶ integrity::verify
               nc-<format>       nc-core   nc-nwb mapping.rs                    types/        zarr / hdf5   nc-nwb
```

`nc_convert::Job` runs these steps for the CLI and the app.

## Rules

1. **The `Session` is the only interface between input and output.** Readers fill it; the writer
   reads it. A new format needs no change in `nc-nwb`.
2. **Writers read typed fields only.** `metadata` / `extra` maps in the model are for reports. If
   an output needs something a format knows, the model gets a typed field in `nc-core`, and every
   reader can fill it.
3. **Streaming.** `Recording::read` and `read_stored` serve bounded sample ranges from
   memory-mapped files. The writer copies chunks in parallel. Nothing holds a whole recording.
4. **The metadata file is a separate input.** It fills what the source lacks and decides naming,
   units and inclusion, keyed by the source's own names.

## Session

| Field | Holds |
|---|---|
| `recordings` | Continuous streams (`Arc<dyn Recording>`): `info()` describes, `read` returns scaled float32, `read_stored` the stored bytes. |
| `events` | `EventSeries`: onsets, optional offsets, values, labels. |
| `snippets` | `SnippetSeries`: spike waveforms (read on demand through `Waveforms`), times, channels, sort codes. |
| `electrode_groups`, `electrodes` | Groups and one `Electrode` per contact, with the recording channels it feeds, position and impedance when known. |
| `tables` | Text tables (impedance exports). |
| `metadata` | Start time, subject, devices, notes, extras. |
| `provenance` | Reader, files read, recorded checksums, warnings. |

API: [`nc_core::Session`](api.md).

## Planning

`nc_nwb::plan(session, metadata, id)`:

1. `MetadataFile::apply` merges the file's electrode groups and electrodes into the session.
2. `Session::validate` checks the model's invariants (lengths, references, time order).
3. `mapping::resolve` decides every NWB object, its name, unit and storage type, and collects
   issues. Errors block writing; warnings do not.

## Writing

`nc_nwb::write` writes the plan through a `Backend`. Each NWB type has one writer in `types/`.
Continuous data is written in chunks along time by several threads. Integer samples stay integers
when the gains can be expressed as `conversion` / `channel_conversion`; otherwise float32.

## Checking

After writing, `integrity::verify` reads every array back from the output and reads the same data
again from the source through the reader, and compares them in ~8 MB blocks (xxh3-64). Neither side
goes through the writer's code. `validate` checks the structure. Both results go to the report.

## Conversion job

```rust
let registry = nc_convert::Registry::builtin();
let mut job = nc_convert::Job::open(&registry, path, &OpenOptions::default())?;
let plan = job.plan(&metadata);              // re-run after every metadata change
let report = job.write(dest, &options, &cancel, &|event| { /* progress */ })?;
report.save(&report.default_path())?;        // <output>.report.json
```

`Job` is `Send`: the app plans on the UI thread and writes on a worker thread.

## App

`nc-app` is a GPUI app over `Job`, in MVVM layers: `domain/` (plain Rust, tested without GPUI),
`services/` (long work on named threads), `viewmodels/`, `widgets/`, `views/`. Module map:
[`crates/app/README.md`](https://github.com/neuro-kitchen/neuro-convert/blob/main/crates/app/README.md).
