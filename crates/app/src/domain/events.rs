//! What changed in the workspace. Each view model listens only to the events it shows, so a
//! progress tick re-renders the convert panel and nothing else.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppEvent {
    /// Phase or status message changed (title, toolbar, status bar).
    Status,
    /// A path holds several recordings; the user must pick one.
    ChoiceOffered,
    /// A new recording is open (tree, form fields, preview, output path).
    RecordingOpened,
    /// A metadata file was loaded: form fields must be rebuilt.
    MetadataReloaded,
    /// The metadata draft changed (edits, include flags).
    MetadataChanged,
    /// A new NWB plan.
    PlanUpdated,
    OutputChanged,
    OptionsChanged,
    /// Copy progress or stage of a running conversion.
    WriteProgress,
    /// A conversion ended (converted, cancelled or failed).
    WriteFinished,
    /// The preview should show `Workspace::preview`.
    PreviewRequested,
}

/// Collects events without duplicates, in first-seen order.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Events(Vec<AppEvent>);

impl Events {
    pub fn push(&mut self, e: AppEvent) {
        if !self.0.contains(&e) {
            self.0.push(e);
        }
    }

    pub fn extend(&mut self, other: Events) {
        for e in other.0 {
            self.push(e);
        }
    }

    #[cfg(test)]
    pub fn contains(&self, e: AppEvent) -> bool {
        self.0.contains(&e)
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = AppEvent> + '_ {
        self.0.iter().copied()
    }
}

impl<const N: usize> From<[AppEvent; N]> for Events {
    fn from(events: [AppEvent; N]) -> Self {
        let mut out = Events::default();
        for e in events {
            out.push(e);
        }
        out
    }
}
