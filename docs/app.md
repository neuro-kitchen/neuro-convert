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
Three regions. The middle one's heading says what is selected, its facts and where it goes (e.g.
*Stream HDEG · 32 ch · 24 414 Hz · 47 min 12 s · V · TimeSeries*). Each side panel has a toggle
beside its title; hiding the panel hides the title, and the toggle stays at the same edge (at the
end of the middle heading) to show it again. The choice is remembered:
- **Left panel, Recording:** everything in the recording, with a checkbox per item (unticked items
  are left out; the choice is saved with the metadata). A ⚠ marks items with an issue; streams are
  tagged *ElectricalSeries*, *TimeSeries*, or *TimeSeries?* (many channels at a high rate:
  probably electrode data, decide on the right).
- **Middle:** the data. A stream shows its preview (see Preview); events, tables, snippets and
  electrode groups show their rows in a table (click a header to sort; numbers sort as numbers).
- **Right panel, Settings** of the selected item. For a stream:
  - **What is this signal?** **Detect automatically** (on by default) shows the detected type in a
    greyed dropdown: *ElectricalSeries* when the recording supplies electrodes (e.g. a Neuropixels
    probe), else *TimeSeries*. Switch it off to choose; the dropdown is highlighted until you pick.
    One line under it says what the type holds: *ElectricalSeries* is voltage from electrodes
    (spikes, LFP, EEG, ECoG, EMG; needs electrodes), *TimeSeries* any other signal (EMG envelope,
    temperature, stimulus monitor, sync…).
  - **Electrodes** (ElectricalSeries only): choose an electrode group or **New group…**, then its location
    (a dropdown of common brain areas and what you typed before; **Other…** opens a text box),
    description and device. Every
    channel becomes one electrode of the group. Groups shared by several streams say so. Picking
    probe designs and headstage wiring is planned separately.
  - **Details**: name in the NWB file, unit, and scale factor (needed when the recording does not
    give a physical scale).

### ③ Metadata
The form is a centered column. Session: description, start time (date and time, preset to the recorded time; "Use the
recorded time" undoes a change) and **time zone** (searchable list of UTC offsets with places;
recording systems store local time). Subject: id, species (dropdown with common names; **Other…** to type one), sex,
age (number + days / weeks / months / years). **More details**: experiment, experimenters, lab,
institution, keywords, strain, identifier. Fields with a problem are outlined and say why.
**I'll upload to DANDI** (remembered) makes DANDI's recommendations (species, age, sex) count as
issues; when off they are not counted.

Values you type for lab, institution, experimenters, species, strain, locations and devices are
remembered and offered next time. **Save metadata** (Ctrl+S) writes a YAML usable with
`neuro-convert convert -m`; **Load metadata** (Ctrl+L) reads one and keeps it for the next
recording. Comments in a loaded YAML are not kept when saving.

### ④ Review & convert
What will be written (summary), the remaining issues with links to fix them, the output (default
next to the recording; in builds with HDF5, **Format** chooses a Zarr folder `.nwb.zarr` or one
HDF5 file `.nwb`) and **Advanced** (compression, chunks, check after
writing, threads). The
only **Convert** button (Ctrl+Enter) is here; when disabled it says what to fix. Progress shows
samples copied, speed and time left; **Cancel** (Esc) stops and removes the partial output. Every
conversion is verified and leaves `<output>.report.json`: the structure always, and the content
compared with the source per **Check after writing** — *Sampled* (default: first, last and random
blocks, seconds), *Full* (everything plus the source files' own checksums, about as long as
writing) or *Off*. **NWB structure** (button on the summary) opens a panel on the right with every
path written and where it comes from; click a row to open its item in Contents. The output is
the store's name (editable) in the folder shown under it (**Choose…** to change both). **Copy diagnostics** puts versions, warnings, issues and the outcome on the clipboard.

## Preview
Dropdowns choose the **stream**, the **channels** (all, or one electrode group when a stream spans
several), how many **lanes** are shown at once (4–64) and **markers** (an event series drawn as
vertical lines); zoom − / + and **Gain**. A stream opens with its first second (the whole
recording when it is shorter; then times are in ms).

Moving around, without buttons:
- **Time:** drag the traces, swipe sideways on a trackpad, or Shift+wheel; the view follows
  continuously (three windows around it are read ahead).
- **Zoom:** Ctrl+wheel (or pinch) zooms around the pointer.
- **Channels:** the wheel scrolls through the channels (three per notch); the bar right of the
  traces shows where you are and can be dragged.
- **Overview** (the bar under the time axis is the whole recording): drag the highlighted window
  to move, drag its edges to zoom; pressing outside it moves it there.
- **Keys** (after clicking the traces): ← / → (Shift: half a window), + / −, ↑ / ↓ one channel,
  Page Up / Down a screen of channels, Home / End.

All channels share one scale, shown by the scale bar at the right (its value is under the plot);
**Fit each channel** scales each to its own peak instead. Each lane draws, per pixel column, the
smallest and largest sample, so short spikes stay visible at any zoom. For probes, the map on the
right shows the sites, the visible ones highlighted.

## Settings
`~/.config/neuro-convert/settings.json` (or `$XDG_CONFIG_HOME`): theme, compression, chunking,
check after writing, reserved threads, open panels, recent files, the DANDI switch and remembered values. `NC_APP_LOG=1` prints
status messages to stderr.

## Limits
- One recording at a time (no batch queue yet).
- Probe designs, headstage wiring and per-channel electrode maps: planned
  (`.tasks/10-01-2026/02-nc-app/12-probe-library.md`).
- A source checksum mismatch (Full check) stops the conversion; converting such a file anyway is
  CLI-only for now (`--skip-source-check`).
