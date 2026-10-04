# neuro-convert

> **Under development.** neuro-convert is available for testing and not ready for production use.
> Expect changes to the command line, the metadata file, the output and the API between versions.

neuro-convert converts neurophysiology recordings to NWB 2.11, as a Zarr store (`.nwb.zarr`) or an
HDF5 file (`.nwb`). It has a command line (`neuro-convert`) and a desktop app
(`neuro-convert-app`). Both run the same conversion.

![neuro-convert app: the Contents step with a Neuropixels recording](images/preview.png)

*The desktop app on an Open Ephys Neuropixels recording: the recording's contents (left), a
preview of the selected stream (middle) and its NWB settings (right).*

## What a conversion does

```text
 recording ──reader──▶ Session ──+ metadata file──▶ NWB plan ──▶ .nwb.zarr / .nwb ──▶ check against source
```

1. A **reader** opens the recording (TDT, SpikeGLX, Intan, Open Ephys, Blackrock, Neuralynx) and
   describes it as a `Session`: streams, events, spikes, electrodes, metadata.
2. A **metadata file** (YAML) adds what the recording does not store: session description,
   subject, time zone, electrode locations, and how each stream is exported.
3. The **plan** lists every NWB object to write, with blocking errors and DANDI warnings.
4. The **writer** copies the data in blocks, keeping the stored integer type, and writes the gains
   as NWB `conversion` / `channel_conversion`.
5. The **check** reads the written data back and compares it with the source (xxh3-64 per block).
   The digests go to `<output>.report.json`.

## Properties

| Property | How |
|---|---|
| Large recordings | Files are memory-mapped and read in blocks; memory use does not grow with recording length. |
| Exact values | Integer samples are stored as integers; physical units come from `conversion`. |
| Checked output | Every conversion is checked against the source; `neuro-convert verify` re-checks a copy from its report. |
| Compared readers | Each reader is compared value for value with a reference reader (neo, TDT's `tdt`, SpikeGLX's rules) on public test data. |
| Traceable files | Program, reader and crate versions are written to the report and to `/general/source_script`. |

## Where to go next

| To | Read |
|---|---|
| Build the tools | [Build and install](guide/install.md) |
| Convert a recording | [Command line](guide/cli.md), [Metadata file](guide/metadata.md), [Desktop app](app.md) |
| Know what a format gives | [Formats](formats/tdt.md), [NWB output](outputs/nwb-mapping.md) |
| Find your way in the code | [Repository layout](dev/repository.md), [Architecture](dev/architecture.md) |
| Add or change a reader | [Writing a reader](readers/README.md), [Changing a reader](dev/changing-a-reader.md) |
| Change the output | [Output](dev/output.md) |
| Look up a type or function | [API reference](dev/api.md) |
