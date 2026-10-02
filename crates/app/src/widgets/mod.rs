//! Reusable components. Each takes plain data (no store, no view model) and renders it, so the
//! same piece looks and behaves the same wherever it is used.

pub mod form;
pub mod include;
pub mod issues;
pub mod layout;
pub mod panel;
pub mod progress;
pub mod text_table;
pub mod traces;

pub use form::{Card, FormRow, MenuSelect, SuggestInput};
pub use include::{IncludeToggle, Inclusion};
pub use issues::{IssueList, IssueRow};
pub use layout::{duration, Muted, PathRow, Section};
pub use panel::SidePanel;
pub use progress::ProgressCard;
pub use text_table::{TableData, TextTable};
pub use traces::{ProbeMap, Traces};
