# Metadata file

A recording does not store everything NWB needs (session description, subject species and age,
time zone, electrode locations), nor how each stream should be exported. The metadata file
supplies both. The CLI reads it with `-m`; the app loads and saves the same format.

Template: [`metadata/session.example.yaml`](https://github.com/neuro-kitchen/neuro-convert/blob/main/metadata/session.example.yaml).
Real examples: [`metadata/examples/`](https://github.com/neuro-kitchen/neuro-convert/tree/main/metadata/examples).

## Example

A TDT block with a 32-channel EMG grid and an impedance export:

```yaml
session:
  description: "Motor evoked potentials recorded with a diaphragm HD-EMG grid."
  timezone: "-05:00"
  lab: "Example Lab"
  institution: "Example University"

subject:
  species: "Rattus norvegicus"
  sex: U
  age: P90D

electrode_groups:
  - name: HDEMG
    description: "32-channel high-density EMG grid"
    location: diaphragm
    impedance: { table: Z_HDEMG }

streams:
  "*": { type: timeseries, unit: a.u. }
  HDEG: { type: electrical, electrode_group: HDEMG, name: HDEMG }
  SU_1: { include: false }
```

## Sections

Keys under `streams`, `events`, `tables` and `snippets` are the recording's own names, as
`neuro-convert inspect` prints them. `"*"` sets the default for every item not listed.

### `session`

| Key | Required | Value |
|---|---|---|
| `description` | yes | One or two sentences. |
| `timezone` | when the recording stores local time | UTC offset of the recorded start time, e.g. `"-05:00"`. |
| `start_time` | no | ISO 8601 with a zone; overrides the recorded time. |
| `identifier` | no | Default: a new UUID. |
| `experiment_description`, `experimenters`, `lab`, `institution`, `keywords` | no | |

### `subject`

| Key | Value |
|---|---|
| `id` | Default: the subject name the recording stores. |
| `species` | Latin binomial (`Mus musculus`). DANDI requires it. |
| `sex` | `M`, `F`, `U` (unknown), `O` (other). |
| `age` | ISO 8601 duration (`P90D`). DANDI requires it. |
| `strain`, `description` | |

### `electrode_groups`

Groups declared here replace groups of the same name from the reader. Readers that know their probe
(SpikeGLX, Open Ephys Neuropixels, Blackrock, Neuralynx, Intan) supply groups and electrodes
themselves.

| Key | Value |
|---|---|
| `name`, `description`, `location` | `location`: anatomical location. |
| `device` | Default: the recording's first device. |
| `impedance` | `{ table: <name> }`: a table of the recording with columns `R1`, `R2`, … in kOhm (Ohm, MOhm also read). `prefix` changes `R`; `row` picks a row (default: the last measurement per channel). |

### `streams`

| Key | Value |
|---|---|
| `type` | `electrical` (NWB `ElectricalSeries`) or `timeseries` (`TimeSeries`). Without it: electrical when every channel has an electrode from the reader. |
| `electrode_group` | Electrical streams without reader electrodes: one electrode per channel in this group. |
| `name`, `description` | Output name (default: the source name). |
| `unit` | `TimeSeries` unit (default `a.u.`). |
| `conversion` | Stored value → volts (electrical) or → `unit`. Needed when the reader reports the scale as unknown. |
| `include` | `false` leaves the stream out. |

### `events`, `tables`

`name`, `description`, `include`.

### `snippets`

Spike snippet stores. Each is written as one `SpikeEventSeries` per channel; sorted snippets
(non-zero sort codes) also go to `/units`.

| Key | Value |
|---|---|
| `electrode_group` | Needed when the reader gives no electrodes. Channel `c` = the `c`-th channel of the group's first stream. |
| `name` | Series are `<name>_ch<c>`. |
| `conversion` | Stored value → volts. |
| `description`, `include` | |

## Checks

`convert --dry-run` (and the app's Review step) prints the plan's issues.

| Errors (nothing is written) | Warnings |
|---|---|
| missing `session.description`; start time without a zone; undeclared `electrode_group`; electrical stream with channels lacking electrodes; snippet channels outside their group; duplicate output names | DANDI fields missing (`subject.species`, `subject.age`); unknown stream keys; streams with an unknown scale and no `conversion`; snippets without electrodes |

Unknown keys in the file are errors (typos are not ignored).
