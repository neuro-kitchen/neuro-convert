//! The four steps of a conversion and which step fixes which issue.

use nc_convert::core::{Issue, Level, Target};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    Source,
    Contents,
    Metadata,
    Review,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Source, Step::Contents, Step::Metadata, Step::Review];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn title(self) -> &'static str {
        match self {
            Step::Source => "Source",
            Step::Contents => "Contents",
            Step::Metadata => "Metadata",
            Step::Review => "Review & convert",
        }
    }

    pub fn next(self) -> Option<Step> {
        Self::ALL.get(self.index() + 1).copied()
    }

    pub fn previous(self) -> Option<Step> {
        self.index().checked_sub(1).map(|i| Self::ALL[i])
    }

    /// Where an issue is fixed: metadata fields in Metadata, streams and the like in Contents,
    /// the rest in Review.
    pub fn of(target: Option<&Target>) -> Step {
        match target {
            Some(Target::Field(_)) => Step::Metadata,
            Some(_) => Step::Contents,
            None => Step::Review,
        }
    }
}

/// Whether `issue` counts: DANDI-only warnings count only when the user plans a DANDI upload.
pub fn counts(issue: &Issue, dandi: bool) -> bool {
    dandi || !issue.dandi || issue.level == Level::Error
}

/// (errors, warnings) per step, in [`Step::ALL`] order.
pub fn per_step(issues: &[Issue], dandi: bool) -> [(usize, usize); 4] {
    let mut out = [(0, 0); 4];
    for i in issues.iter().filter(|i| counts(i, dandi)) {
        let slot = &mut out[Step::of(i.target.as_ref()).index()];
        if i.level == Level::Error { slot.0 += 1 } else { slot.1 += 1 }
    }
    out
}

/// A short name for what an issue is about (`Description`, `stream HDEG`).
pub fn target_label(target: &Target) -> String {
    match target {
        Target::Field(path) => field_label(path).to_string(),
        Target::Stream(n) => format!("Stream {n}"),
        Target::Event(n) => format!("Events {n}"),
        Target::Table(n) => format!("Table {n}"),
        Target::Snippet(n) => format!("Snippets {n}"),
        Target::ElectrodeGroup(n) => format!("Electrode group {n}"),
    }
}

/// The form label of a metadata field path.
pub fn field_label(path: &str) -> &str {
    match path {
        "session.description" => "Description",
        "session.start_time" => "Start time",
        "session.timezone" => "Time zone",
        "session.identifier" => "Identifier",
        "subject.id" => "Subject id",
        "subject.species" => "Species",
        "subject.age" => "Age",
        "subject.sex" => "Sex",
        "subject.strain" => "Strain",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_steps_and_counts() {
        let issues = vec![
            Issue::error("d").at(Target::Field("session.description".into())),
            Issue::warning("s").at(Target::Field("subject.species".into())).dandi(),
            Issue::error("e").at(Target::Stream("HDEG".into())),
            Issue::error("n"),
        ];
        assert_eq!(per_step(&issues, true), [(0, 0), (1, 0), (1, 1), (1, 0)]);
        assert_eq!(per_step(&issues, false), [(0, 0), (1, 0), (1, 0), (1, 0)]);
        assert_eq!(Step::Review.next(), None);
        assert_eq!(Step::Contents.previous(), Some(Step::Source));
        assert_eq!(target_label(&Target::Field("session.timezone".into())), "Time zone");
    }
}
