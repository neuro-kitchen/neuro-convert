# API reference

The API reference is generated from the code with `cargo doc`.

```sh
cargo doc --no-deps --workspace --exclude nc-app --open
```

Pages go to `target/doc/`. Add `--features nc-nwb/hdf5-static` to include the HDF5 backend.

## Where to start

| To | Start at |
|---|---|
| Write a reader | `nc_core::Reader`, `nc_core::Session`, `nc_core::Recording`, `nc_core::testkit::check_reader` |
| Run a conversion from Rust | `nc_convert::Registry`, `nc_convert::Job`, `nc_core::MetadataFile` |
| Use an outside reader | `nc_convert::Registry::with` |
| Change the NWB output | `nc_nwb::plan`, `nc_nwb::mapping::NwbPlan`, `nc_nwb::write` |
| Add a storage backend | `nc_nwb::backend::Backend`, `nc_nwb::backend::RowSink` |
| Check a written file | `nc_nwb::validate`, `nc_nwb::integrity` |

## Example: convert from Rust

```rust
use nc_convert::{nwb::NwbOptions, CancelToken, Job, MetadataFile, OpenOptions, Registry};

let registry = Registry::builtin();
let mut job = Job::open(&registry, "data/rec".as_ref(), &OpenOptions::default())?;
let meta = MetadataFile::load("meta.yaml".as_ref())?;
let plan = job.plan(&meta);
for issue in &plan.issues {
    eprintln!("{:?}: {}", issue.level, issue.message);
}
let report = job.write("rec.nwb.zarr".as_ref(), &NwbOptions::default(), &CancelToken::new(), &|_| {})?;
report.save(&report.default_path())?;
```

## Example: an outside reader

```rust
let registry = nc_convert::Registry::builtin().with(MyReader);
```

`MyReader` implements `nc_core::Reader` in its own crate; nothing in neuro-convert changes.
