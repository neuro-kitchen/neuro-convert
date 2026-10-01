# neuro-convert app: user guide

The desktop app does what `neuro-convert convert` does, without a terminal. Start it with
`cargo run -p nc-app` (or `neuro-convert-app [recording]`).

## The four steps
A bar under the toolbar shows **① Source → ② Contents → ③ Metadata → ④ Review & convert**. Each
step shows its open issues (red: errors that block converting, amber: warnings). Click a step or
use **Back / Next** at the bottom; the **issues** button there lists every issue and takes you to
where it is fixed.

The toolbar has only Open recording, Load / Save metadata and the theme switch.

### ① Source
**Open recording…** (Ctrl+O) picks the recording's folder; Ctrl+Shift+O picks a single file. You
can also drop a folder or file on the window, or start the app with a path. The page lists what
each format expects (from the readers), recent recordings, and, for a TDT tank or a folder with
several SpikeGLX runs, the recordings to choose from.

### ② Contents
Left: everything in the recording, with a checkbox per item (unticked items are left out; the
choice is saved with the metadata). A ⚠ marks items with an issue; streams are tagged *neural* or
*other*. Select an item to set it up on the right. For a stream:
- **What is this signal?** *Neural recording* (stored as `ElectricalSeries`, needs electrodes),
  *Other signal* (EMG, temperature, stimulus…, stored as `TimeSeries`), or *Automatic* (neural when
  the recording supplies electrodes, e.g. a Neuropixels probe).
- **Electrodes** (neural only): choose an electrode group or **New group…**, then its location
  (suggestions: common brain areas and what you typed before), description and device. Every
  channel becomes one electrode of the group. Groups shared by several streams say so.
  Picking probe designs and headstage wiring is planned separately.
- **More**: name in the NWB file, unit, and scale factor (needed when the recording does not give
  a physical scale).
- **Preview** below (see Preview).

### ③ Metadata
Session: description, start time (date-time picker, preset to the recorded time; "Use the
recorded time" undoes a change) and **time zone** (searchable list of UTC offsets with places;
recording systems store local time). Subject: id, species (suggestions with common names), sex,
age (number + days / weeks / months / years). **More details**: experiment, experimenters, lab,
institution, keywords, strain, identifier. Fields with a problem are outlined and say why.
**I'll upload to DANDI** (remembered) makes DANDI's recommendations (species, age, sex) count as
issues; when off they are not counted.

Values you type for lab, institution, experimenters, species, strain, locations and devices are
remembered and offered next time. **Save metadata** (Ctrl+S) writes a YAML usable with
`neuro-convert convert -m`; **Load metadata** (Ctrl+L) reads one and keeps it for the next
recording. Comments in a loaded YAML are not kept when saving.

### ④ Review & convert
What will be written (summary), the remaining issues with links to fix them, the output folder
(`.nwb.zarr`, default next to the recording) and **Advanced** (compression, chunks, threads). The
only **Convert** button (Ctrl+Enter) is here; when disabled it says what to fix. Progress shows
samples copied, speed and time left; **Cancel** (Esc) stops and removes the partial output. Every
conversion is verified and leaves `<output>.report.json`. **Show NWB structure** lists every path
written. **Copy diagnostics** puts versions, warnings, issues and the outcome on the clipboard.

## Preview
Dropdowns choose the **stream**, the **channels** (pages of 16, or one electrode group when a
stream spans several) and **markers** (an event series drawn as vertical lines). **Earlier /
Later**, zoom − / +, and **Gain**. Mouse wheel: next / previous channels; Ctrl+wheel: zoom;
Shift+wheel: pan. The bar under the time axis is the whole recording: click to jump. All channels
share one scale, shown by the scale bar at the right (its value is under the plot); **Fit each
channel** scales each to its own peak instead. Each lane draws, per pixel column, the smallest and
largest sample, so short spikes stay visible at any zoom. For probes, the map on the right shows
the sites, the visible ones highlighted.

## Settings
`~/.config/neuro-convert/settings.json` (or `$XDG_CONFIG_HOME`): theme, compression, chunking,
reserved threads, recent files, the DANDI switch and remembered values. `NC_APP_LOG=1` prints
status messages to stderr.

## Limits
- One recording at a time (no batch queue yet).
- Probe designs, headstage wiring and per-channel electrode maps: planned
  (`.tasks/10-01-2026/02-nc-app/12-probe-library.md`).
- Data-integrity verification (re-reading and comparing samples) is planned:
  `.tasks/10-01-2026/01-data-integrity-verification.md`.
