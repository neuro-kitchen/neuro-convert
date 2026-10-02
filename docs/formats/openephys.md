# Open Ephys binary format (`nc-openephys`)

Recordings saved by the Open Ephys GUI in its **binary** format (GUI 0.4.4 – 0.6+). The legacy
`.continuous` format and the GUI's own NWB format are not read (the latter is already NWB).

## What to open
The save folder, a record node, an experiment or one recording folder (or its
`structure.oebin`). Every `…/recordingK/` with a `structure.oebin` below it is a container:
`Record Node 104/experiment1/recording1` (GUI ≥ 0.5), `experiment1/recording1` (older). With
several, choose one (`--block`, or the app's list).

```
Record Node 104/
  settings.xml                         signal chain: date, probes (site positions)
  experiment1/recording1/
    structure.oebin                    JSON: streams, channels (bit_volts, units), events
    sync_messages.txt                  ≥ 0.6: wall clock (ms since 1970 UTC) + start sample per stream
    continuous/<source>.<stream>/      continuous.dat (int16, interleaved per sample)
                                       sample_numbers.npy + timestamps.npy (≥ 0.6: synchronized s)
                                       timestamps.npy (< 0.6: sample numbers)
    events/<source>.<stream>/TTL/      states.npy (± line, 1-based; < 0.6 channel_states.npy),
                                       sample_numbers.npy, timestamps.npy, full_words.npy
    events/MessageCenter/              text.npy, sample_numbers.npy, timestamps.npy
```

## Streams
Each continuous stream (`stream_name`, or the folder name before 0.6) is split by channel:
- **electrode channels** → `<stream>` (electrical, volts: `bit_volts` is µV unless `units` says
  otherwise);
- **other analog channels** (names with ADC / AUX, NI-DAQ "Analog Input" channels) →
  `<stream>.analog` (volts: their `bit_volts` are volts — Open Ephys's documentation; neo labels
  them µV);
- **sync line** (`*_SYNC`, Neuropixels) → `<stream>.sync` (raw).

Samples stay int16 with the gain as NWB `conversion` / `channel_conversion`.

Timing: GUI ≥ 0.6 writes synchronized seconds per sample (`timestamps.npy`) that put streams from
different devices on one clock; the first value of each stream gives its start (relative to the
earliest stream). Before 0.6, sample number / rate. (neo uses sample numbers for every version, so
streams of different devices — e.g. an NI-DAQ next to a Neuropixels probe — differ from neo by
their clock offset; on the test recording 125 ms.) Sample numbers with gaps are reported.

## Electrodes
One electrode per electrode channel, one group (and device) per probe or stream. Neuropixels AP and
LFP streams of a probe (`ProbeA-AP`, `ProbeA-LFP`) share electrodes; `ProbeA` is the first
`NP_PROBE` of `settings.xml`, `ProbeB` the second, …, whose `ELECTRODE_XPOS` / `ELECTRODE_YPOS`
give the site positions (µm) and `probe_name` / serial the device.

## Events
- TTL: one event series per line (`<stream> TTL <line>`), high periods from the ± states (a line
  still high at the end closes at the end of the recording). Times on the same axis as the
  streams (≥ 0.6 synchronized timestamps; before, the stream's sample numbers).
- Text messages: one series `messages` with the text as labels.
- Spikes: not read yet (no test data).

## Metadata
- Start time: the wall clock in `sync_messages.txt` (UTC, ≥ 0.6), else `settings.xml`'s `<DATE>`
  (local time: set the time zone), else none.
- Experiment: `Record Node … / experimentN / recordingK`; GUI version and first sample numbers in
  `extra`.

## Verification
`tools/python/compare_openephys.py` converts each recording to NWB, reads it back with pynwb and
compares every value of every series with neo's `OpenEphysBinaryRawIO` (streams matched by
values; analog and sync channels differ only by neo's µV label), the start times (≥ 0.6 against
the synchronized `timestamps.npy`, before against neo) and the number of TTL periods. Checked
2026-10-01 on the GIN test recordings (`ephy_testing_data/openephysbinary`):
`v0.6.x_neuropixels_with_sync` (384-ch AP + LFP, sync lines, NI-DAQ, 83 561 TTL periods, 1124
messages), `v0.5.x_two_nodes` (two record nodes × three recordings), `neural_and_non_neural_data_mixed`
(0.4.5, headstage + ADC): all equal. Test data → `data/raw/openephys/`.

Not yet: legacy `.continuous` format, spikes, OneBox ADC streams, joining recordings.
