# Testing

| Level | Command | Needs |
|---|---|---|
| Unit and integration | `cargo test` | nothing |
| App | `cargo test -p nc-app` | app system packages ([Build](../guide/install.md#desktop-app)) |
| HDF5 writer | `cargo test -p nc-nwb --features hdf5-static` | cmake, a C compiler |
| Real recordings | `cargo test` with `data/raw/` present | test data |
| NWB read-back | `tools/python/validate_nwb.py` | Python, uv |
| Reference comparison | `tools/python/compare` | Python, uv, test data, a release build |

## Unit tests

Each test writes its own synthetic fixture; the repository holds no recording files. Every reader
runs `nc_core::testkit::check_reader` on its fixture: detection, `Session::validate`, and reads at
the start, middle and end of every recording.

App tests are headless: they drive real windows with hit testing (`#[gpui_kit::test]`), without
pixels.

## Test data

Public recordings come from neo's test data on GIN. `tools/python/testdata.toml` lists every set
with its file count, size and digest.

```sh
python3 tools/python/fetch_gin.py --format blackrock   # one format
python3 tools/python/fetch_gin.py --all                # every set
python3 tools/python/fetch_gin.py --check              # local files against the digests
```

Files go to `data/raw/` (git-ignored; `$NC_DATA_DIR` overrides). Tests that need them skip with a
message when they are absent.

## NWB read-back

```sh
cargo test -p nc-nwb                                   # writes target/nwb-test/small.nwb.zarr
uv run --no-project --with pynwb --with hdmf-zarr --with nwbinspector \
    tools/python/validate_nwb.py target/nwb-test/small.nwb.zarr --small
```

Reads the store with pynwb + hdmf-zarr, compares exact values, and runs nwbinspector.

## Reference comparison

```sh
cargo build --release -p nc-cli --features hdf5-static
uv run --no-project --with neo --with pynwb --with hdmf-zarr --with tdt --with probeinterface \
    python tools/python/compare <format> <path>... [--block NAME] [--hdf5]
```

| Format | Path | Reference |
|---|---|---|
| `tdt` | block folder | `tdt.read_block` |
| `spikeglx` | `.ap.bin` file | SpikeGLX's conversion rule, probeinterface |
| `intan` | `.rhd` / `.rhs` file or RHX folder | neo `IntanRawIO` |
| `openephys` | binary-format save folder | neo `OpenEphysBinaryRawIO` |
| `openephys-legacy` | folder of `.continuous` files | neo `OpenEphysRawIO` |
| `blackrock` | base path without extension | neo `BlackrockRawIO` |
| `neuralynx` | session folder | neo `NeuralynxRawIO` |

Each run converts the recording (to `target/compare/`), reads the NWB file back with pynwb and
compares every value, time and spike with the reference. `--hdf5` writes `.nwb` instead of Zarr.
It exits non-zero on any mismatch. Known differences are listed on each format page.

A new format adds `tools/python/compare/<format>.py` (shared helpers in `common.py`) and its data
sets in `testdata.toml`.
