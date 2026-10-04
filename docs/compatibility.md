# Compatibility

What each part of neuro-convert reads and writes, which tools read its output, and how changes are
announced. `neuro-convert formats` prints the reader and crate versions of a build; every
conversion records them (report `versions`, NWB `/general/source_script`).

## Readers (crate version 0.1.0, 2026-10-02)

| Reader | Maturity | Source formats and versions | Reference it is compared with |
|---|---|---|---|
| `tdt` | verified | Synapse (Notes.txt, StoresListing.txt, .tin) and OpenEx (.tnt) blocks and tanks; TEV + SEV v0–v3 streams, rawpacked, snips + offline sorts, epocs, scalars, notes | `tdt.read_block` (tdt 0.7.6) |
| `spikeglx` | verified | Neuropixels 3A, 1.0 family, 2.0 (AP / LF / sync); NI-DAQ (MN / MA / XA, DW); OneBox (XA / XD / SY); multi-trigger gates, CatGT `_tcat`; sync-pulse alignment | SpikeGLX's conversion rule (readSGLX.py), probeinterface 0.4.0, neo 0.14.5 |
| `intan` | verified | RHD and RHS 1.0–3.x; traditional file, one file per signal type, one file per channel | neo 0.14.5 `IntanRawIO` |
| `openephys` | verified | Binary format GUI 0.4.4 – 0.6+; legacy `.continuous` / `.events` / `.spikes` (GUI ≤ 0.4.x) | neo 0.14.5 `OpenEphysBinaryRawIO`, `OpenEphysRawIO` |
| `blackrock` | verified | NSx / NEV file spec 2.1, 2.2, 2.3, 3.0, 3.0 with PTP timestamps | neo 0.14.5 `BlackrockRawIO` |
| `neuralynx` | verified | Cheetah 1.x – 6.x, Pegasus 2.x, BML, Neuraview (`.ncs`, `.nse` / `.nst` / `.ntt`, `.nev`) | neo 0.14.5 `NeuralynxRawIO` |

Maturity: **verified** = the reference comparison (`tools/python/compare <format>`) passes on every
data set of its format in `tools/python/testdata.toml`; **experimental** = works on its fixtures,
not yet compared on real data; **community** = maintained outside neuro-convert. Known differences
with the references are listed on each format's page (`docs/formats/`).

## Output

| Output | Layout | Read and checked with |
|---|---|---|
| NWB 2.11.0 (hdmf-common 1.10.0, hdmf-experimental 0.6.0), Zarr v3 store `.nwb.zarr` | hdmf-zarr's layout; schema cached under `/specifications` | pynwb 4.2.0 + hdmf 6.2.0 + hdmf-zarr 0.14.0 + zarr 3.4.0 (hdmf-zarr ≤ 0.13 cannot read Zarr v3) |
| NWB 2.11.0, HDF5 file `.nwb` (builds with `hdf5` / `hdf5-static`) | pynwb's layout (links, object references) | pynwb 4.2.0 + h5py 3.16.0; `pynwb.validate` clean; nwbinspector 0.7.2 (only DANDI metadata suggestions remain) |

nwbinspector 0.7.2 crashes in its compression checks on Zarr v3 stores (upstream); check HDF5
output with it, or the Zarr store's structure with `neuro-convert validate`.

## Versions and changes
- Every crate has its own version (`<crate>::version()`), bumped when its behaviour or API changes,
  with an entry in the crate's `CHANGELOG.md`. A change in what a conversion writes is always a
  version bump of the crate that changed, so files can be traced to the code that wrote them.
- `nc-core` holds the contract outside readers depend on (`Reader`, the `Session` model,
  `Waveforms`, the metadata file). A breaking change there is announced one release ahead: the
  old item is marked `#[deprecated]` with what replaces it and kept for that release, and the
  change is listed in `crates/core/CHANGELOG.md`. Additions that keep old readers compiling
  (a new trait method with a default, a new optional model field) are not breaking.
- The NWB target version is one per release, upgraded deliberately (the schema is vendored in
  `crates/nwb/specs/` and cached in every file).
