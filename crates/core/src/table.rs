//! Small text tables carried with a session.

use serde::{Deserialize, Serialize};

/// A small named table carried with a session (e.g. electrode impedance measurements).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Table {
    /// Name inside its session.
    pub name: String,
    /// What the table holds.
    pub description: String,
    /// Column names.
    pub columns: Vec<String>,
    /// Row-major cells as text; outputs parse the columns they understand.
    pub rows: Vec<Vec<String>>,
}

impl Table {
    /// Index of the column named `name`.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c == name)
    }
}
