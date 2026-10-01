//! A conversion as steps that both the CLI and the app drive:
//! [`Job::open`] → (inspect the session) → [`Job::plan`] (re-run as the metadata changes) →
//! [`Job::write`] (progress events, cancellable) → a [`Report`] that includes a verification of
//! the written store.
//!
//! A `Job` is `Send`, so a GUI can open and plan on one thread and write on a background one.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nc_core::{Detection, Error, Issue, Level, MetadataFile, OpenOptions, Provenance, Result, Session};
use nc_nwb::{NwbOptions, NwbPlan, Progress, WriteSummary};
use serde::Serialize;

use crate::Registry;

/// Shared stop flag: clone it into another thread (e.g. a Ctrl-C handler or a button) and call
/// [`CancelToken::cancel`]; a running [`Job::write`] stops between chunks.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Steps of [`Job::write`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Writing,
    Verifying,
    Done,
}

/// What [`Job::write`] reports while it runs (from its own threads: keep the handler cheap).
#[derive(Debug, Clone, Copy)]
pub enum Event {
    Stage(Stage),
    /// Continuous samples copied so far.
    Progress(Progress),
}

/// Everything about a finished conversion; saved next to the output as `<output>.report.json`.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub source: PathBuf,
    pub output: PathBuf,
    pub summary: WriteSummary,
    pub plan: NwbPlan,
    pub provenance: Provenance,
    /// Structural checks of the written store (`nc_nwb::validate`).
    pub verification: Vec<Issue>,
}

impl Report {
    /// The written store failed verification.
    pub fn has_errors(&self) -> bool {
        self.verification.iter().any(|i| i.level == Level::Error)
    }

    /// Default location: next to the output, `<output>.report.json`.
    pub fn default_path(&self) -> PathBuf {
        self.output.with_extension("report.json")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(|e| Error::format("report", e.to_string()))?;
        std::fs::write(path, text).map_err(|e| Error::io(path, e))
    }
}

pub struct Job {
    source: PathBuf,
    detection: Detection,
    session: Session,
    plan: Option<NwbPlan>,
    /// Generated once, so re-planning keeps the same NWB identifier.
    identifier: String,
}

impl Job {
    /// Detects the format of `path` and reads it.
    pub fn open(registry: &Registry, path: &Path, options: &OpenOptions) -> Result<Self> {
        let detection = registry.detect(path).into_iter().next().ok_or_else(|| Error::UnknownFormat(path.to_path_buf()))?;
        let session = registry.open(path, options)?;
        Ok(Self { source: path.to_path_buf(), detection, session, plan: None, identifier: nc_nwb::new_identifier() })
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn detection(&self) -> &Detection {
        &self.detection
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Model checks of the session as read (before any metadata is applied).
    pub fn issues(&self) -> Vec<Issue> {
        self.session.validate()
    }

    /// Plans the NWB output for `meta` (applying its electrode declarations to the session).
    /// Call again whenever the metadata changes; the latest plan is the one [`Job::write`] uses.
    pub fn plan(&mut self, meta: &MetadataFile) -> &NwbPlan {
        let id = self.identifier.clone();
        let plan = nc_nwb::plan(&mut self.session, meta, move || id);
        self.plan.insert(plan)
    }

    pub fn current_plan(&self) -> Option<&NwbPlan> {
        self.plan.as_ref()
    }

    /// Writes the planned NWB store to `dest`, then verifies it. `events` is called from worker
    /// threads. On cancellation the partly written store is removed and [`Error::Cancelled`]
    /// returned; on other errors it is left for inspection.
    pub fn write(&self, dest: &Path, options: &NwbOptions, cancel: &CancelToken, events: &(dyn Fn(Event) + Sync)) -> Result<Report> {
        let plan = self.plan.as_ref().ok_or_else(|| Error::Unsupported("plan the conversion before writing".into()))?;
        let options = NwbOptions { cancel: Some(cancel.0.clone()), ..options.clone() };
        events(Event::Stage(Stage::Writing));
        let summary = match nc_nwb::write(&self.session, plan, dest, &options, &|p| events(Event::Progress(p))) {
            Ok(s) => s,
            Err(Error::Cancelled) => {
                if dest.exists() {
                    std::fs::remove_dir_all(dest).map_err(|e| Error::io(dest, e))?;
                }
                return Err(Error::Cancelled);
            }
            Err(e) => return Err(e),
        };
        events(Event::Stage(Stage::Verifying));
        let verification = nc_nwb::validate::validate(dest)?;
        events(Event::Stage(Stage::Done));
        Ok(Report {
            source: self.source.clone(),
            output: dest.to_path_buf(),
            summary,
            plan: plan.clone(),
            provenance: self.session.provenance.clone(),
            verification,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use nc_core::{MemoryRecording, Reader};

    use super::*;

    /// A reader with one long-enough 2-channel recording to cancel mid-write.
    struct Fake;

    impl Reader for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn description(&self) -> &'static str {
            "test"
        }
        fn versions(&self) -> &'static [&'static str] {
            &[]
        }
        fn detect(&self, path: &Path) -> Option<Detection> {
            (path.extension()? == "fake").then_some(Detection { format: "fake", version: None, confidence: 1.0 })
        }
        fn open(&self, _: &Path, _: &OpenOptions) -> Result<Session> {
            let mut s = Session::default();
            s.metadata.start_time = Some("2025-01-01T00:00:00".into());
            s.recordings.push(Arc::new(MemoryRecording::new("sig", (0..200_000).map(|v| v as f32).collect(), 2, 1000.0, "V").unwrap()));
            s.provenance.format = "fake".into();
            Ok(s)
        }
    }

    fn out(name: &str) -> PathBuf {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/job-test").join(name);
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        p
    }

    const META: &str = "session: { description: job test, timezone: 'Z' }\n";

    #[test]
    fn test_open_plan_write_verify() {
        let registry = Registry::empty().with(Fake);
        let mut job = Job::open(&registry, Path::new("x.fake"), &OpenOptions::default()).unwrap();
        assert_eq!(job.detection().format, "fake");
        assert!(job.write(&out("unplanned.nwb.zarr"), &NwbOptions::default(), &CancelToken::new(), &|_| {}).is_err());

        // Re-planning keeps the identifier
        let first = job.plan(&MetadataFile::parse("session: { timezone: 'Z' }\n").unwrap()).clone();
        assert!(first.has_errors(), "no description yet");
        let plan = job.plan(&MetadataFile::parse(META).unwrap());
        assert!(!plan.has_errors(), "{:?}", plan.issues);
        assert_eq!(plan.file.identifier, first.file.identifier);

        let stages = Mutex::new(Vec::new());
        let dest = out("ok.nwb.zarr");
        let options = NwbOptions { gzip: None, threads: 2, ..Default::default() };
        let report = job
            .write(&dest, &options, &CancelToken::new(), &|e| {
                if let Event::Stage(s) = e {
                    stages.lock().unwrap().push(s)
                }
            })
            .unwrap();
        assert_eq!(*stages.lock().unwrap(), vec![Stage::Writing, Stage::Verifying, Stage::Done]);
        assert!(!report.has_errors(), "{:?}", report.verification);
        assert_eq!(report.summary.samples, 200_000);
        report.save(&report.default_path()).unwrap();
        assert!(dest.with_extension("report.json").exists());
    }

    #[test]
    fn test_cancel_removes_partial_output() {
        let registry = Registry::empty().with(Fake);
        let mut job = Job::open(&registry, Path::new("x.fake"), &OpenOptions::default()).unwrap();
        job.plan(&MetadataFile::parse(META).unwrap());
        let dest = out("cancelled.nwb.zarr");
        let cancel = CancelToken::new();
        // Cancel on the first progress report, i.e. once the store exists and copying has begun;
        // 1-sample chunks keep the copy running long enough
        let options = NwbOptions { gzip: None, threads: 1, chunks: nc_nwb::ChunkPolicy::Seconds(0.001), ..Default::default() };
        let started = AtomicBool::new(false);
        let result = job.write(&dest, &options, &cancel, &|e| {
            if let Event::Progress(_) = e {
                started.store(dest.exists(), Ordering::Relaxed);
                cancel.cancel();
            }
        });
        assert!(started.load(Ordering::Relaxed), "the store existed when cancelled");
        assert!(matches!(result, Err(Error::Cancelled)), "{:?}", result.err());
        assert!(!dest.exists(), "partial store removed");
    }
}
