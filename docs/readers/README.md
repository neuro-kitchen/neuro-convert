# Writing a reader

A reader turns one family of recording files into neuro-convert's neutral model, a `Session`.
Everything after that — metadata, the NWB plan, Zarr / HDF5 output, verification, the app — works
for every format without knowing it. This guide is what you need to add one. Start from the
template in [`templates/reader/`](https://github.com/neuro-kitchen/neuro-convert/tree/main/templates/reader) and look at the existing readers in
`crates/readers/` (each has a page in [`docs/formats/`](../formats/)).

## 1. The pieces

```text
 your files ──Reader::detect / open──▶ Session ──(metadata YAML)──▶ NwbPlan ──▶ .nwb.zarr / .nwb
             crates/readers/<format>     nc-core                     nc-nwb
```

A reader crate (`crates/readers/<format>`, package `nc-<format>`) depends only on `nc-base` and
`nc-core`. It never mentions NWB: if the output needs something your format knows, the model gets
a typed field for it (one change in `nc-core`, then every reader can fill it).

### `Reader` (`nc_core::Reader`)
| Method | What it does |
|---|---|
| `name()` | Short id used on the command line and in reports (`blackrock`). |
| `version()` | Your crate's `VERSION` (each crate exposes `VERSION` / `version()`); recorded in every conversion. |
| `description()`, `opens()` | One line each: what the format is, and what a user selects to open it (shown by the app). |
| `versions()` | The file-format versions / variants you handle, one line each (`neuro-convert formats`). |
| `detect(path)` | `Some(Detection { format, version, confidence })` when `path` looks like yours. Cheap: read a header, never the data. Confidence 0.9–0.95 for a clear signature. |
| `containers(path)` | When `path` holds several recordings (blocks, runs, segments, starts): their names, else empty. |
| `open(path, options)` | Builds the `Session`. `options.block` chooses a container, `options.only` limits streams, `options.sort` an offline sort. |

### `Session`
| Field | Holds |
|---|---|
| `recordings` | Continuous streams, each an `Arc<dyn Recording>`. |
| `events` | `EventSeries`: onsets (s), optional offsets, values (`channels` per event, row-major), labels. One value per event → NWB events table; several → a `TimeSeries`. |
| `snippets` | `SnippetSeries`: one waveform per snippet (`samples_per_snippet` samples), timestamps, source channel, sort code (0 = unsorted), and `waveforms: Arc<dyn Waveforms>` read on demand. |
| `electrode_groups`, `electrodes` | What your hardware tells: groups (probe, shank, bank) and one `Electrode` per contact, with `channels` (every recording channel it feeds, e.g. AP and LF), position (µm) and impedance when known. The metadata file fills the rest. |
| `tables` | Text tables (e.g. impedance exports). |
| `metadata` | Start time (ISO 8601; leave the zone off if the file has none), subject, devices, notes, `extra` (free key–values for reports). |
| `provenance` | `Provenance::new("<name>")`, every file read (`add_file`), checksums the format records (`set_checksum`), the detected format version, and **warnings**. |

### `Recording`
`info()` describes the stream; `read(channels, samples, out)` returns float32, channel-major,
scaled by each channel's `gain` and `offset`; `read_stored(...)` returns the stored samples as
little-endian bytes of `stored_as` (return `Ok(false)` if you cannot). When `read_stored` works,
NWB keeps the source's integers and writes the gains as `conversion` / `channel_conversion`
(smaller files, exact values). Use `check_read` at the top of both: it validates the request.

`RecordingInfo` essentials: `samples`, `sample_rate`, `start_time` (s from the session's time
zero), `unit` (`V` for anything calibrated to volts), `calibration` (`Known`, or `Unknown { note }`
when the file's scale cannot be trusted — the user is then asked for a conversion), `kind`
(`Electrical` for electrode channels, else `Other`), `stored_as`, `order` (how samples lie in the
file), `storage` (a short label for displays, e.g. `ncs`, `sev v3`), `metadata` (extras).

## 2. Mapping rules (what every reader does the same way)

1. **Time axis.** One time zero per session: the earliest sample, event or spike of what was
   opened. Every `start_time`, onset and spike time is seconds from it. Compute differences in the
   file's integer clock first (ticks, sample numbers, µs), then convert, so times on one clock are
   exact.
2. **Calibration.** Keep the stored integers; express physical units through `gain` / `offset` per
   channel (volts for electrodes and analog inputs). Never rescale samples yourself.
3. **Streams.** Split a file's channels by what they are: electrode channels (electrical), analog
   inputs (`<name>.analog`), digital / sync words (`<name>.digital`, `<name>.sync`). Same rate and
   same clock → one stream.
4. **Electrodes.** One per physical contact, shared by every stream that records it (AP + LF, CSC +
   tetrode wires on the same AD channel). Group by what the hardware groups (probe, shank, bank).
   Positions only when the file states them.
5. **Events.** TTL lines become interval events (`<stream> TTL <line>`, onsets = rising edges,
   offsets = falling edges; a line still high at the end closes at the end, never before its
   onset). Words become events with the word as value. Text becomes labelled events. Keep events
   in time order.
6. **Snippets.** One snippet per channel of each spike (tetrode spike → four snippets with the same
   time); waveforms read from the file through `Waveforms`, never copied into memory.
7. **Containers.** Things that do not share a time axis (TDT blocks, Open Ephys starts, Blackrock
   clock resets, SpikeGLX gates) are containers; things that do (pauses, triggers, gap-free
   sections) are parts of one session, as separate series with their own start times (`.p1`,
   `.t1`, …).
8. **Problems.** Recoverable problems (truncated last record, gaps, clipped channels, unread
   packet types, a missing sidecar) are `provenance.warnings` with a plain sentence. Errors are for
   what makes the recording unusable.
9. **Nothing format-specific outside the crate.** If a writer would need it, it becomes a typed model
   field.

## 3. Traps met so far

| Trap | Where | What to do |
|---|---|---|
| Big-endian samples | Open Ephys legacy `.continuous` | Convert in `read` / `read_stored` (stored bytes must be little-endian). |
| Gaps between records | Open Ephys legacy, Neuralynx | Detect from record timestamps; either zeros (as neo) or parts — say which in the docs. |
| Clock resets vs jitter | Blackrock 2.3 / PTP | A step back of more than a second is a reset (new container); smaller steps are jitter. |
| Stated vs real sample rate | Neuralynx, SpikeGLX | Use the rate the timestamps imply when the hardware clock drifts; keep the stated one in `metadata`. |
| Inverted inputs | Neuralynx | A negative gain, not flipped samples. |
| Channel order in the file | Neuralynx tetrodes (`-ADChannel 55 54 53 52`), SpikeGLX `snsSaveChanSubset` | Keep the file's order; map by id, never assume sorted. |
| Sidecars missing pieces | SpikeGLX LF `.meta` without site maps | Share what one file of the same device has with its siblings. |
| Headers without their first line | Neuralynx NoDateHeader | Detect by content (properties), not only by a magic first line. |
| Packet / record layouts that differ from the docs | Open Ephys `.spikes` (an undocumented sample-rate field) | Check against real files (record size × count = file size) and the reference reader's source. |
| Floating-point time differences | everywhere | Subtract integer ticks first (rule 1). |
| Huge files | Neuropixels, long sessions | Memory-map; read only what is asked; scan with strides and bisection (sync edges), never whole files at open. |

## 4. Definition of done

A reader is done when it has all of:

1. **Unit test on a synthetic fixture** written by the test (no data files in the repo), run
   through `nc_core::testkit::check_reader` (detection, `Session::validate`, read consistency at
   the start, middle and end of every recording) plus value checks.
2. **Real data** through `check_reader` (tests skip when `data/` is absent; get files with
   `tools/python/fetch_gin.py <format>/<set>` for neo's public test data).
3. **A reference comparison** through the whole pipeline: convert, read the NWB back with pynwb,
   compare every value, time and spike with neo (or the vendor's reader). Known differences are
   explained, not hidden. See `tools/python/compare/` (one module per format, shared `common.py`; add yours there) and
   `tools/python/testdata.toml` (add your data sets with their digests).
4. **A format page** `docs/formats/<format>.md`: files, mapping, time, verification, known
   differences, not yet.
5. **Registered and versioned**: a feature `<format>` in `nc-convert` (and the CLI / app feature
   lists), one line in `Registry::builtin()`, the crate in `nc_convert::versions()`, a
   `CHANGELOG.md` in the crate.

## 5. Steps

1. Copy `templates/reader/` to `crates/readers/<format>/` (the workspace picks up every crate
   under `crates/readers/`); rename `nc-template` / `Template` to your format.
2. Add `nc-<format> = { path = "crates/readers/<format>" }` to the root
   `[workspace.dependencies]`.
3. Write `detect` and a fixture writer in the test; then `open` for one stream; then events,
   snippets, electrodes, containers.
4. Register: optional dependency + feature in `crates/convert/Cargo.toml` (in `default`), a line in
   `Registry::builtin()`, the crate in `versions()`, `pub use nc_<format> as <format>` in
   `nc-convert`, the feature in `crates/cli/Cargo.toml` and `crates/app/Cargo.toml`.
5. Real data, the reference comparison, the format page, the changelog.

Out of tree: a reader in your own crate works too —
`nc_convert::Registry::builtin().with(MyReader)` — without changing neuro-convert.

## 6. Maturity

`Reader::maturity()` defaults to `Maturity::Experimental`. Return `Maturity::Verified` once the
reference comparison passes on every data set of your format in `tools/python/testdata.toml`;
`neuro-convert formats` and the app show the label. Readers kept outside neuro-convert are
`Maturity::Community`.

## 7. Versions

Every crate has its own version (`<crate>::VERSION` / `version()`); `Reader::version()` returns
yours. A conversion records the program, the reader and every crate's version in its report and in
the NWB file's `/general/source_script`, so a problem in a file can be traced to the code that
wrote it. Bump your crate's version and add a changelog entry whenever what it reads or how it maps
changes. How breaking changes to `nc-core` (the trait and the model) are announced:
[`docs/compatibility.md`](../compatibility.md).
