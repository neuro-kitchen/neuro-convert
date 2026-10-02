//! The app's data and every rule for changing it, as plain Rust: the open recording (`Job`), the
//! metadata draft, the latest plan, write options and progress, the last report, settings.
//! Each change returns the [`Events`] it causes. Nothing here waits or spawns: long work is split
//! into a `begin_*` (hands work to a service) and a `finish_*` (takes the result).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nc_convert::core::{ItemKind, MetadataFile, OpenOptions, Session};
use nc_convert::nwb::{self, ChunkPolicy, NwbOptions, NwbPlan, Progress, VerifyLevel};
use nc_convert::{CancelToken, Event, Job, Registry, Report, Stage};

use super::events::{AppEvent, Events};
use crate::settings::Settings;
use crate::state::{AppState, Outcome, Phase};

/// How the store is written (the convert panel's options).
#[derive(Debug, Clone, PartialEq)]
pub struct WriteOptions {
    /// gzip level 1–9, `None` = uncompressed.
    pub gzip: Option<u32>,
    pub chunks: ChunkPolicy,
    pub threads: usize,
    /// How much of the written data is compared with the source afterwards.
    pub verify: VerifyLevel,
}

/// A path that holds several recordings: the user picks one.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingChoice {
    pub path: PathBuf,
    pub containers: Vec<String>,
}

/// Everything a writer thread needs; the job comes back with the result.
pub struct WriteTicket {
    pub job: Job,
    pub output: PathBuf,
    pub options: NwbOptions,
    pub cancel: CancelToken,
}

pub struct Workspace {
    pub state: AppState,
    registry: Arc<Registry>,
    pub job: Option<Job>,
    pub pending: Option<PendingChoice>,
    pub meta: MetadataFile,
    pub metadata_path: Option<PathBuf>,
    pub metadata_dirty: bool,
    /// The latest plan (kept while the job is away writing).
    pub plan: Option<NwbPlan>,
    pub output: Option<PathBuf>,
    pub options: WriteOptions,
    pub stage: Option<Stage>,
    pub progress: Option<Progress>,
    cancel: Option<CancelToken>,
    pub report: Option<Report>,
    pub settings: Settings,
    settings_path: Option<PathBuf>,
    /// Recording shown in the preview.
    pub preview: Option<String>,
}

impl Workspace {
    pub fn new(registry: Arc<Registry>, settings: Settings, settings_path: Option<PathBuf>) -> Self {
        let options = WriteOptions {
            gzip: settings.gzip,
            chunks: if settings.auto_chunks { ChunkPolicy::Auto } else { ChunkPolicy::Seconds(1.0) },
            threads: nwb::available_threads(settings.reserved_threads),
            verify: settings.verify,
        };
        Self {
            state: AppState::default(),
            registry,
            job: None,
            pending: None,
            meta: MetadataFile::default(),
            metadata_path: None,
            metadata_dirty: false,
            plan: None,
            output: None,
            options,
            stage: None,
            progress: None,
            cancel: None,
            report: None,
            settings,
            settings_path,
            preview: None,
        }
    }

    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }

    pub fn session(&self) -> Option<&Session> {
        self.job.as_ref().map(Job::session)
    }

    pub fn save_settings(&self) {
        if let Some(p) = &self.settings_path {
            let _ = self.settings.save(p);
        }
    }

    /// Step 1 of opening: a container (tank, multi-run folder) without a chosen `block` offers
    /// the choice instead; otherwise returns the open options to read `path` with.
    pub fn begin_open(&mut self, path: PathBuf, block: Option<String>, events: &mut Events) -> Option<OpenOptions> {
        if !self.state.can_open() {
            return None;
        }
        if block.is_none() {
            let containers = self.registry.containers(&path);
            if containers.len() > 1 {
                self.pending = Some(PendingChoice { path, containers });
                self.state.message = Some("Choose which recording to open".into());
                events.push(AppEvent::ChoiceOffered);
                events.push(AppEvent::Status);
                return None;
            }
        }
        self.pending = None;
        self.state.begin_open(path);
        events.push(AppEvent::Status);
        events.push(AppEvent::ChoiceOffered);
        Some(OpenOptions { block, ..Default::default() })
    }

    /// Step 2 of opening: the reader's result.
    pub fn finish_open(&mut self, result: nc_convert::Result<Job>) -> Events {
        let mut events = Events::from([AppEvent::Status]);
        let Phase::Opening(source) = self.state.phase.clone() else { return events };
        let job = match result {
            Ok(job) => job,
            Err(e) => {
                self.state.open_failed(e.to_string());
                return events;
            }
        };
        // A loaded metadata file is kept for the next recording; otherwise start fresh
        if self.metadata_path.is_none() {
            self.meta = MetadataFile::template(job.session());
            self.metadata_dirty = false;
        }
        self.output = Some(self.default_output(&source));
        let s = job.session();
        self.preview = s
            .recordings
            .iter()
            .find(|r| s.channel_electrodes(&r.info().name).iter().any(Option::is_some))
            .or_else(|| s.recordings.first())
            .map(|r| r.info().name.clone());
        self.job = Some(job);
        self.report = None;
        self.state.opened();
        self.settings.remember_recording(&source);
        self.save_settings();
        events.extend(Events::from([AppEvent::RecordingOpened, AppEvent::OutputChanged, AppEvent::PreviewRequested, AppEvent::MetadataChanged]));
        events.extend(self.replan());
        events
    }

    /// `<output folder or the recording's folder>/<recording name>.nwb.zarr`.
    pub fn default_output(&self, source: &Path) -> PathBuf {
        let stem = source.file_stem().map_or_else(|| "recording".into(), |s| s.to_string_lossy().into_owned());
        let dir = self.settings.output_dir.clone().or_else(|| source.parent().map(Path::to_path_buf)).unwrap_or_default();
        dir.join(format!("{stem}.nwb.zarr"))
    }

    /// Re-plans with the current metadata (cheap: no samples are read).
    pub fn replan(&mut self) -> Events {
        let Some(job) = &mut self.job else { return Events::default() };
        let plan = job.plan(&self.meta).clone();
        let blocked = plan.has_errors();
        self.plan = Some(plan);
        let mut events = Events::from([AppEvent::PlanUpdated]);
        if blocked != self.state.plan_has_errors {
            self.state.plan_has_errors = blocked;
            events.push(AppEvent::Status);
        }
        events
    }

    /// Replaces the metadata draft and re-plans.
    pub fn set_meta(&mut self, meta: MetadataFile) -> Events {
        if meta == self.meta {
            return Events::default();
        }
        self.meta = meta;
        let mut events = Events::from([AppEvent::MetadataChanged]);
        if !self.metadata_dirty {
            self.metadata_dirty = true;
            events.push(AppEvent::Status);
        }
        events.extend(self.replan());
        events
    }

    pub fn included(&self, kind: ItemKind, name: &str) -> bool {
        self.meta.included(kind, name)
    }

    pub fn set_included(&mut self, kind: ItemKind, names: &[String], include: bool) -> Events {
        let mut meta = self.meta.clone();
        for n in names {
            meta.set_included(kind, n, include);
        }
        self.set_meta(meta)
    }

    pub fn load_metadata(&mut self, path: &Path) -> Events {
        let mut events = Events::from([AppEvent::Status]);
        match MetadataFile::load(path) {
            Ok(meta) => {
                self.meta = meta;
                self.metadata_path = Some(path.to_path_buf());
                self.metadata_dirty = false;
                self.settings.remember_metadata(path);
                self.save_settings();
                self.state.message = Some(format!("Loaded {}", path.display()));
                events.extend(Events::from([AppEvent::MetadataReloaded, AppEvent::MetadataChanged]));
                events.extend(self.replan());
            }
            Err(e) => self.state.message = Some(format!("Could not load {}: {e}", path.display())),
        }
        events
    }

    pub fn save_metadata(&mut self, path: &Path) -> Events {
        self.state.message = Some(match self.meta.save(path) {
            Ok(()) => {
                self.metadata_path = Some(path.to_path_buf());
                self.metadata_dirty = false;
                self.settings.remember_metadata(path);
                self.remember_values();
                format!("Saved {} (comments of a loaded file are not kept)", path.display())
            }
            Err(e) => format!("Could not save: {e}"),
        });
        Events::from([AppEvent::Status])
    }

    pub fn set_output(&mut self, output: Option<PathBuf>) -> Events {
        if output == self.output {
            return Events::default();
        }
        self.output = output;
        Events::from([AppEvent::OutputChanged])
    }

    pub fn set_options(&mut self, options: WriteOptions) -> Events {
        if options == self.options {
            return Events::default();
        }
        self.settings.gzip = options.gzip;
        self.settings.auto_chunks = options.chunks == ChunkPolicy::Auto;
        self.settings.verify = options.verify;
        self.options = options;
        self.save_settings();
        Events::from([AppEvent::OptionsChanged])
    }

    /// Opens or closes side panels (remembered; views read `settings.panels`).
    pub fn set_panels(&mut self, panels: crate::settings::Panels) {
        if panels != self.settings.panels {
            self.settings.panels = panels;
            self.save_settings();
        }
    }

    /// Whether DANDI recommendations count as issues (remembered).
    pub fn set_dandi(&mut self, on: bool) -> Events {
        if on == self.settings.dandi {
            return Events::default();
        }
        self.settings.dandi = on;
        self.save_settings();
        Events::from([AppEvent::PlanUpdated])
    }

    /// Remembers the metadata values worth suggesting next time (labs, species, locations…).
    pub fn remember_values(&mut self) {
        let m = &self.meta;
        let mut values: Vec<(&str, String)> = Vec::new();
        for (field, v) in [("lab", &m.session.lab), ("institution", &m.session.institution), ("species", &m.subject.species), ("strain", &m.subject.strain)] {
            if let Some(v) = v {
                values.push((field, v.clone()));
            }
        }
        values.extend(m.session.experimenters.iter().map(|v| ("experimenter", v.clone())));
        for g in &m.electrode_groups {
            values.push(("location", g.location.clone()));
            if let Some(d) = &g.device {
                values.push(("device", d.clone()));
            }
        }
        for (field, v) in values {
            self.settings.remember_value(field, &v);
        }
        self.save_settings();
    }

    pub fn request_preview(&mut self, name: &str) -> Events {
        self.preview = Some(name.to_string());
        Events::from([AppEvent::PreviewRequested])
    }

    pub fn set_theme(&mut self, dark: bool) {
        self.settings.theme = Some(if dark { "dark" } else { "light" }.into());
        self.save_settings();
    }

    /// Step 1 of writing: hands the job to a writer (`None` when converting is not allowed now).
    pub fn begin_write(&mut self, events: &mut Events) -> Option<WriteTicket> {
        let output = self.output.clone()?;
        if self.job.is_none() || !self.state.begin_write() {
            return None;
        }
        self.remember_values();
        let cancel = CancelToken::new();
        self.cancel = Some(cancel.clone());
        self.stage = Some(Stage::Writing);
        self.progress = None;
        self.report = None;
        events.extend(Events::from([AppEvent::Status, AppEvent::WriteProgress]));
        let o = &self.options;
        let options = NwbOptions { gzip: o.gzip, chunks: o.chunks, threads: o.threads.max(1), overwrite: true, verify: o.verify, ..NwbOptions::default() };
        Some(WriteTicket { job: self.job.take().expect("checked"), output, options, cancel })
    }

    /// A writer event (progress or stage).
    pub fn write_event(&mut self, event: Event) -> Events {
        match event {
            Event::Progress(p) => self.progress = Some(p),
            Event::Stage(s) => {
                // Each stage counts its own progress
                self.stage = Some(s);
                self.progress = None;
            }
        }
        Events::from([AppEvent::WriteProgress])
    }

    /// Step 2 of writing: the job comes back with its result.
    pub fn finish_write(&mut self, job: Job, result: nc_convert::Result<Report>) -> Events {
        self.job = Some(job);
        self.cancel = None;
        self.stage = None;
        let outcome = match result {
            Ok(report) => {
                let outcome = if report.has_errors() { Outcome::Failed("the written store failed verification".into()) } else { Outcome::Converted(report.output.clone()) };
                let _ = report.save(&report.default_path());
                self.report = Some(report);
                outcome
            }
            Err(nc_convert::Error::Cancelled) => Outcome::Cancelled,
            Err(e) => Outcome::Failed(e.to_string()),
        };
        self.state.finish_write(outcome);
        Events::from([AppEvent::WriteFinished, AppEvent::Status])
    }

    pub fn cancel(&mut self) -> Events {
        match &self.cancel {
            Some(c) => {
                c.cancel();
                self.state.message = Some("Cancelling …".into());
                Events::from([AppEvent::Status])
            }
            None => Events::default(),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use nc_convert::core::{Detection, EventSeries, MemoryRecording, Reader};

    /// A reader for `*.fake` paths: two recordings and one event series; `tank.fake` holds two
    /// blocks.
    pub struct Fake;

    impl Reader for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn description(&self) -> &'static str {
            "test reader"
        }
        fn opens(&self) -> &'static str {
            "A .fake file"
        }
        fn versions(&self) -> &'static [&'static str] {
            &[]
        }
        fn detect(&self, path: &Path) -> Option<Detection> {
            (path.extension()? == "fake").then_some(Detection { format: "fake", version: None, confidence: 1.0 })
        }
        fn containers(&self, path: &Path) -> Vec<String> {
            if path.file_stem().is_some_and(|s| s == "tank") { vec!["b1".into(), "b2".into()] } else { vec![] }
        }
        fn open(&self, _: &Path, _: &OpenOptions) -> nc_convert::Result<Session> {
            let mut s = Session::default();
            s.metadata.start_time = Some("2025-01-01T00:00:00".into());
            s.metadata.subject.id = Some("rat7".into());
            s.recordings.push(Arc::new(MemoryRecording::new("Wav1", (0..4000).map(|v| v as f32).collect(), 2, 1000.0, "V").unwrap()));
            s.recordings.push(Arc::new(MemoryRecording::new("Temp", vec![0.5; 100], 1, 10.0, "a.u.").unwrap()));
            s.events.push(EventSeries { name: "Tick".into(), onsets: vec![0.1, 0.2], values: vec![1.0, 2.0], channels: 1, ..Default::default() });
            s.provenance.format = "fake".into();
            Ok(s)
        }
    }

    pub fn workspace(dir: &Path) -> Workspace {
        let settings = Settings { output_dir: Some(dir.to_path_buf()), gzip: None, ..Settings::default() };
        Workspace::new(Arc::new(Registry::empty().with(Fake)), settings, None)
    }

    pub fn scratch(name: &str) -> PathBuf {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/app-test").join(name);
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// Opens synchronously (what the open service does on its thread).
    pub fn open(ws: &mut Workspace, path: &str) -> Events {
        let mut events = Events::default();
        let Some(options) = ws.begin_open(PathBuf::from(path), None, &mut events) else { return events };
        let result = Job::open(&ws.registry(), Path::new(path), &options);
        events.extend(ws.finish_open(result));
        events
    }

    #[test]
    fn test_open_plan_select_and_convert() {
        let dir = scratch("convert");
        let mut ws = workspace(&dir);
        let events = open(&mut ws, "session.fake");
        for e in [AppEvent::RecordingOpened, AppEvent::PlanUpdated, AppEvent::PreviewRequested, AppEvent::OutputChanged] {
            assert!(events.contains(e), "{e:?} in {events:?}");
        }
        assert_eq!(ws.meta.subject.id.as_deref(), Some("rat7"), "template from the session");
        assert_eq!(ws.preview.as_deref(), Some("Wav1"));
        assert!(ws.state.plan_has_errors);

        let mut meta = ws.meta.clone();
        meta.session.description = Some("test".into());
        meta.session.timezone = Some("Z".into());
        meta.session.lab = Some("Lab A".into());
        let events = ws.set_meta(meta);
        assert!(events.contains(AppEvent::MetadataChanged) && events.contains(AppEvent::PlanUpdated) && events.contains(AppEvent::Status));
        assert!(!ws.state.plan_has_errors, "{:?}", ws.plan.as_ref().unwrap().issues);
        assert!(ws.set_meta(ws.meta.clone()).is_empty(), "no change, no events");

        let events = ws.set_included(ItemKind::Stream, &["Temp".into()], false);
        assert!(events.contains(AppEvent::PlanUpdated) && !events.contains(AppEvent::Status), "plan errors unchanged: no status event");
        assert_eq!(ws.plan.as_ref().unwrap().skipped, vec!["stream Temp".to_string()]);

        let mut events = Events::default();
        let t = ws.begin_write(&mut events).unwrap();
        assert!(ws.job.is_none() && ws.state.phase == Phase::Writing && !ws.state.can_edit_metadata());
        assert_eq!(ws.settings.remembered("lab"), ["Lab A"], "typed values are remembered when converting");
        let result = t.job.write(&t.output, &t.options, &t.cancel, &|_| {});
        let events = ws.finish_write(t.job, result);
        assert!(events.contains(AppEvent::WriteFinished));
        assert!(matches!(ws.state.last_outcome, Some(Outcome::Converted(_))), "{:?}", ws.state.last_outcome);
        assert!(ws.report.as_ref().unwrap().default_path().exists());
        assert!(ws.job.is_some(), "the job comes back after writing");
    }

    #[test]
    fn test_containers_metadata_files_and_failures() {
        let dir = scratch("metadata");
        let mut ws = workspace(&dir);
        let events = open(&mut ws, "tank.fake");
        assert!(events.contains(AppEvent::ChoiceOffered));
        assert_eq!(ws.pending.as_ref().unwrap().containers, vec!["b1", "b2"]);
        assert_eq!(ws.state.phase, Phase::Empty);

        open(&mut ws, "a.fake");
        ws.set_included(ItemKind::Event, &["Tick".into()], false);
        let file = dir.join("meta.yaml");
        ws.save_metadata(&file);
        assert!(!ws.metadata_dirty && std::fs::read_to_string(&file).unwrap().contains("Tick"));

        let mut other = workspace(&dir);
        assert!(other.load_metadata(&file).contains(AppEvent::MetadataReloaded));
        open(&mut other, "b.fake");
        assert!(!other.included(ItemKind::Event, "Tick"), "a loaded file survives opening another recording");

        let mut bad = workspace(&dir);
        open(&mut bad, "x.unknown");
        assert_eq!(bad.state.phase, Phase::Empty);
        assert!(bad.state.message.as_deref().unwrap().contains("no reader"));
        assert!(bad.begin_write(&mut Events::default()).is_none(), "nothing open");
    }
}
