# Command line

```text
neuro-convert formats
neuro-convert inspect  <recording> [--json] [open options]
neuro-convert convert  <recording> -m <meta.yaml> -o <output> [open options] [convert options]
neuro-convert validate <output>
neuro-convert verify   <output> [--report <report.json>]
```

## A conversion, start to end

```sh
neuro-convert inspect data/rec                                   # 1. what is in the recording
cp metadata/session.example.yaml meta.yaml                       # 2. write the metadata
neuro-convert convert data/rec -m meta.yaml -o rec.nwb.zarr --dry-run   # 3. check the plan
neuro-convert convert data/rec -m meta.yaml -o rec.nwb.zarr      # 4. convert and check
neuro-convert verify rec.nwb.zarr                                # 5. later, re-check a copy
```

`-o name.nwb.zarr` writes a Zarr store; `-o name.nwb` writes one HDF5 file (builds with HDF5).

## Commands

| Command | Does |
|---|---|
| `formats` | Lists the readers in this build with their versions, file-format versions and maturity, and the outputs. |
| `inspect` | Shows streams, events, spike stores, tables, electrodes, metadata and the reader's warnings. `--json` for scripts; `--read-sec <s>` times reading `s` seconds of the largest stream. |
| `convert` | Opens, plans, writes and checks. Prints the plan; stops before writing if the plan has errors. |
| `validate` | Checks the NWB structure of an output: required fields, electrodes table, references, series shapes. |
| `verify` | `validate` plus the content: re-reads the blocks recorded in `<output>.report.json` and compares their digests. No source needed (use it after copying or uploading). |

## Open options (`inspect`, `convert`)

| Option | Use |
|---|---|
| `--block <name>` | Open one recording of a folder that holds several: TDT tank block, SpikeGLX gate, Open Ephys recording, Blackrock segment, Neuralynx session. Without it, a folder with several fails and lists them. |
| `--only A,B` | Read only these streams or stores. |
| `--sort <id>` | TDT: apply an offline sort (`sort/<id>/`). |

## Convert options

| Option | Default | Use |
|---|---|---|
| `-m, --metadata <file>` | — | [Metadata file](metadata.md). |
| `-o, --output <path>` | — | `.nwb.zarr` (Zarr) or `.nwb` (HDF5). |
| `--dry-run` | off | Print the plan and its issues; write nothing. |
| `--overwrite` | off | Replace an existing output. |
| `--gzip <level>` | 1 | Compression level per chunk. |
| `--no-compression` | off | Store chunks uncompressed. |
| `--chunk <s\|auto>` | 1 | Chunk length in seconds, or `auto` (about 10 MB per chunk). |
| `--threads <n>` | all CPUs | Writer threads. |
| `--verify full\|sampled\|off` | `full` | Content check after writing: every block, first + last + 8 random blocks per array, or structure only. |
| `--skip-source-check` | off | Convert even when a source file fails its own recorded checksum (SpikeGLX `fileSHA1`). |

Ctrl-C stops a conversion and removes the partial output.

## Output files

| File | Holds |
|---|---|
| `<output>` | The NWB store or file. |
| `<output>.report.json` | Summary, plan, provenance (files read, warnings), versions, source checks and content digests. |

How the check works and what the report records: [NWB output, content verification](../outputs/nwb-mapping.md#content-verification-nc_nwbintegrity-jobwrite).
