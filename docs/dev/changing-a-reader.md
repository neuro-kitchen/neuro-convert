# Changing a reader

For a new reader, see [Writing a reader](../readers/README.md). This page is for changes to an
existing one: a new file version, a fix, a new stream type.

## Steps

1. **Reproduce.** Find or make a recording that shows the change. Public data: add the set to
   `tools/python/testdata.toml` and fetch it with `tools/python/fetch_gin.py`. No data files go in
   the repository; unit tests write synthetic fixtures.
2. **Test first.** Add a unit test with a fixture that has the new case, or a real-data test in
   `crates/readers/<format>/tests/`.
3. **Change the reader.** Follow the [mapping rules](../readers/README.md#2-mapping-rules-what-every-reader-does-the-same-way).
   If the output needs new information, add a typed field to the model in `nc-core`; never
   special-case a format in `nc-nwb`.
4. **Run the checks.**
   ```sh
   cargo test -p nc-<format>
   cargo test
   uv run --no-project --with neo --with pynwb --with hdmf-zarr --with tdt --with probeinterface \
       python tools/python/compare <format> <path>...
   ```
   Run the comparison on every data set of the format in `testdata.toml`, not only the new one.
5. **Document.** Update `docs/formats/<format>.md`: the mapping, the verification table (data set,
   result, date), known differences, "Not yet".
6. **Version.** Bump `version` in the crate's `Cargo.toml` and add an entry to its `CHANGELOG.md`.

## When to bump what

| Change | Version | Changelog |
|---|---|---|
| Anything that changes what a conversion writes (values, names, times, electrodes) | yes | yes, say what changes in the output |
| New file version or stream type read | yes | yes |
| Bug fix | yes | yes |
| Internal refactor, same output | no | optional |
| Docs only | no | no |

The version is written to every report and NWB file, so a file can be traced to the reader that
wrote it.

## Maturity

`Reader::maturity()` is `Verified` only while the reference comparison passes on every data set of
the format in `testdata.toml`. If a change breaks a comparison and the difference is intended,
document it on the format page (as the Intan DC amplifier sign is).

## Changing `nc-core`

The `Reader` trait and the model are used by readers outside this repository.

| Change | Breaking | What to do |
|---|---|---|
| New trait method with a default | no | add, changelog |
| New optional model field | no | add, changelog |
| Renamed or removed item, changed signature | yes | keep the old item one release with `#[deprecated(note = "use …")]`, list it in `crates/core/CHANGELOG.md` |

Rules: [Compatibility](../compatibility.md#versions-and-changes).
