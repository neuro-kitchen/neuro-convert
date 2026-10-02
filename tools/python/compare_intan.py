"""Compare neuro-convert's Intan reader, through the whole pipeline, with neo's IntanRawIO.

Usage (from the workspace root):
    cargo build --release
    uv run --no-project --with neo --with pynwb --with hdmf-zarr tools/python/compare_intan.py <recording> [...]

Each recording (an .rhd / .rhs file or an RHX folder) is converted with `neuro-convert convert`
to target/intan-compare/<name>.nwb.zarr, read back with pynwb, and every value of every stream is
compared with neo's rescaled signals (same physical unit). Known difference: neo scales the RHS DC
amplifier with +19.23 mV per step, Intan's own reader (and neuro-convert) with −19.23 mV, so DC
values are expected to match neo with the opposite sign. Exits non-zero on any other mismatch.
"""

import subprocess
import sys
from pathlib import Path

import numpy as np
from hdmf_zarr import NWBZarrIO
from neo.rawio import IntanRawIO

BIN = "target/release/neuro-convert"
OUT = Path("target/intan-compare")

# our stream → (neo stream name suffix, factor from neo's unit to ours, expected sign)
STREAMS = {
    "amplifier": ("amplifier channel", None, 1.0),
    "aux": ("auxiliary input channel", None, 1.0),
    "supply": ("supply voltage channel", None, 1.0),
    "analog_in": ("ADC input channel", None, 1.0),
    "analog_out": ("ADC output channel", None, 1.0),
    "dc_amplifier": ("DC Amplifier channel", None, -1.0),
    "stim": ("Stim channel", None, 1.0),
}
TO_SI = {"uV": 1e-6, "mV": 1e-3, "V": 1.0, "A": 1.0, "uA": 1e-6}


def neo_file(path: Path) -> Path:
    if path.is_dir():
        for name in ("info.rhd", "info.rhs"):
            if (path / name).is_file():
                return path / name
    return path


def ours(path: Path) -> dict:
    OUT.mkdir(parents=True, exist_ok=True)
    meta = OUT / f"{path.stem}.yaml"
    meta.write_text("session: { description: Intan comparison, timezone: 'Z', start_time: '2020-01-01T00:00:00' }\n")
    dest = OUT / f"{path.stem}.nwb.zarr"
    subprocess.run([BIN, "convert", str(path), "-m", str(meta), "-o", str(dest), "--overwrite", "--verify", "sampled"], check=True, stdout=subprocess.DEVNULL)
    out = {}
    with NWBZarrIO(str(dest), "r") as io:
        nwb = io.read()
        for name, series in nwb.acquisition.items():
            data = np.asarray(series.data[:], dtype=np.float64)
            if data.ndim == 1:
                data = data[:, None]
            scale = series.conversion
            cc = getattr(series, "channel_conversion", None)
            values = data * scale * (np.asarray(cc[:])[None, :] if cc is not None else 1.0)
            out[name] = values + (series.offset or 0.0)
        events = getattr(nwb, "events", None) or {}
        for name, table in events.items():
            out["event:" + name] = np.asarray(table["timestamp"][:], dtype=np.float64)
    return out


def main(paths: list[str]) -> int:
    problems = []
    for p in map(Path, paths):
        print(f"=== {p}")
        mine = ours(p)
        r = IntanRawIO(filename=str(neo_file(p)))
        r.parse_header()
        streams = r.header["signal_streams"]
        chans = r.header["signal_channels"]
        rate = r.get_signal_sampling_rate(stream_index=0)
        for name, values in mine.items():
            if name.startswith("event:"):
                line = name[6:]
                found = [(i, j) for i, st in enumerate(streams) for j, c in enumerate(c for c in chans if c["stream_id"] == st["id"]) if "digital" in st["name"] and line in (c["name"], c["id"])]
                if not found:
                    print(f"  {line}: no matching neo digital channel")
                    continue
                i, j = found[0]
                bits = r.get_analogsignal_chunk(stream_index=i)[:, j].astype(np.int64)
                edges = np.flatnonzero(np.diff(np.concatenate([[0], (bits != 0).astype(np.int64)])) == 1) / r.get_signal_sampling_rate(stream_index=i)
                same = len(edges) == len(values) and np.allclose(edges, values)
                print(f"  {line:13} {len(values)} onsets, neo {len(edges)} rising edges  {'ok' if same else 'MISMATCH'}")
                if not same:
                    problems.append(f"{p.name}/{line}: onsets differ")
                continue
            if name not in STREAMS:
                continue
            suffix, _, sign = STREAMS[name]
            idx = [i for i, s in enumerate(streams) if s["name"].endswith(suffix)]
            if not idx:
                problems.append(f"{p.name}/{name}: neo has no stream ending in {suffix!r} ({[s['name'] for s in streams]})")
                continue
            i = idx[0]
            raw = r.get_analogsignal_chunk(stream_index=i)
            ref = r.rescale_signal_raw_to_float(raw, dtype="float64", stream_index=i)
            units = [c["units"] for c in chans if c["stream_id"] == streams[i]["id"]]
            ref = ref * TO_SI.get(units[0], 1.0) * sign
            n = min(len(ref), len(values))
            if ref.shape[1] != values.shape[1]:
                problems.append(f"{p.name}/{name}: {values.shape[1]} channels, neo {ref.shape[1]}")
                continue
            if len(ref) != len(values):
                print(f"  note {name}: {len(values)} samples, neo {len(ref)} (comparing {n})")
            diff = np.abs(ref[:n] - values[:n])
            tol = 1e-6 * np.abs(ref[:n]).max() + 1e-12
            worst = diff.max() if diff.size else 0.0
            status = "ok" if worst <= tol else "MISMATCH"
            print(f"  {name:13} {values.shape[1]:3} ch × {n} samples  max |diff| {worst:.3g} ({units[0]} in neo)  {status}")
            if status != "ok":
                problems.append(f"{p.name}/{name}: max diff {worst:.3g}")
    for msg in problems:
        print("PROBLEM:", msg)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
