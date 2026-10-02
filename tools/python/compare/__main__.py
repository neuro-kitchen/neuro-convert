"""Compare neuro-convert with a reference reader, format by format.

Usage (from the workspace root, after `cargo build --release`):

    uv run --no-project --with neo --with pynwb --with hdmf-zarr --with tdt --with probeinterface \\
        python tools/python/compare <format> <path> [<path> ...] [--block NAME] [--at SECONDS] [--hdf5]

Formats and what a path is:
    tdt               a block folder                       (reference: tdt.read_block)
    spikeglx          an .ap.bin file                      (SpikeGLX's conversion rule, probeinterface)
    intan             an .rhd / .rhs file or RHX folder    (neo IntanRawIO)
    openephys         a binary-format save folder          (neo OpenEphysBinaryRawIO)
    openephys-legacy  a folder of .continuous files        (neo OpenEphysRawIO)
    blackrock         a base path without extension        (neo BlackrockRawIO)
    neuralynx         a session folder                     (neo NeuralynxRawIO)

Every format except tdt and spikeglx converts through the whole pipeline (to target/compare/) and
reads the NWB back with pynwb; `--hdf5` writes `.nwb` files instead of Zarr stores (needs a CLI
built with `--features hdf5` or `hdf5-static`). `--block` picks one container (default: every
one). Test data: `tools/python/fetch_gin.py --format <format>`. Exits non-zero on any mismatch;
known differences with the references are explained in docs/formats/<format>.md.
"""

import argparse
import importlib
import sys
from pathlib import Path

# Run as a folder, Python puts this folder first on the path, where `tdt.py` would shadow the `tdt`
# package: import the comparisons as the `compare` package instead
sys.path[0] = str(Path(__file__).resolve().parent.parent)

FORMATS = {
    "tdt": "tdt",
    "spikeglx": "spikeglx",
    "intan": "intan",
    "openephys": "openephys",
    "openephys-legacy": "openephys_legacy",
    "blackrock": "blackrock",
    "neuralynx": "neuralynx",
}


def main() -> int:
    p = argparse.ArgumentParser(prog="compare", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("format", choices=sorted(FORMATS))
    p.add_argument("paths", nargs="+", type=Path)
    p.add_argument("--block", help="one container (block, segment, start) instead of every one")
    p.add_argument("--at", type=float, default=50.0, help="spikeglx: where the values are compared (s)")
    p.add_argument("--hdf5", action="store_true", help="convert to NWB/HDF5 (.nwb) instead of Zarr")
    opts = p.parse_args()

    from compare import common

    common.FORMAT = "hdf5" if opts.hdf5 else "zarr"
    module = importlib.import_module(f"compare.{FORMATS[opts.format]}")
    problems = []
    for path in opts.paths:
        print(f"=== {opts.format}: {path}")
        found = module.compare(path, opts)
        problems += [f"{path}: {m}" for m in found]
        print(f"{path}: {'ok' if not found else f'{len(found)} mismatches'}")
    for m in problems:
        print("MISMATCH:", m)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
