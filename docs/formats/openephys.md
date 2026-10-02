# Open Ephys binary and legacy formats (`nc-openephys`)

Recordings saved by the Open Ephys GUI in its **binary** format (GUI 0.4.4 – 0.6+) and in the
**legacy** format (one `.continuous` file per channel; see "Legacy format" below). The GUI's own
NWB format is not read (it is already NWB).

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

## Legacy format (`legacy.rs`)
Files (each with a 1024-byte text header `header.key = value;`):

| File | Content | Becomes |
|---|---|---|
| `<proc>_[<source>_]CH<n>[_<start>].continuous` | records: int64 first sample number, uint16 count (1024), uint16 recording number, 1024 big-endian int16, 10-byte marker | `<Processor>-<proc>` (electrical, `bitVolts` µV) |
| `…AUX<n>…` / `…ADC<n>…` `.continuous` | same | `<Processor>-<proc>.analog` (`bitVolts` V) |
| `all_channels[_<start>].events` | 16-byte records; type 3 = TTL, id 1 / 0 = rising / falling, channel 0-based | `<Processor>-<proc> TTL <channel + 1>` (high periods) |
| `messages[_<start>].events` | `<sample number> <text>` lines | `messages` (labelled, sorted by time) |
| `<electrode>[_<start>].spikes` | spike records (n channels × m samples uint16, offset 32768; gains per channel) | snippet store `<electrode>`, one snippet per channel of each spike (channels 1…n), sort code = sorted id, volts = (raw − 32768) / gain / 1000 |

- Every acquisition start (no suffix, `_2`, `_3`, …; `settings_<n>.xml`) is a container
  `experiment<n>` (neo's segments); several record-node folders prefix it
  (`Record Node 120/experiment1`). The processor name comes from `settings.xml`
  (`<PROCESSOR name="Sources/Rhythm FPGA" NodeId="100">` → `Rhythm_FPGA-100`) or the file name.
- Records may have gaps (a paused recording): missing samples read as 0, with a warning (as
  neo). Channel files covering different samples are clipped to the common range (as neo; e.g.
  `OpenEphys_SampleData_3` `CH32`, which neo refuses to open).
- Times: sample number / rate, relative to the start's first sample. Spikes need an electrode
  group in the metadata (`snippets: { '*': { electrode_group: … } }`) to be written.
- Verified with `tools/python/compare_openephys_legacy.py` against neo's `OpenEphysRawIO`
  through NWB, 2026-10-02: `OpenEphys_SampleData_1` (2 ch with gaps, 454 stereotrode spikes),
  `OpenEphys_SampleData_2_(multiple_starts)` (2 starts, 265 + 74 spikes), `OpenEphys_SampleData_3`
  (2 starts, 5 TTL lines; compared without CH32): every sample, TTL onset and spike waveform /
  time equal. neo's spike API returns nothing for legacy files (it compares the integer sorted id
  with a string), so spikes are compared with its memory map and scaling.

Not yet: binary-format spikes, OneBox ADC streams, joining recordings.
