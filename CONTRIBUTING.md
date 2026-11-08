# Contributing to neuro-convert

Thanks for helping. Issues are welcome for bugs, output that differs from a reference reader, and
features (the forms ask for what we need to reproduce). For a security problem, see
[SECURITY.md](SECURITY.md) instead. Everyone taking part follows the neuro-kitchen
[Code of Conduct](https://github.com/neuro-kitchen/.github/blob/main/CODE_OF_CONDUCT.md).

## Setup

```bash
git clone https://github.com/neuro-kitchen/neuro-convert && cd neuro-convert
cargo build                                   # every crate except the app (Zarr output)
cargo build -p nc-cli --features hdf5-static  # + HDF5 output, HDF5 built from source (cmake, C compiler)
cargo build -p nc-app                         # the desktop app (large build; system libraries in the README)
```

Rust 1.85 or later (edition 2024).

## Changes go through pull requests

`main` is protected: no direct pushes, no force pushes. Create a branch, open a pull request, fill
in the template, and resolve review threads before merging. Keep each pull request to one thing.

## Tests

- `cargo test` runs every crate except the app; `cargo test -p nc-app` runs the app's headless UI
  tests.
- Readers are tested on a synthetic fixture and on real recordings. Real data lives in `data/raw/`
  (or `$NC_DATA_DIR`) and those tests skip when it is absent: `python3 tools/python/fetch_gin.py --all`
  downloads the public test data.
- A change in what a reader reads or how it maps to NWB is checked against its reference reader
  (neo, `tdt`, SpikeGLX's rules, probeinterface) with `tools/python/compare` (see the README,
  "Tests"); add `--hdf5` when the HDF5 writer is involved.
- Never commit recordings, subject data or credentials. Use small synthetic fixtures in tests.

## Code

- Match the surrounding code: naming, comment density, module layout.
- Dependencies point one way: `cli, app → convert → readers, nwb → core → base`. Readers do not
  know NWB; the NWB writer does not know any reader.
- New readers start from `templates/reader/` and follow
  [`docs/readers/README.md`](docs/readers/README.md) (mapping rules, known traps, definition of
  done), with a `docs/formats/<format>.md` page.
- Samples keep their stored type; gains go to NWB `conversion` / `channel_conversion`.
- Every crate has its own version and `CHANGELOG.md`. A change in what a reader reads or how it
  maps is a version bump with a changelog entry; breaking changes to `nc-core` are deprecated one
  release ahead ([`docs/compatibility.md`](docs/compatibility.md)).
- Commit messages: `type(scope): summary` (`feat`, `fix`, `perf`, `docs`, `test`, `ci`,
  `refactor`, `chore`), with what changed and how it was checked in the body.

## Licences

neuro-convert is MIT OR Apache-2.0. Code written from another project's source (a reference
reader, a file-format SDK) keeps its licence and notice: say where it comes from (file, version)
in the module docs. Code written from a format specification says so and links it. Only take code
whose licence is compatible with MIT OR Apache-2.0.

By contributing you agree that your contribution is licensed as MIT OR Apache-2.0, at the user's
option, without additional terms or conditions.
