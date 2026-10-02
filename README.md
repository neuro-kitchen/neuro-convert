# neuro-convert

Read neurophysiology recordings into one neutral model and convert them to NWB stored as Zarr.
Pure Rust, streaming (multi-hour recordings never sit in memory), modular (one crate per input
format).

```
neuro-convert formats                         # inputs / outputs this build supports
neuro-convert inspect <recording> [--json]    # streams, events, tables, metadata, warnings
neuro-convert convert <recording> -m meta.yaml -o out.nwb.zarr [--dry-run] [--gzip 1]
neuro-convert validate out.nwb.zarr           # structural checks of the NWB store
neuro-convert verify out.nwb.zarr             # structure + content against its report's digests
```
Options for opening a recording: `--block <name>` (TDT tanks), `--sort <id>` (offline spike
sorts), `--only A,B` (load only these streams / stores).

## Supported
| Input | Versions |
|---|---|
| TDT block or tank | Synapse / OpenEx; TEV and SEV (v0–v3, hour files) streams, rawpacked, snips (+ `--sort` offline sorts), epocs, scalars, runtime notes, impedance CSVs; `--block` for tanks |
| SpikeGLX run | Neuropixels 3A / 1.0 family / 2.0 AP, LF and sync (gains from the IMRO table, electrode positions from the geometry map); NI-DAQ analog + digital; `--block` for folders with several runs |
| Intan RHD / RHS | Traditional `.rhd` / `.rhs` files and RHX folders (one file per signal type or per channel): amplifier, aux, supply, temperature, board ADC / DAC, RHS DC amplifier and stimulation current, digital lines as events; electrodes per headstage port with impedances |
| Open Ephys (binary) | GUI 0.4.4 – 0.6+: continuous streams split into electrode / analog / sync channels, Neuropixels site positions from `settings.xml`, TTL lines and messages as events, synchronized timestamps; `--block` for record node / experiment / recording |

| Output | Format |
|---|---|
| NWB 2.11.0 | Zarr v3 store in hdmf-zarr's layout (hdmf-zarr ≥ 0.14 / pynwb, DANDI), schema cached; continuous series (integers kept, gains as `conversion` / `channel_conversion`), events, spike snippets + sorted units, electrodes with positions, tables |

Not yet: NWB/HDF5, Open Ephys legacy `.continuous` format, SpikeGLX OneBox and sync alignment.

Ctrl-C during `convert` stops cleanly and removes the partial store; every conversion is verified
(structure, and content compared with the source: `--verify full|sampled|off`, plus SpikeGLX's own
file checksums at `full`) and leaves `<output>.report.json` with the content digests.

## Metadata file
The source files never say everything NWB needs (session description, subject species/age, time
zone, electrode placement) nor how each stream should be exported. A YAML file supplies both; keys
are the source's own names, so any lab's naming works. Start from
[`metadata/session.example.yaml`](metadata/session.example.yaml); real examples are
[`metadata/examples/tdt-15-25-33_meps.yaml`](metadata/examples/tdt-15-25-33_meps.yaml) and
[`metadata/examples/spikeglx-ibl-imec_385_100s.yaml`](metadata/examples/spikeglx-ibl-imec_385_100s.yaml).
`convert --dry-run` prints the resulting plan with errors (blocking) and DANDI warnings.

## Workspace
```
crates/base/          nc-base     errors, sample types, decoding, mmap, text, ISO time (no domain)
crates/core/          nc-core     the neutral model (Session, Recording, events, snippets, tables,
                                  metadata, provenance), the Reader trait, the metadata YAML
crates/readers/tdt/   nc-tdt      TDT reader (tsq, tev streams, sev, epocs, snips, notes, tin, …)
crates/readers/spikeglx/ nc-spikeglx  SpikeGLX reader (meta, bin, probe gains and geometry, files)
crates/readers/intan/ nc-intan    Intan RHD / RHS reader (header, data layouts)
crates/readers/openephys/ nc-openephys  Open Ephys binary reader (oebin, npy, settings.xml)
crates/nwb/           nc-nwb      NWB writer: mapping (plan), types (one file per NWB type),
                                  backend (Zarr), validate; vendored schema in specs/
crates/convert/       nc-convert  the API: reader registry, conversion Job (open → plan → write →
                                  verify, progress, cancel), re-exports (used by CLI and app)
crates/cli/           nc-cli      the `neuro-convert` command (clap)
docs/                 format notes and the NWB mapping
metadata/             metadata template and examples
tools/python/         cross-checks against TDT's reader and pynwb / nwbinspector
data/                 local test recordings (git-ignored)
```
Dependencies only point down: `cli → convert → {readers, nwb} → core → base`. Readers never see
NWB, and the NWB writer never sees a reader. A new format is a new crate under `crates/readers/`
that implements `nc_core::Reader`; it is enabled through a feature of `nc-convert`, or added at
run time with `Registry::builtin().with(MyReader)`.

## Tests
```
cargo test --workspace          # unit + integration
uv run --no-project --with pynwb --with hdmf-zarr --with nwbinspector \
    tools/python/validate_nwb.py target/nwb-test/small.nwb.zarr --small
cargo build --release && uv run --no-project --with tdt --with numpy \
    tools/python/compare_tdt.py data/raw/tdt-examples/15-25-33_meps
uv run --no-project --with numpy --with probeinterface \
    tools/python/compare_spikeglx.py data/raw/spikeglx/imec_385_100s/imec_385_100s.ap.bin
uv run --no-project --with neo --with pynwb --with hdmf-zarr \
    tools/python/compare_intan.py data/raw/intan/*
uv run --no-project --with neo --with pynwb --with hdmf-zarr \
    tools/python/compare_openephys.py data/raw/openephys/v0.6.x_neuropixels_with_sync
```
Real-data tests read `raw/tdt-examples/15-25-33_meps` (TDT), `raw/spikeglx/imec_385_100s`
(SpikeGLX), `raw/intan/` and `raw/openephys/` (neo's public test files on GIN,
`NeuralEnsemble/ephy_testing_data`) under `data/` or `$NC_DATA_DIR`, and skip (with a message)
when absent.
