# Versions and releases

## Crate versions

Every crate has its own version in its `Cargo.toml`, exposed as `<crate>::VERSION` /
`version()`. `nc_convert::versions()` lists them all; `neuro-convert formats` prints them.

A conversion records the program, the reader and every crate's version in:
- the report (`versions`),
- the NWB file (`/general/source_script`).

Bump a crate's version and add a `CHANGELOG.md` entry whenever its behaviour or API changes. A
change in what a conversion writes is always a bump of the crate that changed. Table:
[Changing a reader](changing-a-reader.md#when-to-bump-what).

## Release

```sh
scripts/release.sh
```

1. Builds `nc-cli` with `--locked`, every reader and HDF5 (`hdf5-static`; set
   `NC_HDF5_FEATURE=hdf5` for the system library).
2. Packs `bin/`, `LICENSE-MIT`, `LICENSE-APACHE`, `README.md`, `docs/`, the example metadata and
   `VERSIONS.txt` into `dist/neuro-convert-<version>-<target>.tar.gz`, with a `.sha256`.
3. Fails if the CLI was built without HDF5.

The version in the archive name is `nc-cli`'s.

The desktop app (`nc-app`) is not built by the release script or by CI. It is released only after
a review of the app as a desktop platform.

## Compatibility rules

What each reader supports, its maturity and the tool versions the output was checked with:
[Compatibility](../compatibility.md).
