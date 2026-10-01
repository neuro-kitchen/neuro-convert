//! Problems found while checking a session or planning an output.

/// Severity of a problem found while planning an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// The output cannot be written.
    Error,
    /// Written, but incomplete for archives such as DANDI.
    Warning,
}

/// What an issue is about, so a front end can show it where it is fixed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum Target {
    /// A metadata field, by its path in the metadata file (`session.description`,
    /// `session.timezone`, `subject.species`, …).
    Field(String),
    Stream(String),
    Event(String),
    Table(String),
    Snippet(String),
    ElectrodeGroup(String),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Issue {
    pub level: Level,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    /// Only matters for an upload to the DANDI archive (the NWB file is valid without it).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dandi: bool,
}

impl Issue {
    pub fn error(message: impl Into<String>) -> Self {
        Self { level: Level::Error, message: message.into(), target: None, dandi: false }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self { level: Level::Warning, message: message.into(), target: None, dandi: false }
    }

    pub fn at(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }

    /// Marks the issue as a DANDI requirement.
    pub fn dandi(mut self) -> Self {
        self.dandi = true;
        self
    }
}
