"""What every format's comparison shares: running neuro-convert, converting a recording, reading
the NWB back (Zarr or HDF5), turning a series into physical values, matching times."""

import contextlib
import json
import subprocess
from pathlib import Path

import numpy as np

BIN = "target/release/neuro-convert"
OUT = Path("target/compare")
# "zarr" (`.nwb.zarr` stores) or "hdf5" (`.nwb` files; needs a CLI built with `--features hdf5`)
FORMAT = "zarr"

SESSION = "session: { description: comparison with a reference reader, timezone: 'Z', start_time: '2020-01-01T00:00:00' }\n"
TO_SI = {"uV": 1e-6, "µV": 1e-6, "mV": 1e-3, "V": 1.0, "A": 1.0, "uA": 1e-6, "": 1.0}


def inspect_json(path: Path, *extra: str) -> dict:
    """`neuro-convert inspect --json` of `path`."""
    return json.loads(subprocess.check_output([BIN, "inspect", "--json", *extra, str(path)]))


def containers(path: Path) -> list[str]:
    """The containers neuro-convert lists for `path` (blocks, segments, starts), or `[]`."""
    out = subprocess.run([BIN, "inspect", str(path)], capture_output=True, text=True)
    text = out.stdout + out.stderr
    if "--block <name>:" in text:
        return [b.strip() for b in text.split("--block <name>:")[1].strip().splitlines()[0].split(",")]
    return []


def convert(path: Path, block: str | None = None, *, tag: str | None = None, extra_yaml: str = "", args: tuple[str, ...] = ()) -> Path:
    """Converts `path` (container `block`) to `target/compare/<tag>` in the chosen FORMAT and
    returns the output path. `extra_yaml` is appended to the metadata file."""
    OUT.mkdir(parents=True, exist_ok=True)
    tag = (tag or path.name + ("_" + block if block else "")).replace("/", "_").replace(" ", "_")
    meta = OUT / f"{tag}.yaml"
    meta.write_text(SESSION + extra_yaml)
    dest = OUT / f"{tag}{'.nwb' if FORMAT == 'hdf5' else '.nwb.zarr'}"
    cmd = [BIN, "convert", str(path), "-m", str(meta), "-o", str(dest), "--overwrite", "--verify", "sampled", *args]
    if block:
        cmd += ["--block", block]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
    return dest


@contextlib.contextmanager
def open_nwb(dest: Path):
    """The NWB file read with pynwb: hdmf-zarr for a store, h5py for a `.nwb` file."""
    if dest.is_dir():
        from hdmf_zarr import NWBZarrIO

        io = NWBZarrIO(str(dest), mode="r")
    else:
        from pynwb import NWBHDF5IO

        io = NWBHDF5IO(str(dest), mode="r")
    try:
        yield io.read()
    finally:
        io.close()


def values(series) -> np.ndarray:
    """A series' samples in its unit, `[time, channel]` float64 (conversion, per-channel
    conversion and offset applied)."""
    data = np.asarray(series.data[:], dtype=np.float64)
    if data.ndim == 1:
        data = data[:, None]
    cc = getattr(series, "channel_conversion", None)
    scaled = data * series.conversion * (np.asarray(cc[:])[None, :] if cc is not None else 1.0)
    return scaled + (getattr(series, "offset", None) or 0.0)


def close(a, b, tol: float = 1e-6) -> bool:
    a, b = np.asarray(a, dtype=float).ravel(), np.asarray(b, dtype=float).ravel()
    return a.shape == b.shape and np.allclose(a, b, rtol=1e-5, atol=tol, equal_nan=True)


def nearest(times: np.ndarray, t: np.ndarray) -> np.ndarray:
    """Index of the nearest of the sorted `times` for each of `t` (float times differ in their
    last bits between readers)."""
    right = np.clip(np.searchsorted(times, t), 0, len(times) - 1)
    left = np.clip(right - 1, 0, len(times) - 1)
    return np.where(np.abs(times[left] - t) <= np.abs(times[right] - t), left, right)


class Problems(list):
    """Mismatches found; `check(ok, message)` records one when `ok` is false."""

    def check(self, ok: bool, message: str) -> None:
        if not ok:
            self.append(message)
