//! `neuro-convert`: inspect recordings and convert them to publication formats.

mod commands;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "neuro-convert", version, about = "Read neurophysiology recordings and convert them to publication formats")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the input formats and versions this build reads
    Formats,
    /// Show what a recording contains: streams, events, metadata, warnings
    Inspect {
        /// Recording path (e.g. a TDT block folder)
        path: PathBuf,
        /// Also time reading this many seconds of every channel of the largest stream
        #[arg(long)]
        read_sec: Option<f64>,
        /// Print a machine-readable summary (JSON) instead
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        open: commands::OpenArgs,
    },
    /// Convert a recording to NWB (Zarr), using a metadata file for what the source lacks
    Convert(commands::convert::ConvertArgs),
    /// Check an NWB-Zarr store's structure (references, lengths, required fields)
    Validate {
        /// Store to check (`.nwb.zarr`, or `.nwb` in HDF5 builds)
        path: PathBuf,
    },
    /// Check a store's structure and re-check its content against the digests in its conversion
    /// report (no source needed; e.g. after copying or uploading the store)
    Verify {
        /// Store to check (`.nwb.zarr`, or `.nwb` in HDF5 builds)
        path: PathBuf,
        /// Conversion report (default: `<store>.report.json` next to it)
        #[arg(long)]
        report: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Formats => commands::formats::run(),
        Command::Inspect { path, read_sec, json: true, open } => commands::inspect::json(&path, read_sec, &open.options()),
        Command::Inspect { path, read_sec, json: false, open } => commands::inspect::run(&path, read_sec, &open.options()),
        Command::Convert(args) => commands::convert::run(&args),
        Command::Validate { path } => commands::convert::validate(&path),
        Command::Verify { path, report } => {
            let report = report.unwrap_or_else(|| path.with_extension("report.json"));
            commands::convert::verify(&path, Some(&report))
        }
    }
}
