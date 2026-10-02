//! The convert screen: output path field, options, progress and result, as display state built
//! by pure functions ([`convert_state`], [`diagnostics`]).
//! Listens to: OutputChanged, OptionsChanged, WriteProgress, WriteFinished, Status, PlanUpdated.

use std::path::PathBuf;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{App, AppContext as _, ClipboardItem, Context, Entity, Subscription, Window};
use nc_convert::core::Level;
use nc_convert::nwb::ChunkPolicy;
use nc_convert::Stage;

use crate::domain::{AppEvent, Workspace, WriteOptions};
use crate::state::{Outcome, Phase};
use crate::store::Store;
use crate::widgets::{duration, IssueRow};

/// Everything the convert panel shows.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvertState {
    pub opened: bool,
    pub writing: bool,
    pub can_convert: bool,
    pub can_cancel: bool,
    /// The plan has errors (why Convert is disabled).
    pub blocked: bool,
    /// What blocks converting, e.g. `2 errors: Description, Time zone`.
    pub blocked_reason: Option<String>,
    pub options: WriteOptions,
    /// Percent and text, while writing.
    pub progress: Option<(f32, String)>,
    pub result: Option<ResultView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultView {
    pub ok: Option<bool>,
    pub headline: String,
    pub summary: Option<String>,
    pub issues: Vec<IssueRow>,
    pub output: Option<PathBuf>,
    pub report: Option<PathBuf>,
}

pub fn convert_state(ws: &Workspace) -> ConvertState {
    let writing = ws.state.phase == Phase::Writing;
    let progress = writing.then(|| progress_line(ws.stage, ws.progress.as_ref()));
    let result = ws.state.last_outcome.as_ref().map(|o| {
        let (ok, headline) = match o {
            Outcome::Converted(p) => (Some(true), format!("Converted and verified: {}", p.display())),
            Outcome::Cancelled => (None, "Cancelled; the partial output was removed.".to_string()),
            Outcome::Failed(e) => (Some(false), format!("Failed: {e}")),
        };
        let r = ws.report.as_ref();
        ResultView {
            ok,
            headline,
            summary: r.map(|r| {
                let written = format!("{} series, {:.2} G samples in {}", r.summary.series, r.summary.samples as f64 / 1e9, duration(r.summary.seconds));
                let source = (!r.source_checks.is_empty()).then(|| format!("; source checksums: {}/{} match", r.source_checks.iter().filter(|c| c.ok()).count(), r.source_checks.len()));
                let content = r.digests.as_ref().map_or_else(|| "; content not compared (verification off)".to_string(), |d| format!("; {}", d.summary()));
                format!("{written}{}{content}", source.unwrap_or_default())
            }),
            issues: r.map(|r| r.verification.iter().map(|i| IssueRow { error: i.level == Level::Error, text: i.message.clone().into(), target: i.target.clone() }).collect()).unwrap_or_default(),
            output: r.map(|r| r.output.clone()),
            report: r.map(|r| r.default_path()),
        }
    });
    ConvertState {
        opened: ws.session().is_some() || writing,
        writing,
        can_convert: ws.state.can_convert(),
        can_cancel: ws.state.can_cancel(),
        blocked: ws.plan.as_ref().is_some_and(|p| p.has_errors()),
        blocked_reason: ws.plan.as_ref().and_then(blocked_reason),
        options: ws.options.clone(),
        progress,
        result,
    }
}

/// Percent and text for the running stage: hashing the source (MB/s), writing (samples/s, time
/// left) or comparing with the source.
pub fn progress_line(stage: Option<Stage>, p: Option<&nc_convert::nwb::Progress>) -> (f32, String) {
    let Some(p) = p else {
        return match stage {
            Some(Stage::CheckingSource) => (0.0, "Checking the source files' checksums…".to_string()),
            Some(Stage::Verifying) => (0.0, "Comparing the written data with the source…".to_string()),
            _ => (0.0, "Starting…".to_string()),
        };
    };
    let pct = if p.total > 0 { p.done as f64 * 100.0 / p.total as f64 } else { 0.0 };
    let secs = p.elapsed.as_secs_f64();
    let rate = p.done as f64 / 1e6 / secs.max(1e-9);
    let left = if p.done > 0 { secs * p.total.saturating_sub(p.done) as f64 / p.done as f64 } else { f64::NAN };
    let left = if left.is_finite() { format!(", about {} left", duration(left)) } else { String::new() };
    let line = match stage {
        Some(Stage::CheckingSource) => format!("Checking source checksums: {pct:.1} % · {rate:.0} MB/s{left}"),
        Some(Stage::Verifying) => format!("Comparing with the source: {pct:.1} %{left}"),
        _ => format!("{pct:.1} % · {rate:.1} M samples/s · {}{left}", duration(secs)),
    };
    (pct as f32, line)
}

/// `N errors: Description, Time zone, …` (`None` without errors).
pub fn blocked_reason(plan: &nc_convert::nwb::NwbPlan) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    for i in plan.issues.iter().filter(|i| i.level == Level::Error) {
        let n = i.target.as_ref().map_or_else(|| "NWB plan".to_string(), crate::domain::steps::target_label);
        if !names.contains(&n) {
            names.push(n);
        }
    }
    let count = plan.issues.iter().filter(|i| i.level == Level::Error).count();
    (count > 0).then(|| format!("{count} error{} to fix: {}", if count == 1 { "" } else { "s" }, names.join(", ")))
}

/// Versions, recording, reader warnings, plan issues and the last outcome, for bug reports.
pub fn diagnostics(ws: &Workspace) -> String {
    let mut d = format!("neuro-convert-app {}\nrecording: {:?}\n", env!("CARGO_PKG_VERSION"), ws.state.source);
    if let Some(s) = ws.session() {
        d += &format!("format: {} {:?}\n", s.provenance.format, s.provenance.version);
        for w in &s.provenance.warnings {
            d += &format!("reader warning: {w}\n");
        }
    }
    if let Some(p) = &ws.plan {
        for i in &p.issues {
            d += &format!("plan {:?}: {}\n", i.level, i.message);
        }
    }
    d + &format!("output: {:?}\noutcome: {:?}\n", ws.output, ws.state.last_outcome)
}

pub struct ConvertVm {
    store: Entity<Store>,
    pub output: Entity<InputState>,
    pub state: ConvertState,
    _subscriptions: Vec<Subscription>,
}

impl ConvertVm {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let output = cx.new(|cx| InputState::new(window, cx).placeholder("name.nwb.zarr"));
        let subscriptions = vec![
            cx.subscribe_in(&store, window, |this, store, event: &AppEvent, window, cx| {
                if *event == AppEvent::OutputChanged {
                    let text = store.read(cx).ws.output.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if this.output.read(cx).value() != text {
                        this.output.update(cx, |s, cx| s.set_value(text, window, cx));
                    }
                }
                if matches!(event, AppEvent::OutputChanged | AppEvent::OptionsChanged | AppEvent::WriteProgress | AppEvent::WriteFinished | AppEvent::Status | AppEvent::PlanUpdated | AppEvent::RecordingOpened) {
                    this.refresh(cx);
                }
            }),
            cx.subscribe(&output, |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    // The input holds the store's name; the folder comes from Choose…
                    let text = state.read(cx).value().trim().to_string();
                    let folder = this.store.read(cx).ws.output.as_ref().and_then(|p| p.parent()).map(PathBuf::from);
                    let path = (!text.is_empty()).then(|| folder.map_or_else(|| PathBuf::from(&text), |f| f.join(&text)));
                    this.store.update(cx, |s, cx| s.apply(cx, |ws| ws.set_output(path)));
                }
            }),
        ];
        let state = convert_state(&store.read(cx).ws);
        Self { store, output, state, _subscriptions: subscriptions }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let state = convert_state(&self.store.read(cx).ws);
        if state != self.state {
            self.state = state;
            cx.notify();
        }
    }

    fn set_options(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut WriteOptions)) {
        self.store.update(cx, |s, cx| {
            let mut o = s.ws.options.clone();
            f(&mut o);
            s.apply(cx, |ws| ws.set_options(o));
        });
    }

    pub fn set_gzip(&mut self, on: bool, cx: &mut Context<Self>) {
        self.set_options(cx, |o| o.gzip = on.then_some(1));
    }

    pub fn set_chunks(&mut self, chunks: ChunkPolicy, cx: &mut Context<Self>) {
        self.set_options(cx, |o| o.chunks = chunks);
    }

    /// The NWB structure panel is open (remembered).
    pub fn structure_open(&self, cx: &App) -> bool {
        self.store.read(cx).ws.settings.panels.structure
    }

    pub fn set_structure_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.store.update(cx, |s, _| {
            let mut p = s.ws.settings.panels;
            p.structure = open;
            s.ws.set_panels(p);
        });
        cx.notify();
    }

    /// The output's folder: (shortened, full) for the line under the name.
    pub fn folder(&self, cx: &App) -> Option<(String, String)> {
        let parent = self.store.read(cx).ws.output.as_ref()?.parent()?.to_path_buf();
        Some((crate::domain::format::short_path(&parent, crate::domain::format::home().as_deref(), 60), parent.display().to_string()))
    }

    pub fn set_verify(&mut self, verify: nc_convert::nwb::VerifyLevel, cx: &mut Context<Self>) {
        self.set_options(cx, |o| o.verify = verify);
    }

    pub fn step_threads(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.set_options(cx, |o| o.threads = (o.threads as isize + delta).max(1) as usize);
    }

    pub fn copy_diagnostics(&self, cx: &mut Context<Self>) {
        let text = diagnostics(&self.store.read(cx).ws);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workspace::tests::{open, scratch, workspace};
    use crate::domain::Events;

    #[test]
    fn test_state_through_a_conversion() {
        let mut ws = workspace(&scratch("convert-vm"));
        assert!(!convert_state(&ws).opened);
        open(&mut ws, "session.fake");
        let s = convert_state(&ws);
        assert!(s.opened && s.blocked && !s.can_convert && s.progress.is_none());
        assert_eq!(s.blocked_reason.as_deref(), Some("2 errors to fix: Description, Time zone"));

        let mut meta = ws.meta.clone();
        meta.session.description = Some("d".into());
        meta.session.timezone = Some("Z".into());
        ws.set_meta(meta);
        let t = ws.begin_write(&mut Events::default()).unwrap();
        ws.write_event(nc_convert::Event::Progress(nc_convert::nwb::Progress { done: 50, total: 200, elapsed: std::time::Duration::from_secs(2) }));
        let (pct, line) = convert_state(&ws).progress.unwrap();
        assert_eq!(pct, 25.0);
        assert!(line.starts_with("25.0 % · 0.0 M samples/s · 2.00 s, about 6.00 s left"), "{line}");
        assert!(convert_state(&ws).can_cancel);
        // A new stage starts its own count
        ws.write_event(nc_convert::Event::Stage(Stage::Verifying));
        assert_eq!(convert_state(&ws).progress.unwrap().1, "Comparing the written data with the source…");

        let result = t.job.write(&t.output, &t.options, &t.cancel, &|_| {});
        ws.finish_write(t.job, result);
        let r = convert_state(&ws).result.unwrap();
        assert_eq!(r.ok, Some(true));
        let summary = r.summary.unwrap();
        assert!(summary.starts_with("2 series") && summary.contains("content matches the source"), "{summary}");
        assert!(r.issues.is_empty() && r.report.unwrap().exists());
        assert!(diagnostics(&ws).contains("outcome: Some(Converted("));
    }
}
