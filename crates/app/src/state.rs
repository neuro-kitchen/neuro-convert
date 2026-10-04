//! What the app is doing, as plain data. It decides which actions are available and is the only
//! place state changes happen, so it is tested without a window. No GPUI types here.

// The open / write transitions are tested now and driven by the UI from M2 (open) and M4 (write)
#![allow(dead_code)]

use std::path::PathBuf;

/// The app's life cycle: Empty → Opening → Ready ⇄ Writing.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// No recording open.
    Empty,
    /// A recording is being read (background).
    Opening(PathBuf),
    /// A recording is open; metadata can be edited and planned.
    Ready,
    /// A conversion is running (background).
    Writing,
}

/// How the last conversion ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Converted(PathBuf),
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub phase: Phase,
    /// The open recording.
    pub source: Option<PathBuf>,
    /// The latest plan has errors (or there is no plan yet): converting is blocked.
    pub plan_has_errors: bool,
    pub last_outcome: Option<Outcome>,
    /// One-line message for the status bar.
    pub message: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self { phase: Phase::Empty, source: None, plan_has_errors: true, last_outcome: None, message: None }
    }
}

impl AppState {
    pub fn can_open(&self) -> bool {
        matches!(self.phase, Phase::Empty | Phase::Ready)
    }

    pub fn can_edit_metadata(&self) -> bool {
        self.phase == Phase::Ready
    }

    pub fn can_convert(&self) -> bool {
        self.phase == Phase::Ready && !self.plan_has_errors
    }

    pub fn can_cancel(&self) -> bool {
        self.phase == Phase::Writing
    }

    /// Starts opening `path`; `false` (nothing changes) when not allowed now.
    pub fn begin_open(&mut self, path: PathBuf) -> bool {
        if !self.can_open() {
            return false;
        }
        self.message = Some(format!("Opening {} …", path.display()));
        self.phase = Phase::Opening(path);
        true
    }

    /// The background open succeeded: the new recording replaces any previous one.
    pub fn opened(&mut self) {
        let Phase::Opening(path) = &self.phase else { return };
        let path = path.clone();
        self.phase = Phase::Ready;
        self.message = Some(format!("Opened {}", path.display()));
        self.source = Some(path);
        self.plan_has_errors = true;
        self.last_outcome = None;
    }

    /// The background open failed: back to the previous recording, if any.
    pub fn open_failed(&mut self, error: impl Into<String>) {
        if matches!(self.phase, Phase::Opening(_)) {
            self.phase = if self.source.is_some() { Phase::Ready } else { Phase::Empty };
            self.message = Some(format!("Could not open: {}", error.into()));
        }
    }

    /// Starts a conversion; `false` when not allowed now.
    pub fn begin_write(&mut self) -> bool {
        if !self.can_convert() {
            return false;
        }
        self.phase = Phase::Writing;
        self.message = Some("Converting …".into());
        true
    }

    pub fn finish_write(&mut self, outcome: Outcome) {
        if self.phase != Phase::Writing {
            return;
        }
        self.message = Some(match &outcome {
            Outcome::Converted(p) => format!("Converted to {}", p.display()),
            Outcome::Cancelled => "Cancelled; the partial output was removed".into(),
            Outcome::Failed(e) => format!("Conversion failed: {e}"),
        });
        self.last_outcome = Some(outcome);
        self.phase = Phase::Ready;
    }

    /// Short label of the phase for the status bar.
    pub fn phase_label(&self) -> &'static str {
        match self.phase {
            Phase::Empty => "No recording",
            Phase::Opening(_) => "Opening",
            Phase::Ready => "Ready",
            Phase::Writing => "Converting",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_and_convert_flow() {
        let mut s = AppState::default();
        assert!(s.can_open() && !s.can_edit_metadata() && !s.can_convert() && !s.can_cancel());
        assert!(s.begin_open("a".into()));
        assert!(!s.can_open() && !s.begin_open("b".into()), "one open at a time");
        s.opened();
        assert_eq!((s.phase.clone(), s.source.clone()), (Phase::Ready, Some("a".into())));
        assert!(s.can_edit_metadata() && !s.can_convert(), "no plan yet");

        s.plan_has_errors = false;
        assert!(s.begin_write());
        assert!(s.can_cancel() && !s.can_open() && !s.can_edit_metadata());
        s.finish_write(Outcome::Cancelled);
        assert_eq!(s.phase, Phase::Ready);
        assert_eq!(s.last_outcome, Some(Outcome::Cancelled));
    }

    #[test]
    fn test_failed_open_keeps_previous_recording() {
        let mut s = AppState::default();
        s.begin_open("a".into());
        s.open_failed("no reader");
        assert_eq!(s.phase, Phase::Empty);
        assert!(s.message.as_deref().unwrap().contains("no reader"));

        s.begin_open("a".into());
        s.opened();
        s.begin_open("b".into());
        s.open_failed("broken");
        assert_eq!((s.phase.clone(), s.source.clone()), (Phase::Ready, Some("a".into())));
    }

    #[test]
    fn test_transitions_ignored_out_of_phase() {
        let mut s = AppState::default();
        s.opened();
        s.finish_write(Outcome::Cancelled);
        assert_eq!(s, AppState::default());
        assert!(!s.begin_write());
    }
}
