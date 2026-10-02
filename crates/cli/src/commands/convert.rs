use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use nc_convert::core::{Level, MetadataFile, Session};
use nc_convert::nwb::{self, NwbOptions, NwbPlan};
use nc_convert::{CancelToken, Event, Job, Registry, Stage};

#[derive(Args)]
pub struct ConvertArgs {
    /// Recording to convert (e.g. a TDT block folder)
    input: PathBuf,
    /// Metadata file (YAML): session, subject, electrode groups, per-stream mapping
    #[arg(short, long)]
    metadata: Option<PathBuf>,
    /// Output: a Zarr store (`name.nwb.zarr`) or, in builds with the `hdf5` feature, an HDF5 file (`name.nwb`)
    #[arg(short, long)]
    output: PathBuf,
    /// gzip level 1-9 for datasets
    #[arg(long, default_value_t = 1)]
    gzip: u32,
    /// Write uncompressed (fastest, ~30 % larger)
    #[arg(long)]
    no_compression: bool,
    /// Chunk length along time: seconds (default 1) or `auto` (about 10 MB per chunk, NeuroConv's
    /// default, so series of any rate and channel count get similar-sized chunks)
    #[arg(long, default_value = "1")]
    chunk: nwb::ChunkPolicy,
    /// Worker threads for copying data (default: every CPU this process may use)
    #[arg(long)]
    threads: Option<usize>,
    /// Replace the output if it exists
    #[arg(long)]
    overwrite: bool,
    /// Print the plan and exit without writing
    #[arg(long)]
    dry_run: bool,
    /// Compare the written data with the source: `full` (every block, and source files against
    /// the checksums their format records), `sampled` (first, last and random blocks), `off`
    #[arg(long, default_value = "full")]
    verify: nwb::VerifyLevel,
    /// Do not compare source files with the checksums their format records (e.g. SpikeGLX
    /// `fileSHA1` of a file edited after recording)
    #[arg(long)]
    skip_source_check: bool,
    #[command(flatten)]
    open: super::OpenArgs,
}

pub fn run(a: &ConvertArgs) -> anyhow::Result<()> {
    let mut job = Job::open(&Registry::builtin(), &a.input, &a.open.options())?;
    job.set_program("neuro-convert", env!("CARGO_PKG_VERSION"));
    let meta = match &a.metadata {
        Some(p) => MetadataFile::load(p)?,
        None => MetadataFile::default(),
    };
    job.plan(&meta);
    let plan = job.current_plan().expect("just planned");
    print_plan(plan, job.session());
    if a.dry_run {
        return Ok(());
    }
    if plan.has_errors() {
        anyhow::bail!("fix the errors above in the metadata file, then run again");
    }

    let gzip = (!a.no_compression).then_some(a.gzip);
    let mut options = NwbOptions { gzip, overwrite: a.overwrite, chunks: a.chunk, verify: a.verify, source_checksums: !a.skip_source_check, ..Default::default() };
    if let Some(t) = a.threads {
        options.threads = t;
    }
    // Ctrl-C stops the copy cleanly and removes the partial store
    let cancel = CancelToken::new();
    let on_signal = cancel.clone();
    ctrlc::set_handler(move || on_signal.cancel())?;

    println!("\nWriting {} ({} threads, {}) …", a.output.display(), options.threads, gzip.map_or("uncompressed".into(), |l| format!("gzip {l}")));
    let stage = std::sync::Mutex::new(Stage::Writing);
    let result = job.write(&a.output, &options, &cancel, &|e| match e {
        Event::Progress(p) => {
            let pct = if p.total > 0 { p.done as f64 * 100.0 / p.total as f64 } else { 100.0 };
            // Progress counts bytes (source check) or samples (all channels); series differ in
            // sample size, so no MB/s while writing
            let rate = p.done as f64 / 1e6 / p.elapsed.as_secs_f64().max(1e-9);
            let unit = if *stage.lock().unwrap() == Stage::CheckingSource { "MB/s       " } else { "M samples/s" };
            print!("\r  {pct:5.1}%  {rate:7.1} {unit}  {:5.0} s", p.elapsed.as_secs_f64());
            let _ = std::io::stdout().flush();
        }
        Event::Stage(s) => {
            *stage.lock().unwrap() = s;
            match s {
                Stage::CheckingSource => println!("  checking source checksums …"),
                Stage::Writing => {}
                Stage::Verifying => println!("\n  verifying the store ({:?}) …", options.verify),
                Stage::Done => {}
            }
        }
    });
    let report = match result {
        Err(nc_convert::Error::Cancelled) => anyhow::bail!("\ncancelled; the partial output was removed"),
        r => r?,
    };
    let s = &report.summary;
    println!("\nDone: {} series, {:.2} G samples in {:.1} s", s.series, s.samples as f64 / 1e9, s.seconds);
    for c in &report.source_checks {
        println!("source {} {}: {}", c.algorithm, if c.ok() { "ok" } else { "MISMATCH" }, c.path.display());
    }
    if let Some(d) = &report.digests {
        println!("{}", d.summary());
    }
    for i in &report.verification {
        println!("{} {}", if i.level == Level::Error { "ERROR:  " } else { "warning:" }, i.message);
    }
    let report_path = report.default_path();
    report.save(&report_path)?;
    println!("Report: {}", report_path.display());
    if report.has_errors() {
        anyhow::bail!("the written store failed verification (see above)");
    }
    Ok(())
}

fn print_plan(plan: &NwbPlan, session: &Session) {
    let f = &plan.file;
    println!("=== NWB plan ===");
    println!("identifier:   {}", f.identifier);
    println!("start time:   {}", f.start_time);
    println!("description:  {}", f.description);
    println!("subject:      {} ({})", plan.subject.id.as_deref().unwrap_or("?"), plan.subject.species.as_deref().unwrap_or("species?"));
    for g in &plan.groups {
        let n = session.electrodes.iter().filter(|e| e.group == g.name).count();
        println!("electrodes:   group {} at {} on {} ({n} electrodes)", g.name, g.location, g.device);
    }
    for s in &plan.series {
        let i = session.recordings[s.recording].info();
        let kind = if s.electrodes.is_some() { "ElectricalSeries" } else { "TimeSeries" };
        println!("acquisition/{:8} ← {:6} {kind:16} {:>3} ch  {:>10.1} Hz  {}", s.name, s.source, i.channel_count(), i.sample_rate, s.unit);
    }
    for e in &plan.events {
        let place = if e.table { "events" } else { "acquisition" };
        println!("{place}/{:8} ← {:6} ({} events)", e.name, e.source, session.events[e.event].len());
    }
    for p in &plan.snippets {
        let sn = &session.snippets[p.snippet];
        let sorted = sn.sort_codes.iter().any(|&c| c != 0);
        println!(
            "acquisition/{}_ch*  ← {:6} SpikeEventSeries  {:>3} ch  {} snippets x {} samples{}",
            p.name,
            p.source,
            p.rows.len(),
            sn.len(),
            sn.samples_per_snippet,
            if sorted { "  (+ sorted units in /units)" } else { "" }
        );
    }
    for t in &plan.tables {
        println!("analysis/{} ← table", t.name);
    }
    for s in &plan.skipped {
        println!("skipped:      {s}");
    }
    for i in &plan.issues {
        println!("{} {}", if i.level == Level::Error { "ERROR:  " } else { "warning:" }, i.message);
    }
}

pub fn validate(path: &std::path::Path) -> anyhow::Result<()> {
    let issues = nwb::validate::validate(path)?;
    let errors = issues.iter().filter(|i| i.level == Level::Error).count();
    for i in &issues {
        println!("{} {}", if i.level == Level::Error { "ERROR:  " } else { "warning:" }, i.message);
    }
    println!("{}: {errors} errors, {} warnings", path.display(), issues.len() - errors);
    if errors > 0 {
        anyhow::bail!("{} is not a valid NWB-Zarr store", path.display());
    }
    Ok(())
}

/// Structural checks plus, with a conversion report, the content digests it recorded.
pub fn verify(path: &std::path::Path, report: Option<&std::path::Path>) -> anyhow::Result<()> {
    let mut issues = nwb::validate::validate(path)?;
    if let Some(r) = report {
        let text = std::fs::read_to_string(r)?;
        let json: serde_json::Value = serde_json::from_str(&text)?;
        let saved: nwb::Digests = match json.get("digests") {
            Some(d) if !d.is_null() => serde_json::from_value(d.clone())?,
            _ => anyhow::bail!("{} has no digests (converted with --verify off?)", r.display()),
        };
        let (checked, content) = nwb::integrity::recheck(path, &saved, nwb::available_threads(0))?;
        issues.extend(content);
        println!("{}", checked.summary().replace("the source", "the report"));
    }
    let errors = issues.iter().filter(|i| i.level == Level::Error).count();
    for i in &issues {
        println!("{} {}", if i.level == Level::Error { "ERROR:  " } else { "warning:" }, i.message);
    }
    println!("{}: {errors} errors, {} warnings", path.display(), issues.len() - errors);
    if errors > 0 {
        anyhow::bail!("{} failed verification", path.display());
    }
    Ok(())
}
