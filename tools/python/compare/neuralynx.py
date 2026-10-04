"""Neuralynx: streams matched by channel names; per segment (neo) / part (here) every sample of
every channel (volts) and its start time; every spike time and waveform per wire (in the file's
wire order) and unit; every event time per (event id, TTL). neo reports the stated sample rate;
neuro-convert stores the measured one (as neo uses for its own timing), so rates are reported, not
compared. When neo refuses a folder (streams with different gaps) each `.ncs` file is compared on
its own. Path: a session folder."""

from pathlib import Path

import numpy as np
from neo.rawio import NeuralynxRawIO

from .common import convert, nearest, open_nwb

REQUIRES = ["neo"]


def compare(folder: Path, opts, files: list[str] | None = None, dest: Path | None = None) -> list[str]:
    problems = []
    dest = dest or convert(folder, opts.block, tag=folder.as_posix().strip("/"))
    try:
        neo = NeuralynxRawIO(dirname=str(folder), include_filenames=files) if files else NeuralynxRawIO(dirname=str(folder))
        neo.parse_header()
    except ValueError as e:
        if files or "Incompatible section structures" not in str(e):
            raise
        # neo refuses streams whose gaps differ: compare one .ncs file at a time
        print(f"{folder.name}: neo refuses the folder ({e}); comparing each .ncs file on its own")
        for f in sorted(p.name for p in folder.iterdir() if p.suffix.lower() == ".ncs"):
            problems += compare(folder, opts, [f], dest)
        return problems
    chans = neo.header["signal_channels"]
    with open_nwb(dest) as nwb:
        series = {k: v for k, v in nwb.acquisition.items() if hasattr(v, "rate") and v.rate}
        compared = 0
        for si, stream in enumerate(neo.header["signal_streams"]):
            mask = chans["stream_id"] == stream["id"]
            names = list(chans["name"][mask])
            gains = chans["gain"][mask] * 1e-6
            # Our series of this stream: the ones whose electrodes carry these channel names
            mine = sorted(series, key=lambda k: (k.split(".p")[0], int(k.split(".p")[1]) if ".p" in k else 0))
            candidates = {}
            for k in mine:
                rows = np.asarray(series[k].electrodes.data[:])
                labels = [nwb.electrodes["channel_name"][int(r)] for r in rows]
                candidates.setdefault(k.split(".p")[0], (labels, []))[1].append(k)
            match = [base for base, (labels, _) in candidates.items() if labels == names]
            # A header without `-AcqEntName`: neo says "unknown", here the file name is used
            if not match and names == ["unknown"]:
                match = [base for base, (labels, _) in candidates.items() if len(labels) == 1 and abs(series[candidates[base][1][0]].rate - neo.get_signal_sampling_rate(si)) / neo.get_signal_sampling_rate(si) < 0.05]
            if not match:
                problems.append(f"stream {stream['name']} ({names}): no series here with these channels")
                continue
            parts = candidates[match[0]][1]
            if len(parts) != neo.segment_count(0):
                problems.append(f"{match[0]}: {len(parts)} parts here, {neo.segment_count(0)} segments in neo")
            for seg, key in enumerate(parts[: neo.segment_count(0)]):
                s = series[key]
                ours = np.asarray(s.data[:]).astype("f8") * s.conversion
                if s.channel_conversion is not None:
                    ours = ours * np.asarray(s.channel_conversion)[None, :]
                raw = neo.get_analogsignal_chunk(0, seg, stream_index=si).astype("f8")
                ref = raw * gains[None, :]
                if ours.shape != ref.shape:
                    problems.append(f"{key}: shape {ours.shape} vs neo {ref.shape}")
                    m = min(len(ours), len(ref))
                    ours, ref = ours[:m], ref[:m]
                bad = ~np.isclose(ours, ref, rtol=1e-6, atol=1e-12)
                if bad.any():
                    i, j = np.argwhere(bad)[0]
                    problems.append(f"{key}: {bad.sum()} values differ (first sample {i} ch {j}: {ours[i, j]} vs {ref[i, j]})")
                t = neo.get_signal_t_start(0, seg, si)
                # One file at a time, neo's zero is that file's start: compare relative starts
                here = s.starting_time - (series[parts[0]].starting_time if files else 0.0)
                if not np.isclose(here, t - (neo.get_signal_t_start(0, 0, si) if files else 0.0), atol=1e-6):
                    problems.append(f"{key}: starts at {s.starting_time:.6f} s, neo {t:.6f} s")
                compared += ours.size
            print(f"{folder.name} {match[0]}: {len(parts)} parts, {compared} values compared (rate here {series[parts[0]].rate:.4f} Hz, neo {neo.get_signal_sampling_rate(si):.4f} Hz)")

        # Spikes per unit (neo) → our store of that file, wire by wire
        total = 0
        for u in range(neo.spike_channels_count()):
            name = neo.header["spike_channels"][u]["name"]  # ch<name>#<id>#<unit>
            entity, chan_id, unit = name[2:].rsplit("#", 2)
            gain = neo.header["spike_channels"][u]["wf_gain"] * 1e-6
            t, w = [], []
            for seg in range(neo.segment_count(0)):
                ts = neo.get_spike_timestamps(0, seg, u, None, None)
                t.append(neo.rescale_spike_timestamp(ts, "float64"))
                w.append(neo.get_spike_raw_waveforms(0, seg, u, None, None).astype("f8") * gain)
            t, w = np.concatenate(t), np.concatenate(w)
            wires = [k for k in nwb.acquisition if k.startswith(f"{entity}_ch")]
            if not wires:
                problems.append(f"spikes {entity}: missing here")
                continue
            # Wires in the file's own order (`-ADChannel 55 54 53 52`), as neo's waveforms
            spike_file = next(p for p in folder.iterdir() if p.stem == entity or p.stem.startswith(entity) and p.suffix.lower() in (".ntt", ".nse", ".nst"))
            header = spike_file.read_bytes()[:16384].decode("latin-1")
            order = next(line.split()[1:] for line in header.splitlines() if line.startswith("-ADChannel"))
            for wi, key in enumerate(f"{entity}_ch{c}" for c in order):
                s = nwb.acquisition[key]
                times = np.asarray(s.timestamps[:])
                data = np.asarray(s.data[:])[:, 0, :].astype("f8") * s.conversion
                # Nearest spike here (the float times differ in the last bits)
                right = np.clip(np.searchsorted(times, t), 0, len(times) - 1)
                left = np.clip(right - 1, 0, len(times) - 1)
                idx = np.where(np.abs(times[left] - t) <= np.abs(times[right] - t), left, right)
                ok = np.isclose(times[idx], t, atol=1e-9)
                if not ok.all():
                    problems.append(f"spikes {name} wire {wi}: {(~ok).sum()} spike times not found")
                    continue
                if not np.allclose(data[idx], w[:, wi, :], rtol=1e-6, atol=1e-12):
                    problems.append(f"spikes {name} wire {wi}: waveforms differ")
            total += len(t)
        print(f"{folder.name}: {total} spikes compared")

        # Events per (event id, TTL)
        ev = getattr(nwb, "events", None)
        count = 0
        for e in range(neo.event_channels_count()):
            name = neo.header["event_channels"][e]["name"]  # "<entity> event_id=<id> ttl=<ttl>"
            entity, rest = name.split(" event_id=")
            # neo names an event file without `-AcqEntName` "unknown"; here it is "Events"
            entity = "Events" if entity == "unknown" else entity
            eid, ttl = rest.split(" ttl=")
            key = f"{entity} id{eid} ttl{ttl}"
            t, labels = [], []
            for seg in range(neo.segment_count(0)):
                ts, _, lab = neo.get_event_timestamps(0, seg, e, None, None)
                t.append(neo.rescale_event_timestamp(ts, "float64", e))
                labels += [str(x).strip() for x in lab]
            t = np.concatenate(t)
            if ev is None or key not in ev:
                problems.append(f"events {key}: missing here")
                continue
            mine = np.sort(np.asarray(ev[key]["timestamp"][:]))
            # Every neo event must be here; neo drops events that fall between its segments (a
            # known difference), so extra events here are fine only outside neo's segments
            found = np.isclose(mine[nearest(mine, np.sort(t))], np.sort(t), atol=1e-9) if len(mine) else np.zeros(len(t), bool)
            spans = [(neo.segment_t_start(0, s), neo.segment_t_stop(0, s)) for s in range(neo.segment_count(0))]
            extra = [x for x in mine if not np.any(np.isclose(t, x, atol=1e-9))]
            inside = [x for x in extra if any(a <= x <= b for a, b in spans)]
            if not found.all() or inside:
                problems.append(f"events {key}: {len(mine)} here, {len(t)} in neo; {int((~found).sum())} of neo's missing, {len(inside)} extra inside neo's segments")
            elif extra:
                print(f"  events {key}: {len(extra)} more here than in neo, all between neo's segments (neo drops them)")
            count += len(t)
        print(f"{folder.name}: {count} events compared")
    return problems
