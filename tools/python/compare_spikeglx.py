"""Compare neuro-convert's SpikeGLX reader with independent references.

Usage (from the workspace root):
    cargo build --release
    uv run --no-project --with numpy --with probeinterface tools/python/compare_spikeglx.py <ap.bin> [--at 50]

- Values: channel 0 at `--at` seconds, scaled to volts with SpikeGLX's own rule
  (readSGLX.py: imAiRangeMax / imMaxInt (512 when absent) / AP gain from ~imroTbl).
- Positions: every electrode against probeinterface.read_spikeglx.
Exits non-zero on any mismatch.
"""

import json
import subprocess
import sys
from pathlib import Path

import numpy as np
import probeinterface

BIN = "target/release/neuro-convert"


def read_meta(path: Path) -> dict:
    meta = {}
    for line in path.read_text().splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            meta[k.lstrip("~")] = v
    return meta


def main(bin_path: Path, at: float) -> int:
    meta_path = bin_path.with_suffix(".meta")
    meta = read_meta(meta_path)
    problems = []

    ours = json.loads(subprocess.check_output([BIN, "inspect", "--json", "--read-sec", str(at), str(bin_path)]))
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

    for p in problems:
        print("MISMATCH:", p)
    return 1 if problems else 0


if __name__ == "__main__":
    args = sys.argv[1:]
    at = float(args[args.index("--at") + 1]) if "--at" in args else 50.0
    sys.exit(main(Path(args[0]), at))
