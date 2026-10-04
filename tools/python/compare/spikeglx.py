"""SpikeGLX: values against SpikeGLX's own conversion rule (readSGLX.py: `imAiRangeMax /
imMaxInt` (512 when absent) / AP gain — `imChan0apGain` when present, else 80 for the NP2.0
family, else the IMRO entry's AP gain) for channel 0 at `--at` seconds, and every electrode's
position against probeinterface (identical geometry up to one constant origin offset per axis).
Path: an `.ap.bin` file."""

from pathlib import Path

import numpy as np
import probeinterface

from .common import inspect_json

REQUIRES = ["probeinterface"]


def read_meta(path: Path) -> dict:
    meta = {}
    for line in path.read_text().splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            meta[k.lstrip("~")] = v
    return meta


def compare(bin_path: Path, opts) -> list[str]:
    at = opts.at
    meta_path = bin_path.with_suffix(".meta")
    meta = read_meta(meta_path)
    problems = []

    ours = inspect_json(bin_path, "--read-sec", str(at))
    ap = next(r for r in ours["recordings"] if r["name"].endswith(".ap"))

    # Reference values: SpikeGLX's conversion of channel 0
    n_saved = int(meta["nSavedChans"])
    raw = np.memmap(bin_path, dtype="<i2", mode="r").reshape(-1, n_saved)
    max_int = int(meta.get("imMaxInt", 512))
    # readSGLX.py: `imChan0apGain` when the metadata has it (SpikeGLX ≥ 2023); else NP2.0
    # family fixed 80; NP1 / 3A: the IMRO entry's AP gain
    np2 = int(meta.get("imDatPrb_type", 0)) in (21, 24, 2003, 2004, 2005, 2006, 2013, 2014, 2020, 2021)
    if "imChan0apGain" in meta:
        ap_gain = float(meta["imChan0apGain"])
    else:
        ap_gain = 80.0 if np2 else float(meta["imroTbl"].split(")(")[1].split()[3])
    volts = float(meta["imAiRangeMax"]) / max_int / ap_gain
    first = ap["first_sample"]
    ref = raw[first : first + len(ap["values"]), 0] * volts
    if not np.allclose(ap["values"], ref, rtol=1e-6, atol=1e-12):
        problems.append(f"values at sample {first}: ours {ap['values']} vs SpikeGLX {ref.tolist()}")
    print(f"values  : ch0 from sample {first}: {np.round(ref * 1e6, 3).tolist()} µV  ({'OK' if not problems else 'MISMATCH'})")

    # Reference positions: probeinterface. Origins differ by convention (neuro-convert follows
    # SpikeGLX / SGLXMetaToCoords: x from the shank edge; probeinterface: x from the leftmost
    # site), so the geometry must match up to one constant offset per axis.
    probe = probeinterface.read_spikeglx(meta_path)
    ref_pos = np.asarray(probe.contact_positions)
    our_pos = np.array([e["position_um"][:2] for e in ours["electrodes"]])
    ok = our_pos.shape == ref_pos.shape
    offset = (our_pos - ref_pos)[0] if ok else None
    if not ok or not np.allclose(our_pos - ref_pos, offset):
        problems.append(f"positions differ beyond a constant offset (shapes {our_pos.shape} vs {ref_pos.shape})")
        print(f"position: {len(our_pos)} electrodes vs probeinterface: MISMATCH")
    else:
        print(f"position: {len(our_pos)} electrodes match probeinterface {probe.model_name!r} up to origin offset (x, y) = {offset.tolist()} µm (OK)")

    return problems
