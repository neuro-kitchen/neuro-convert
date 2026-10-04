"""Intan: every value of every stream against neo's IntanRawIO (same physical unit), and digital
lines' onsets against neo's rising edges, through the whole pipeline. Known difference: neo scales
the RHS DC amplifier with +19.23 mV per step, Intan's own reader (and neuro-convert) with −19.23 mV,
so DC values match neo with the opposite sign. Path: an `.rhd` / `.rhs` file or an RHX folder."""

from pathlib import Path

import numpy as np
from neo.rawio import IntanRawIO

from .common import TO_SI, convert, open_nwb, values

REQUIRES = ["neo"]

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


def neo_file(path: Path) -> Path:
    if path.is_dir():
        for name in ("info.rhd", "info.rhs"):
            if (path / name).is_file():
                return path / name
    return path



def ours(path: Path) -> dict:
    dest = convert(path, tag=path.stem)
    out = {}
    with open_nwb(dest) as nwb:
        for name, series in nwb.acquisition.items():
            out[name] = values(series)
        events = getattr(nwb, "events", None) or {}
        for name, table in events.items():
            out["event:" + name] = np.asarray(table["timestamp"][:], dtype=np.float64)
    return out


def compare(p: Path, opts) -> list[str]:
    problems = []
    if True:
        print(f"=== {p}")
        mine = ours(p)
        r = IntanRawIO(filename=str(neo_file(p)))
        r.parse_header()
        streams = r.header["signal_streams"]
        chans = r.header["signal_channels"]
        rate = r.get_signal_sampling_rate(stream_index=0)
        for name, vals in mine.items():
            if name.startswith("event:"):
                line = name[6:]
                found = [(i, j) for i, st in enumerate(streams) for j, c in enumerate(c for c in chans if c["stream_id"] == st["id"]) if "digital" in st["name"] and line in (c["name"], c["id"])]
                if not found:
                    print(f"  {line}: no matching neo digital channel")
                    continue
                i, j = found[0]
                bits = r.get_analogsignal_chunk(stream_index=i)[:, j].astype(np.int64)
                edges = np.flatnonzero(np.diff(np.concatenate([[0], (bits != 0).astype(np.int64)])) == 1) / r.get_signal_sampling_rate(stream_index=i)
                same = len(edges) == len(vals) and np.allclose(edges, vals)
                print(f"  {line:13} {len(vals)} onsets, neo {len(edges)} rising edges  {'ok' if same else 'MISMATCH'}")
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
            n = min(len(ref), len(vals))
            if ref.shape[1] != vals.shape[1]:
                problems.append(f"{p.name}/{name}: {vals.shape[1]} channels, neo {ref.shape[1]}")
                continue
            if len(ref) != len(vals):
                print(f"  note {name}: {len(vals)} samples, neo {len(ref)} (comparing {n})")
            diff = np.abs(ref[:n] - vals[:n])
            tol = 1e-6 * np.abs(ref[:n]).max() + 1e-12
            worst = diff.max() if diff.size else 0.0
            status = "ok" if worst <= tol else "MISMATCH"
            print(f"  {name:13} {vals.shape[1]:3} ch × {n} samples  max |diff| {worst:.3g} ({units[0]} in neo)  {status}")
            if status != "ok":
                problems.append(f"{p.name}/{name}: max diff {worst:.3g}")
    return problems
