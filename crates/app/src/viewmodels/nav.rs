//! Which step is on screen, which steps can be entered, and issue counts per step. "Reveal" an
//! issue: go to its step and announce its target so that step's view model can select or focus
//! it. Listens to: RecordingOpened, Status, PlanUpdated.

use gpui_kit::{App, Context, Entity, EventEmitter, Subscription};
use nc_convert::core::{Issue, Target};

use crate::domain::steps::{per_step, Step};
use crate::domain::AppEvent;
use crate::state::Phase;
use crate::store::Store;

pub enum NavEvent {
    /// Select or focus what this target names (the step is already shown).
    Reveal(Target),
}

pub struct NavVm {
    store: Entity<Store>,
    pub step: Step,
    _store: Subscription,
}

impl EventEmitter<NavEvent> for NavVm {}

impl NavVm {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let sub = cx.subscribe(&store, |this, store, event: &AppEvent, cx| match event {
            // A new recording: on to its contents
            AppEvent::RecordingOpened => {
                this.step = Step::Contents;
                cx.notify();
            }
            AppEvent::Status | AppEvent::ChoiceOffered => {
                let ws = &store.read(cx).ws;
                // Opening, choosing a block, or nothing open: back to the source
                if ws.pending.is_some() || matches!(ws.state.phase, Phase::Opening(_)) || (ws.session().is_none() && ws.state.phase != Phase::Writing) {
                    this.step = Step::Source;
                }
                cx.notify();
            }
            AppEvent::PlanUpdated => cx.notify(),
            _ => {}
        });
        Self { store, step: Step::Source, _store: sub }
    }

    /// Steps after Source need an open recording.
    pub fn enabled(&self, step: Step, cx: &App) -> bool {
        let ws = &self.store.read(cx).ws;
        step == Step::Source || ws.session().is_some() || ws.state.phase == Phase::Writing
    }

    pub fn go(&mut self, step: Step, cx: &mut Context<Self>) {
        if self.enabled(step, cx) && step != self.step {
            self.step = step;
            cx.notify();
        }
    }

    /// Shows where `target` is fixed.
    pub fn reveal(&mut self, target: Target, cx: &mut Context<Self>) {
        self.go(Step::of(Some(&target)), cx);
        cx.emit(NavEvent::Reveal(target));
    }

    /// Issues of the current plan that count (DANDI-only ones when the switch is on).
    pub fn issues(&self, cx: &App) -> Vec<Issue> {
        let ws = &self.store.read(cx).ws;
        let dandi = ws.settings.dandi;
        let mut issues: Vec<Issue> = ws.plan.as_ref().map(|p| p.issues.iter().filter(|i| crate::domain::steps::counts(i, dandi)).cloned().collect()).unwrap_or_default();
        issues.sort_by_key(|i| i.level != nc_convert::core::Level::Error);
        issues
    }

    pub fn counts(&self, cx: &App) -> [(usize, usize); 4] {
        let ws = &self.store.read(cx).ws;
        ws.plan.as_ref().map(|p| per_step(&p.issues, ws.settings.dandi)).unwrap_or_default()
    }
}
