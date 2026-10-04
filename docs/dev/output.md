# Output: NWB types and storage backends

The output is NWB, in `nc-nwb`. It has two layers:

| Layer | Decides | Where |
|---|---|---|
| NWB types | which NWB objects are written, with which attributes | `mapping.rs`, `types/` |
| Storage backend | how groups, attributes and arrays are stored on disk | `backend/` |

The NWB types write only through the `Backend` trait, so Zarr and HDF5 share every type writer.

Mapping from the model to NWB, object by object: [NWB output](../outputs/nwb-mapping.md).

## Adding or changing an NWB type

1. **Model.** If the information is not in the `Session` yet, add a typed field in `nc-core` and
   fill it in the readers that know it.
2. **Plan.** In `mapping.rs`, add the item to `NwbPlan` (name, inclusion, issues). Use the
   metadata file only for naming, units and inclusion.
3. **Writer.** Add `types/<type>.rs` (or extend one) writing through `Backend`. `types::typed`
   gives the `namespace`, `neurodata_type` and `object_id` attributes.
4. **Check.** If the type holds sample data, add its arrays to `integrity.rs` so they are compared
   with the source. Add structural checks to `validate.rs` when pynwb or DANDI require them.
5. **Test.** Extend `crates/nwb/tests/write.rs`, then read the store back with pynwb:
   ```sh
   cargo test -p nc-nwb
   uv run --no-project --with pynwb --with hdmf-zarr --with nwbinspector \
       tools/python/validate_nwb.py target/nwb-test/small.nwb.zarr --small
   ```
6. **Document and version.** Update `docs/outputs/nwb-mapping.md`; bump `nc-nwb` and its
   changelog.

The NWB version is fixed per release: the schema is vendored in `crates/nwb/specs/` and cached in
every file. Upgrading it is a separate, deliberate change.

## Adding a storage backend

HDF5 was added this way, beside Zarr.

### The trait

```rust
pub trait Backend {
    fn group(&self, path: &str, attrs: Attrs) -> Result<()>;
    fn string(&self, path: &str, value: &str, attrs: Attrs) -> Result<()>;
    fn strings(&self, path: &str, values: &[String], dim: &str, attrs: Attrs) -> Result<()>;
    fn f64_scalar(&self, path: &str, value: f64, attrs: Attrs) -> Result<()>;
    fn f64s(&self, path: &str, values: &[f64], shape: &[u64], dims: &[&str], attrs: Attrs) -> Result<()>;
    fn i64s(&self, path: &str, values: &[i64], dim: &str, attrs: Attrs) -> Result<()>;
    fn u64s(&self, path: &str, values: &[u64], dim: &str, attrs: Attrs) -> Result<()>;
    fn stream(&self, path: &str, shape: &[u64], chunk_rows: u64, ty: SampleType,
              dims: &[&str], attrs: Attrs) -> Result<Box<dyn RowSink>>;
    fn finish(&self) -> Result<()> { Ok(()) }
}

pub trait RowSink: Send + Sync {
    fn write_rows(&self, start: u64, rows: u64, bytes: &[u8]) -> Result<()>;
}
```

- Paths are HDMF paths (`/acquisition/HDEMG/data`).
- Attributes follow hdmf-zarr's conventions: `_DTYPE`, `_ARRAY_DIMENSIONS`, `_LINKS`,
  `_REFERENCE`. A backend translates them to its format (HDF5: soft links and object references).
- `stream` returns a sink that many threads write to at once, in rows along the first dimension.
- `finish` runs after everything is written (HDF5 creates its object references there, once every
  target exists).

### Steps

1. `backend/<name>.rs`: implement `Backend` and `RowSink`.
2. `backend/mod.rs`: add a `Format` variant, recognise it in `Format::of` (from the output name),
   handle it in `with_format` and `create`.
3. `integrity.rs`: read arrays back from the new format (`StoredArray`, the store reader), so the
   content check works. `validate.rs`: structural checks for the format.
4. If it needs a system library, put it behind a feature in `nc-nwb` and pass the feature through
   `nc-convert`, `nc-cli` and `nc-app` (see `hdf5` / `hdf5-static`).
5. Test: `crates/nwb/tests/write.rs` with the new extension, read back with pynwb (or the
   format's reference library), `tools/python/compare <format> --hdf5`-style runs through the
   whole pipeline.

## Another output format

Every output today is NWB. A non-NWB output (another schema, a different file layout) would be a
new crate beside `nc-nwb` that reads the `Session`. `Job` and the app are written for NWB plans; a
second output format needs a change there too.
