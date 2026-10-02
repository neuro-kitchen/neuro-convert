# Neuralynx (`nc-neuralynx`)

Session folders recorded by Cheetah (1.x – 6.x), Pegasus (2.x), BML or Neuraview: one file per
entity, each with a 16 KiB text header (`## …` lines, `-Key value` properties). References: the
Neuralynx file format documentation, neo's `NeuralynxRawIO` (behaviour matched and compared below).

## What to open
The session folder, or any file in it (the whole folder opens). A folder holding several session
folders lists them as containers (`--block <name>`). `.nvt` (video tracking) and `.nrd` (raw data)
are not read (warning).

## Files
| File | Records | Becomes |
|---|---|---|
| `.ncs` | 1044 bytes: uint64 µs, uint32 channel, uint32 rate, uint32 valid, 512 int16 | continuous channels |
| `.nse` / `.nst` / `.ntt` | 48-byte header (uint64 µs, uint32 channel, uint32 cell, 8 int32 features) + int16 samples, `[sample][wire]` | spikes (1 / 2 / 4 wires) |
| `.nev` | 184 bytes: µs, event id, TTL value, 128-byte string | events |

## Mapping
- **Streams.** `.ncs` files with the same stated rate, input range and DSP filters form a stream
  (neo), named `ncs_<rate>Hz` (`_2`, … when two share a rate), ordered by rate. Channels are the
  files' `AcqEntName`s, int16 with gain = `ADBitVolts` (V per bit; negative when `InputInverted`,
  as neo). Electrical; one electrode per AD channel (`ADChannel`) in group `Neuralynx`.
- **Gaps.** Records are grouped into gap-free sections as neo does: a record starting more than
  the tolerance away from the previous record's predicted end begins a new one (tolerance 0 µs for
  Cheetah < 4 and BML / Atlas, 0.2 sample periods for Digital Lynx and later). Each section is a
  part `ncs_<rate>Hz.p<k>` at its own start. A file whose records do not line up with the others
  of its stream is skipped (warning; neo refuses the folder).
- **Sample rate.** Cheetah < 4: whole microseconds per sample (`1e6 / floor(1e6 / stated)`); BML /
  Atlas: the stated rate; later systems: measured from the longest section's timestamps (neo's
  `sampFreqUsed`; the hardware clock drifts from the rounded stated rate). The stated rate is in
  the series' metadata and description.
- **Spikes.** One snippet store per spike file (its entity name, e.g. `TT1`), one snippet per wire
  of each spike: channel = the wire's AD channel (shared electrode with a `.ncs` of the same AD
  channel), sort code = cell number, volts = int16 × the wire's `ADBitVolts`.
- **Events.** One series per (event id, TTL value): `<entity> id<event id> ttl<TTL>`, value = TTL,
  label = the event string.
- **Time.** Seconds from the earliest timestamp in the folder (signals, spikes, events). Start
  time: the earliest `Time Opened` / `TimeCreated` (local time of the acquisition computer).

## Verification
`tools/python/compare_neuralynx.py` converts each folder, reads it back with pynwb and compares
with neo: every sample of every channel per segment / part and its start time, every spike time
and waveform per wire and unit, every event time per (id, TTL). 2026-10-02, neo's GIN test files
(`tools/python/fetch_gin.py neuralynx/<set>`): Cheetah 1.1.0, 4.0.2 (whole-µs rate), 5.4.0,
5.5.1 (gap, stereotrodes), 5.6.3 (gap, 2 tetrodes with wires listed 55 54 53 52, 46 060 spikes),
5.7.4 (4 parts), 6.3.2 (incomplete blocks, 3 parts), 6.4.1dev (3 streams), BML, Pegasus 2.1.1,
Neuraview 2, NoDateHeader, over_segmentation_example (9 parts): all values, spikes and times
equal. Known differences: neo drops events that fall between its segments (Cheetah 5.6.3: 1,
5.7.4: 3) — every event is kept here; neo reports the stated rate where the measured one is stored
here; `two_streams_with_small_time_differences` is refused by neo (its two streams have different
gaps) and compared one file at a time (equal).
