## What and why

<!-- What this changes and why; link the issue it closes (Closes #…). -->

## Checks

- [ ] `cargo test` passes (and `cargo test -p nc-app` if the app changed)
- [ ] HDF5 output built and tested if the NWB writer changed: `--features hdf5-static`
- [ ] Reader changes compared with the reference reader: `tools/python/compare <format> <path>…` (data sets used: <!-- … -->)
- [ ] Output read back with pynwb / validated: `neuro-convert validate`, `neuro-convert verify`
- [ ] Docs (`docs/formats/`, `docs/outputs/nwb-mapping.md`) and the crate `CHANGELOG.md` / version updated if what is read or how it maps changed

## Data and licences

- [ ] No recordings, subject data or credentials committed; tests use synthetic fixtures or `data/raw/`
- [ ] Code ported from elsewhere keeps its upstream notice and a compatible licence (see CONTRIBUTING.md)
