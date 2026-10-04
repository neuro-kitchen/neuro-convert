//! The Source step: what can be opened (one card per reader), recent recordings, the container
//! choice, and what was detected. Listens to: Status, ChoiceOffered, RecordingOpened.

use std::path::PathBuf;

use gpui_kit::{App, Context, Entity, Subscription};

use crate::domain::format::{home, short_path};
use crate::domain::{AppEvent, PendingChoice};
use crate::state::Phase;
use crate::store::Store;
use crate::widgets::duration;

#[derive(Debug, Clone, PartialEq)]
pub struct FormatCard {
    pub name: String,
    pub description: String,
    pub opens: String,
    /// `verified` / `experimental` / `community`, and what it means.
    pub maturity: (String, String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecentItem {
    pub path: PathBuf,
    pub name: String,
    pub short: String,
    /// The detected format, when the path still exists and is recognized.
    pub format: Option<String>,
}

/// The open recording, as a one-line chip.
#[derive(Debug, Clone, PartialEq)]
pub struct Detected {
    pub format: String,
    pub detail: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Pick,
    Choosing(PendingChoice),
    Opening(PathBuf),
}

pub struct SourceVm {
    store: Entity<Store>,
    pub formats: Vec<FormatCard>,
    pub recent: Vec<RecentItem>,
    pub detected: Option<Detected>,
    _store: Subscription,
}

impl SourceVm {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let sub = cx.subscribe(&store, |this, _, event: &AppEvent, cx| match event {
            AppEvent::RecordingOpened => {
                this.refresh(cx);
                cx.notify();
            }
            AppEvent::Status | AppEvent::ChoiceOffered => cx.notify(),
            _ => {}
        });
        let mut this = Self { store, formats: Vec::new(), recent: Vec::new(), detected: None, _store: sub };
        this.refresh(cx);
        this
    }

    /// Reader cards, recent list (detection reads a few files: done here, not while drawing).
    fn refresh(&mut self, cx: &App) {
        let ws = &self.store.read(cx).ws;
        let registry = ws.registry();
        self.formats = registry.readers().map(|r| FormatCard { name: r.name().to_string(), description: r.description().to_string(), opens: r.opens().to_string(), maturity: (r.maturity().label().to_string(), r.maturity().explain().to_string()) }).collect();
        let home = home();
        self.recent = ws
            .settings
            .recent_recordings
            .iter()
            .filter(|p| p.exists())
            .map(|p| RecentItem {
                path: p.clone(),
                name: p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()),
                short: short_path(p.parent().unwrap_or(p), home.as_deref(), 60),
                format: registry.detect(p).first().map(|d| d.format.to_string()),
            })
            .collect();
        self.detected = ws.session().map(|s| Detected {
            format: s.provenance.format.clone(),
            detail: format!(
                "{}{} · {} streams",
                s.provenance.version.as_ref().map(|v| format!("{v} · ")).unwrap_or_default(),
                duration(s.duration()),
                s.recordings.len()
            ),
            path: ws.state.source.as_ref().map(|p| short_path(p, home.as_deref(), 80)).unwrap_or_default(),
        });
    }

    pub fn screen(&self, cx: &App) -> Screen {
        let ws = &self.store.read(cx).ws;
        match (&ws.state.phase, &ws.pending) {
            (Phase::Opening(p), _) => Screen::Opening(p.clone()),
            (_, Some(p)) => Screen::Choosing(p.clone()),
            _ => Screen::Pick,
        }
    }

    pub fn open(vm: &Entity<Self>, path: PathBuf, block: Option<String>, cx: &mut App) {
        let store = vm.read(cx).store.clone();
        store.update(cx, |s, cx| s.open(path, block, cx));
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::format::short_path;
    use std::path::Path;

    #[test]
    fn test_recent_paths_are_short() {
        let s = short_path(Path::new("/home/u/data/raw/tdt-examples"), Some(Path::new("/home/u")), 60);
        assert_eq!(s, "~/data/raw/tdt-examples");
    }
}
