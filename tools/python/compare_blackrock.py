"""Compare neuro-convert's Blackrock reader, through the whole pipeline, with neo's BlackrockRawIO.

Usage (from the workspace root):
    cargo build --release
    uv run --no-project --with neo --with pynwb --with hdmf-zarr tools/python/compare_blackrock.py <base> [<base> ...]

<base> is the path without extension (e.g. data/raw/blackrock/blackrock_3_0/file_spec_3_0). Every
container of the recording (clock segments) is converted to target/blackrock-compare/ and read back
with pynwb. Compared with neo, per NSx file (neo opened with that file only, gap tolerance 0 for
PTP): every sample of every channel (volts; neo's 2.1 reader drops the last sample, so the common
length is compared), the start of every part relative to the first; the spikes (every time and
waveform, per electrode and unit class, offsets per container from the first NSx part); the digital
input words (times and values). Exits non-zero on any mismatch.
"""

import subprocess
import sys
from pathlib import Path

import numpy as np
from hdmf_zarr import NWBZarrIO
from neo.rawio import BlackrockRawIO

BIN = "target/release/neuro-convert"
OUT = Path("target/blackrock-compare")
TO_SI = {"uV": 1e-6, "mV": 1e-3, "V": 1.0}


def containers(base: Path) -> list[str | None]:
    out = subprocess.run([BIN, "inspect", str(base.parent), "--block", base.name], capture_output=True, text=True)
    text = out.stdout + out.stderr
    if "--block <name>:" in text:
        return [b.strip() for b in text.split("--block <name>:")[1].strip().splitlines()[0].split(",") if b.strip().startswith(base.name)]
    out = subprocess.run([BIN, "inspect", str(base.parent)], capture_output=True, text=True)
    text = out.stdout + out.stderr
    if "--block <name>:" in text:
        names = [b.strip() for b in text.split("--block <name>:")[1].strip().splitlines()[0].split(",")]
        mine = [n for n in names if n == base.name or n.startswith(base.name + "/")]
        return mine or names
    return [None]


def convert(base: Path, block: str | None) -> Path:
    OUT.mkdir(parents=True, exist_ok=True)
    tag = (base.name + ("_" + block.replace("/", "_") if block else "")).replace(" ", "_")
    meta = OUT / f"{tag}.yaml"
    meta.write_text("session: { description: Blackrock comparison, timezone: 'Z' }\n")
    dest = OUT / f"{tag}.nwb.zarr"
    src = next(p for p in base.parent.iterdir() if p.stem == base.name and p.suffix[1:].lower() in ("nev", "ns1", "ns2", "ns3", "ns4", "ns5", "ns6"))
    cmd = [BIN, "convert", str(src), "-m", str(meta), "-o", str(dest), "--overwrite", "--verify", "sampled"]
    if block:
        cmd += ["--block", block]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
    return dest


def series_values(s) -> np.ndarray:
    data = np.asarray(s.data[:]).astype("f8") * s.conversion + getattr(s, "offset", 0.0)
    if getattr(s, "channel_conversion", None) is not None:
        data = data * np.asarray(s.channel_conversion)[None, :]
    return data


def main(base: Path) -> list[str]:
    problems = []
    blocks = containers(base)
    stores = [(b, convert(base, b)) for b in blocks]
    ios = [NWBZarrIO(str(d), mode="r") for _, d in stores]
    nwbs = [io.read() for io in ios]
    nsx = sorted(int(p.suffix[3:]) for p in base.parent.iterdir() if p.stem == base.name and p.suffix[1:3].lower() == "ns" and p.suffix[3:].isdigit())

    first_part_start = {}  # (container, nsx) -> (ours start, neo t_start)
    for n in nsx:
        neo = BlackrockRawIO(filename=str(base), nsx_to_load=n, gap_tolerance_ms=0.0)
        neo.parse_header()
        chans = neo.header["signal_channels"]
        units = [u.strip() for u in chans["units"]]
        si = np.array([TO_SI.get(u, 1.0) for u in units])
        electrical = [i for i, u in enumerate(units) if u == "uV"]
        analog = [i for i in range(len(units)) if i not in electrical]
        order = electrical + analog
        # Our parts in order: per container, ns<n>[.p<k>] (+ .analog)
        ours = []
        for c, nwb in enumerate(nwbs):
            names = sorted({k.replace(".analog", "") for k in nwb.acquisition if k == f"ns{n}" or k.startswith(f"ns{n}.")}, key=lambda k: int(k.rsplit(".p", 1)[1]) if ".p" in k else 0)
            for name in names:
                parts = [nwb.acquisition[k] for k in (name, name.replace(f"ns{n}", f"ns{n}.analog")) if k in nwb.acquisition]
                values = np.hstack([series_values(p) for p in parts])
                ours.append((c, name, parts[0].starting_time, values))
        segs = neo.segment_count(0)
        if segs != len(ours):
            problems.append(f"ns{n}: {len(ours)} parts here, {segs} segments in neo")
        compared = 0
        for k, (c, name, start, values) in enumerate(ours[:segs]):
            raw = neo.get_analogsignal_chunk(0, k, stream_index=0).astype("f8")
            ref = (raw * chans["gain"][None, :] + chans["offset"][None, :]) * si[None, :]
            ref = ref[:, order]
            m = min(len(ref), len(values))
            if abs(len(ref) - len(values)) > 1:
                problems.append(f"ns{n} {name}: {len(values)} samples, neo {len(ref)}")
            bad = ~np.isclose(values[:m], ref[:m], rtol=1e-6, atol=1e-9)
            if bad.any():
                i, j = np.argwhere(bad)[0]
                problems.append(f"ns{n} {name}: {bad.sum()} values differ (first sample {i} ch {j}: {values[i, j]} vs {ref[i, j]})")
            t = neo.get_signal_t_start(0, k, 0)
            first_part_start.setdefault((c, n), (start, t))
            s0, t0 = first_part_start[(c, n)]
            if not np.isclose(start - s0, t - t0, atol=1e-6):
                problems.append(f"ns{n} {name}: starts {start - s0:.6f} s after the first part, neo {t - t0:.6f} s")
            compared += m * values.shape[1]
        print(f"{base.name} ns{n}: {len(ours)} parts, {compared} values compared")

    # Spikes and digital input (neo with the first NSx file, or the NEV alone)
    neo = BlackrockRawIO(filename=str(base), nsx_to_load=nsx[0] if nsx else None, gap_tolerance_ms=0.0)
    neo.parse_header()
    offsets = {c: t - s for (c, n), (s, t) in first_part_start.items() if n == (nsx[0] if nsx else None)}
    our_spikes = {}
    for c, nwb in enumerate(nwbs):
        off = offsets.get(c, 0.0)
        for key, s in nwb.acquisition.items():
            if not key.startswith("spikes_ch"):
                continue
            ch = int(key.rsplit("ch", 1)[1])
            our_spikes.setdefault(ch, []).append((np.asarray(s.timestamps[:]) + off, np.asarray(s.data[:])[:, 0, :].astype("f8") * s.conversion))
        units = nwb.units.to_dataframe() if nwb.units is not None else None
    our_spikes = {ch: (np.concatenate([t for t, _ in v]), np.concatenate([w for _, w in v])) for ch, v in our_spikes.items()}
    total = 0
    for u in range(neo.spike_channels_count()):
        name = neo.header["spike_channels"][u]["name"]
        ch = int(name.split("#")[0][2:])
        t, w = [], []
        for seg in range(neo.segment_count(0)):
            ts = neo.get_spike_timestamps(0, seg, u, None, None)
            t.append(neo.rescale_spike_timestamp(ts, "float64"))
            wf = neo.get_spike_raw_waveforms(0, seg, u, None, None).astype("f8")
            w.append(wf[:, 0, :] * neo.header["spike_channels"][u]["wf_gain"] * 1e-6)
        t, w = np.concatenate(t), np.concatenate(w)
        if ch not in our_spikes:
            problems.append(f"spikes ch{ch}: missing here")
            continue
        times, waves = our_spikes[ch]
        for ti, wi in zip(t, w):
            j = np.flatnonzero(np.isclose(times, ti, atol=2e-6))
            if not any(np.allclose(waves[k][: len(wi)], wi, rtol=1e-5, atol=1e-12) for k in j):
                problems.append(f"spikes {name}: spike at {ti:.6f} s not found with the same waveform")
                break
        total += len(t)
    ours_total = sum(len(t) for t, _ in our_spikes.values())
    if ours_total != total:
        problems.append(f"spikes: {ours_total} here, {total} in neo")
    print(f"{base.name}: {total} spikes compared")

    for e in range(neo.event_channels_count()):
        name = neo.header["event_channels"][e]["name"]
        if name != "digital_input_port":
            continue
        t, v = [], []
        for seg in range(neo.segment_count(0)):
            ts, _, labels = neo.get_event_timestamps(0, seg, e, None, None)
            t.append(neo.rescale_event_timestamp(ts, "float64", e))
            v.append(np.array([float(x) for x in labels]))
        t, v = np.concatenate(t), np.concatenate(v)
        mine_t, mine_v = [], []
        for c, nwb in enumerate(nwbs):
            ev = getattr(nwb, "events", None)
            if ev is not None and "digital_input" in ev:
                mine_t.append(np.asarray(ev["digital_input"]["timestamp"][:]) + offsets.get(c, 0.0))
                mine_v.append(np.asarray(ev["digital_input"]["value"][:]))
        mine_t = np.concatenate(mine_t) if mine_t else np.array([])
        mine_v = np.concatenate(mine_v) if mine_v else np.array([])
        if len(mine_t) != len(t) or not np.allclose(mine_t, t, atol=2e-6) or not np.array_equal(mine_v, v):
            problems.append(f"digital input: {len(mine_t)} words here, {len(t)} in neo, or times / values differ")
        print(f"{base.name}: {len(t)} digital input words compared")
    for io in ios:
        io.close()
    return problems


if __name__ == "__main__":
    all_problems = []
    for arg in sys.argv[1:]:
        all_problems += main(Path(arg))
    for p in all_problems:
        print("MISMATCH:", p)
    sys.exit(1 if all_problems else 0)
