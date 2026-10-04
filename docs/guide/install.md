# Build and install

neuro-convert is built from source with Cargo. Rust 1.85 or later (edition 2024).

```sh
git clone https://github.com/neuro-kitchen/neuro-convert
cd neuro-convert
```

## Command line

```sh
cargo build --release -p nc-cli
```

The binary is `target/release/neuro-convert`. This build writes Zarr (`.nwb.zarr`).

### HDF5 output

HDF5 (`.nwb`) needs the HDF5 C library. Pick one feature:

| Feature | HDF5 comes from | Needs |
|---|---|---|
| `hdf5-static` | built from source during the build | cmake, a C compiler |
| `hdf5` | the system | `hdf5-devel` (Fedora) / `libhdf5-dev` (Debian, Ubuntu) |

```sh
cargo build --release -p nc-cli --features hdf5-static
```

`neuro-convert formats` lists the outputs the build supports.

### Fewer readers

Every reader is a Cargo feature, all on by default: `tdt`, `spikeglx`, `intan`, `openephys`,
`blackrock`, `neuralynx`. To build with only some:

```sh
cargo build --release -p nc-cli --no-default-features --features spikeglx,openephys
```

## Desktop app

```sh
cargo build --release -p nc-app --features hdf5-static
```

The binary is `target/release/neuro-convert-app`. `cargo build` without `-p` skips the app (GPUI
is a large build).

The app links against xcb, xkbcommon, fontconfig, freetype and Wayland, and needs Vulkan at run
time. The X11 libraries are needed on Wayland too. Fedora:

```sh
sudo dnf install libxcb-devel libxkbcommon-devel libxkbcommon-x11-devel fontconfig-devel freetype-devel wayland-devel vulkan-loader
```

A desktop entry is in `packaging/linux/`.

## Release archive

```sh
scripts/release.sh                       # HDF5 built from source
NC_HDF5_FEATURE=hdf5 scripts/release.sh  # system HDF5
```

Builds the CLI with every reader and HDF5, and packs it with the docs and the example metadata into
`dist/neuro-convert-<version>-<target>.tar.gz` (plus `.sha256`). `VERSIONS.txt` in the archive
lists every reader and crate version.

The desktop app is not in the archive: it is built from source until it has been reviewed for
release.
