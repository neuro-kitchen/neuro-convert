"""Open Ephys legacy format (.continuous / .events / .spikes): per acquisition start (neo's
segment) every sample of every channel (volts), the TTL onsets, and every spike waveform and time,
against neo's OpenEphysRawIO. neo's spike API returns nothing for legacy files (it compares the
integer sorted_id with the string from the unit name), so its memory map of each `.spikes` file
and its scaling are used. Path: a folder of `.continuous` files."""

from pathlib import Path

import numpy as np
from neo.rawio import OpenEphysRawIO

from .common import TO_SI, containers, convert, open_nwb

REQUIRES = ["neo"]
# Spike stores are written only with an electrode group
SPIKE_GROUP = "electrode_groups:\n  - { name: SpikeElectrode, description: spike electrode, location: unknown }\nsnippets:\n  '*': { electrode_group: SpikeElectrode }\n"


def compare(folder: Path, opts) -> list[str]:
    problems = []
    neo = OpenEphysRawIO(dirname=str(folder))
    neo.parse_header()
    chans = neo.header["signal_channels"]
    for block in [opts.block] if opts.block else (containers(folder) or ["experiment1"]):
        seg = int(block.rsplit("experiment", 1)[1]) - 1
        dest = convert(folder, block, extra_yaml=SPIKE_GROUP)
        t_start = neo.get_signal_t_start(0, seg, 0)
        with open_nwb(dest) as nwb:
            series = {n: s for n, s in nwb.acquisition.items() if hasattr(s, "rate") and s.rate}
            # Samples: every channel, matched by name
            raw = neo.get_analogsignal_chunk(0, seg, stream_index=0)
            ref = raw.astype("f8") * chans["gain"][None, :] * np.array([TO_SI[u] for u in chans["units"]])[None, :]
            checked = 0
            for s in series.values():
                data = np.asarray(s.data[:]).astype("f8") * s.conversion
                if s.channel_conversion is not None:
                    data = data * np.asarray(s.channel_conversion)[None, :]
                for k in range(data.shape[1]):
                    # Channels in neo's order (CH by number, then AUX / ADC), as written
                    want = ref[: data.shape[0], checked]
                    if data.shape[0] != ref.shape[0]:
                        problems.append(f"{block} {s.name}: {data.shape[0]} samples vs neo {ref.shape[0]}")
                    if not np.allclose(data[:, k], want, rtol=1e-6, atol=1e-9):
                        bad = np.flatnonzero(~np.isclose(data[:, k], want, rtol=1e-6, atol=1e-9))
                        problems.append(f"{block} {s.name} ch{k}: {len(bad)} samples differ (first at {bad[0]}: {data[bad[0], k]} vs {want[bad[0]]})")
                    checked += 1
            print(f"{folder.name} {block}: {checked} channels × {ref.shape[0]} samples compared")

            # TTL onsets (neo: all event records with labels type#processor#channel; type 3 = TTL)
            if neo.event_channels_count() > 0:
                ts, _, labels = neo.get_event_timestamps(0, seg, 0)
                times = neo.rescale_event_timestamp(ts, event_channel_index=0) - t_start
                ttl = [(t, int(l.split("#")[2])) for t, l in zip(times, labels) if l.startswith("3#")]
                events = getattr(nwb, "events", None) or {}
                ours = {}
                for name, table in (events.items() if hasattr(events, "items") else []):
                    if " TTL " in name:
                        ours[int(name.rsplit(" ", 1)[1]) - 1] = np.asarray(table["timestamp"][:])
                for line in sorted({c for _, c in ttl}):
                    # neo lists every change; our onsets are the rising ones, a subset of those times
                    neo_times = np.array([t for t, c in ttl if c == line])
                    mine = ours.get(line, np.array([]))
                    missing = [t for t in mine if not np.any(np.isclose(neo_times, t, atol=1e-9))]
                    if missing:
                        problems.append(f"{block} TTL line {line + 1}: onsets {missing[:3]} not among neo's event times")
                print(f"{folder.name} {block}: {len(ttl)} TTL changes, lines {sorted(ours)} checked")

            # Spikes: every waveform and time, per spike file and channel. neo's spike API returns
            # nothing for legacy files (it compares the integer sorted_id with the string from the
            # unit name), so its memory map of each .spikes file and its scaling are used:
            # µV = raw × 1000 / gain + offset (−32768 × gain), gain from the first spike.
            for name, data_spike in getattr(neo, "_spikes_memmap", {}).get(seg, {}).items():
                name = name.rsplit("_", 1)[0] if name.rsplit("_", 1)[-1].isdigit() else name
                if len(data_spike) == 0:
                    continue
                gain = 1000.0 / data_spike[0]["gains"][0]
                nb_chan = int(data_spike[0]["nb_channel"])
                wf = (data_spike["samples"].reshape(len(data_spike), nb_chan, -1).astype("f8") * gain - 32768 * gain) * 1e-6
                t = data_spike["timestamp"] / neo._spike_sampling_rate - t_start
                for c in range(nb_chan):
                    key = f"{name}_ch{c + 1}"
                    if key not in nwb.acquisition:
                        problems.append(f"{block} {key}: missing")
                        continue
                    s = nwb.acquisition[key]
                    data = np.asarray(s.data[:])[:, 0, :].astype("f8") * s.conversion
                    times = np.asarray(s.timestamps[:])
                    if data.shape != wf[:, c, :].shape:
                        problems.append(f"{block} {key}: {data.shape} vs neo {wf[:, c, :].shape}")
                        continue
                    if not np.allclose(times, t, atol=1e-9):
                        problems.append(f"{block} {key}: spike times differ")
                    if not np.allclose(data, wf[:, c, :], rtol=1e-5, atol=1e-9):
                        problems.append(f"{block} {key}: waveforms differ (max {np.max(np.abs(data - wf[:, c, :]))})")
                print(f"{folder.name} {block}: spikes {name}: {len(data_spike)} × {nb_chan} channels compared")
    return problems
