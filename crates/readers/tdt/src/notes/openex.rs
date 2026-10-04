//! OpenEx note file (`.tnt`): `NOTEFILE_VERSION[x.y]` then one note per line.

/// A parsed `.tnt` file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TntNotes {
    /// `NOTEFILE_VERSION`.
    pub version: Option<String>,
    /// One entry per note line.
    pub notes: Vec<String>,
}

/// Parses `.tnt` text.
pub fn parse_tnt(text: &str) -> TntNotes {
    let mut out = TntNotes::default();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(v) = line.strip_prefix("NOTEFILE_VERSION[").and_then(|v| v.strip_suffix(']')) {
            out.version = Some(v.to_string());
        } else {
            out.notes.push(line.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_and_notes() {
        let t = parse_tnt("NOTEFILE_VERSION[1.0]\r\n\r\n");
        assert_eq!(t.version.as_deref(), Some("1.0"));
        assert!(t.notes.is_empty());
    }
}
