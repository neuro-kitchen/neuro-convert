"""Open Ephys binary format: every value of every series against neo's OpenEphysBinaryRawIO
(streams matched by values), relative start times (GUI ≥ 0.6: against the synchronized
`timestamps.npy`, which neuro-convert uses and neo does not), and the number of TTL periods.
Known difference: neo labels non-electrode channels without units, or with the file's "uV" (ADC),
as microvolts; their `bit_volts` are volts (Open Ephys's documentation), which neuro-convert uses,
so those match neo × 1e-6 ("V vs neo uV"). Path: a save folder; every recording is compared, or
`--block` one."""

from pathlib import Path

import numpy as np
from neo.rawio import OpenEphysBinaryRawIO

from .common import TO_SI, containers, convert, open_nwb, values

REQUIRES = ["neo"]


def compare(path: Path, opts) -> list[str]:
    problems = []
    for block in [opts.block] if opts.block else (containers(path) or [None]):
        print(f"=== {path} {block or ''}")
        dest = convert(path, block)
        neo_dir = path / block.split("/")[0] if block else path
        r = OpenEphysBinaryRawIO(dirname=str(neo_dir))
        r.parse_header()
        seg = int(block.split("recording")[-1]) - 1 if block else 0
        streams = r.header["signal_streams"]
        chans = r.header["signal_channels"]
        neo_by_names = {}
        for si, st in enumerate(streams):
            cs = [c for c in chans if c["stream_id"] == st["id"]]
            neo_by_names[tuple(c["name"] for c in cs)] = (si, cs)
        starts = {}
        with open_nwb(dest) as nwb:
            for name, series in nwb.acquisition.items():
                if not hasattr(series, "data") or not hasattr(series, "rate") or series.rate is None:
                    continue
                ours = values(series)
                # Streams are matched by channel count and values (names differ between readers)
                cand = [(k, v) for k, v in neo_by_names.items() if len(k) == ours.shape[1]]
                best = None
                for k, (si, cs) in cand:
                    raw = r.get_analogsignal_chunk(seg_index=seg, stream_index=si)
                    ref = r.rescale_signal_raw_to_float(raw, dtype="float64", stream_index=si) * TO_SI.get(cs[0]["units"], 1.0)
                    n = min(len(ref), len(ours))
                    for factor, why in [(1.0, ""), (1e6, " (V vs neo uV)")]:
                        diff = np.abs(ref[:n] * factor - ours[:n]).max()
                        if diff <= 1e-6 * np.abs(ours[:n]).max() + 1e-15:
                            best = (st_name := streams[si]["name"], why, n)
                            starts[name] = (series.starting_time, r.get_signal_t_start(block_index=0, seg_index=seg, stream_index=si))
                            break
                    if best:
                        break
                if best:
                    print(f"  {name:22} {ours.shape[1]:3} ch × {best[2]} samples = neo {best[0]}{best[1]}")
                else:
                    problems.append(f"{path.name}/{name}: no neo stream with the same values ({len(cand)} candidates)")
            # Relative start times. GUI >= 0.6 writes synchronized seconds (timestamps.npy) that place
            # streams of different devices on one clock: neuro-convert uses them, neo uses each
            # device's own sample numbers, so for those recordings check against the files instead.
            rec = sorted((path / block if block else path).rglob("structure.oebin"))[0].parent
            synced = {}
            for name in starts:
                base = name.rsplit(".", 1)[0] if name.endswith((".sync", ".analog")) else name
                d = [x for x in (rec / "continuous").iterdir() if x.name.endswith(base)]
                if d and (d[0] / "sample_numbers.npy").exists():
                    synced[name] = float(np.load(d[0] / "timestamps.npy")[0])
            if synced:
                starts = {k: (v[0], synced[k]) for k, v in starts.items() if k in synced}
                print("  (start times checked against the GUI's synchronized timestamps.npy)")
            if len(starts) > 1:
                o0 = min(v[0] for v in starts.values())
                n0 = min(v[1] for v in starts.values())
                for name, (o, n_) in starts.items():
                    d = abs((o - o0) - (n_ - n0))
                    print(f"  start {name:16} {o - o0:.6f} s (reference {n_ - n0:.6f})  {'ok' if d < 1e-4 else 'MISMATCH'}")
                    if d >= 1e-4:
                        problems.append(f"{path.name}/{name}: start differs by {d:.6f} s")
            # TTL onsets
            events = getattr(nwb, "events", None) or {}
            ev_count = r.event_channels_count() if hasattr(r, "event_channels_count") else len(r.header["event_channels"])
            n_on = {name: len(t["timestamp"][:]) for name, t in events.items() if "TTL" in name}
            neo_ttl = 0
            for ei in range(ev_count):
                ts, dur, labels = r.get_event_timestamps(block_index=0, seg_index=seg, event_channel_index=ei)
                if dur is not None and len(ts):
                    neo_ttl += len(ts)
            if n_on:
                ours_total = sum(n_on.values())
                print(f"  TTL periods: ours {ours_total}, neo {neo_ttl}  {'ok' if ours_total == neo_ttl else 'MISMATCH'}")
                if ours_total != neo_ttl:
                    problems.append(f"{path.name}: TTL counts differ")
    return problems
