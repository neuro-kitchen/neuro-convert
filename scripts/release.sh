#!/usr/bin/env bash
# Builds the release binary — the CLI `neuro-convert` — with every reader and NWB/HDF5 built in,
# and packs it into dist/neuro-convert-<version>-<target>.tar.gz (plus its .sha256).
#
# The desktop app (`nc-app`) is not released yet: it is not built here until it has been reviewed
# for release as a desktop platform.
#
#   scripts/release.sh                      # HDF5 built from source (cmake + C compiler; no system package)
#   NC_HDF5_FEATURE=hdf5 scripts/release.sh # link the system HDF5 instead (dnf install hdf5-devel)
#
# The archive holds bin/, the licenses, README, docs/, the example metadata and VERSIONS.txt
# (`neuro-convert formats`: every reader and crate version in the build).
set -euo pipefail
cd "$(dirname "$0")/.."

feature=${NC_HDF5_FEATURE:-hdf5-static}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/cli/Cargo.toml | head -1)
target=$(rustc -vV | sed -n 's/^host: //p')
name="neuro-convert-$version-$target"
out="dist/$name"

echo "Building $name (HDF5: $feature) …"
cargo build --release --locked -p nc-cli --features "$feature"

rm -rf "$out" "dist/$name.tar.gz" "dist/$name.tar.gz.sha256"
mkdir -p "$out/bin"
cp target/release/neuro-convert "$out/bin/"
cp LICENSE-MIT LICENSE-APACHE README.md "$out/"
cp -r docs "$out/docs"
mkdir -p "$out/metadata" && cp metadata/*.yaml "$out/metadata/"
"$out/bin/neuro-convert" formats > "$out/VERSIONS.txt"
grep -q "HDF5 file" "$out/VERSIONS.txt" || { echo "the CLI was built without HDF5" >&2; exit 1; }

tar -C dist -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "dist/$name.tar.gz"
cat "dist/$name.tar.gz.sha256"
