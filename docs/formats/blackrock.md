# Blackrock Microsystems (`nc-blackrock`)

Cerebus / NeuroPort / Gemini recordings: `.ns1`–`.ns6` (continuous, one sample rate per file) and
`.nev` (spikes, digital / serial input, comments) sharing a base name. File specs 2.1, 2.2, 2.3,
3.0 and 3.0 with PTP timestamps. References: Blackrock's NEV / NSx file specifications, neo's
`BlackrockRawIO` (behaviour matched and compared below).

## What to open
A folder (every base name in it is a recording; several → `--block <base>`) or any of the files
(the files with its base name open together). `.ccf` / `.sif` / `.toc` / `.csr` are ignored.

## Files
| File | Layout | Module |
|---|---|---|
| `.nsX` 2.1 (`NEURALSG`) | 32-byte header, uint32 electrode ids, int16 samples to the end; no timestamps, no scaling | `nsx.rs` |
| `.nsX` 2.2 / 2.3 (`NEURALCD`), 3.0 (`BRSMPGRP`) | 314-byte header (period in 1/30 000 s, timestamp resolution, UTC origin), 66-byte `CC` header per channel (id, label, bank / pin, digital / analog ranges, units, filters), data blocks `{flag, timestamp (u32; 3.0 u64), count, samples}` | `nsx.rs` |
| `.nsX` 3.0 PTP (resolution 1 ns) | one block per sample with its own timestamp | `nsx.rs` |
| `.nev` (`NEURALEV` / `BREVENTS`) | 336-byte header, 32-byte extended headers (`NEUEVWAV`, `NEUEVLBL`, …), fixed-size packets `{timestamp, id, payload}` | `nev.rs` |

## Mapping
| Source | Becomes |
|---|---|
| NSx channels in µV (front end; 2.1: id < 129) | `ns<N>` electrical, int16 with gain = analog range / digital range (2.1: the NEV's digitization factor, 21516 → 152 592.547 nV as neo) |
| other NSx channels (analog inputs, mV) | `ns<N>.analog` |
| several data blocks (pauses) / PTP gaps > 2 periods | parts `ns<N>.p1`, `.p2`, … each at its own start time (blocks with < 2 samples skipped, as neo) |
| NEV spikes (ids 1–2048) | snippet store `spikes`: channel = electrode id, sort code = unit class (0 unsorted, 1–16 sorted, 255 noise), volts = raw × digitization factor (nV) (`spikes_<width>` per length if they differ) |
| digital input (id 0, reason bit 0; 2.1 / 2.2 reason 1) | events `digital_input`, value = 16-bit word |
| serial input (reason bit 7; 2.1 / 2.2 reason 129) | events `serial_input` |
| comments (id 0xFFFF, ANSI / UTF-16) | events `comments` (labels) |
| other packets (video sync, tracking, buttons, configuration, system ids ≥ 0x8000) | not read (warning lists ids and counts) |

Electrodes: one per electrode id, shared by every NSx file and the spikes, grouped by front-end
bank (`bank A`, …; connector 1 = A) on device `Blackrock`.

## Time
Seconds from the earliest timestamp of the container (NSx parts and NEV packets; integer ticks
subtracted first, so same-clock times are exact). A timestamp going back by more than a second,
or a "critical load restart" comment, is a clock reset: each clock epoch is a container
`segment<k>` (neo's segments). Smaller steps back (PTP jitter between hubs) stay in the epoch.
`start_time`: the NSx (else NEV) time origin, UTC.

## Verification
`tools/python/compare blackrock` converts each container, reads it back with pynwb and compares
with neo per NSx file: every sample of every channel, part start times, every spike time and
waveform per electrode and unit, digital input words. 2026-10-02, neo's GIN test files
(`tools/python/fetch_gin.py blackrock/<set>`):

| Set | Spec | Compared | Result |
|---|---|---|---|
| `FileSpec2.3001` | 2.3 | 9.0 M values, 964 spikes, 190 digital words | equal |
| `blackrock_2_1/l101210-001` | 2.1 | ns2 + ns5 (10.5 M values), 14 210 spikes, 8 words | equal (neo's 2.1 reader drops the last sample) |
| `blackrock_3_0/file_spec_3_0` | 3.0 | 2.4 M values, 864 spikes | equal |
| `blackrock_3_0_ptp/20231027-125608-001` | 3.0 PTP | ns2 + ns6 (4.3 M values), 12 832 spikes | equal |
| `segment/PauseCorrect` | 2.3, pause | 2 parts, 11 183 spikes | equal |
| `segment/ResetCorrect` | 2.3, clock reset | 2 segments, 13 079 spikes | equal |
| `blackrock_ptp_with_missing_samples` | 3.0 PTP, gaps | 5 + 3 parts, every value | equal; 999 spikes here, 996 in neo (neo treats the NEV's small PTP steps back as resets and loses 3) |

Unit test: a synthetic 2.3 pair with a pause, a clock reset, spikes, a digital word and comments.
