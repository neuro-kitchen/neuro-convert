//! Display helpers and value lists: paths, SI units, axis ticks, ISO ages, time zones and
//! suggestion lists. Pure and tested.

use std::path::Path;

/// `path` with the home folder as `~` and the middle elided to fit `max` characters.
pub fn short_path(path: &Path, home: Option<&Path>, max: usize) -> String {
    let full = match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    };
    let chars: Vec<char> = full.chars().collect();
    if chars.len() <= max || max < 8 {
        return full;
    }
    // Keep the end (the name) whole when possible
    let tail = (max * 2 / 3).min(chars.len());
    let head = max - tail - 1;
    format!("{}…{}", chars[..head].iter().collect::<String>(), chars[chars.len() - tail..].iter().collect::<String>())
}

/// An issue message for people: metadata-file paths (`session.timezone`,
/// `streams.MonA.conversion`) become the names the app shows next to the fields.
pub fn plain_issue(message: &str) -> String {
    const NAMES: [(&str, &str); 14] = [
        ("session.description", "the description"),
        ("session.timezone", "the time zone"),
        ("session.start_time", "the start time"),
        ("session.identifier", "the identifier"),
        ("session.experimenters", "the experimenters"),
        ("session.lab", "the lab"),
        ("session.institution", "the institution"),
        ("session.keywords", "the keywords"),
        ("subject.species", "the species"),
        ("subject.age", "the age"),
        ("subject.sex", "the sex"),
        ("subject.id", "the subject id"),
        ("subject.strain", "the strain"),
        ("subject.description", "the subject description"),
    ];
    let mut out = message.to_string();
    for (path, name) in NAMES {
        out = out.replace(path, name);
    }
    // streams.<name>.conversion / .unit
    for (suffix, name) in [(".conversion", "scale factor"), (".unit", "unit")] {
        while let Some(start) = out.find("streams.") {
            let rest = &out[start + 8..];
            let Some(end) = rest.find(suffix) else { break };
            let stream = rest[..end].to_string();
            out.replace_range(start..start + 8 + end + suffix.len(), &format!("the {name} of {stream}"));
        }
    }
    let mut chars = out.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().collect::<String>() + chars.as_str())
}

pub fn home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(Into::into)
}

/// `value` in `unit` with an SI prefix: `0.0001 V` → `100 µV`. Units that are not SI base
/// units (`a.u.`, `°C`) are left as they are.
pub fn si(value: f64, unit: &str) -> String {
    let prefixable = matches!(unit, "V" | "A" | "s" | "Hz" | "Ω" | "m");
    if !prefixable || value == 0.0 || !value.is_finite() {
        return format!("{} {unit}", trim(value, 3));
    }
    const PREFIXES: [(f64, &str); 7] = [(1e9, "G"), (1e6, "M"), (1e3, "k"), (1.0, ""), (1e-3, "m"), (1e-6, "µ"), (1e-9, "n")];
    let a = value.abs();
    let (scale, p) = PREFIXES.iter().find(|(s, _)| a >= *s * 0.999_999).copied().unwrap_or((1e-9, "n"));
    format!("{} {p}{unit}", trim(value / scale, 3))
}

/// At most `digits` significant digits, without trailing zeros.
fn trim(v: f64, digits: usize) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let decimals = (digits as i32 - 1 - v.abs().log10().floor() as i32).max(0) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// The largest 1-2-5 step that gives at least `min_count` ticks across `span`.
pub fn nice_step(span: f64, min_count: usize) -> f64 {
    if span.is_nan() || span <= 0.0 {
        return 1.0;
    }
    let raw = span / min_count.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * mag).rev().find(|s| *s <= raw).unwrap_or(mag)
}

/// Tick positions in `[start, end]` at a nice step.
pub fn ticks(start: f64, end: f64, min_count: usize) -> Vec<f64> {
    let step = nice_step(end - start, min_count);
    let first = (start / step).ceil() as i64;
    let last = (end / step).floor() as i64;
    (first..=last).map(|i| i as f64 * step).collect()
}

/// The largest 1-2-5 value not above `limit` (scale bars).
pub fn nice_below(limit: f64) -> f64 {
    if limit.is_nan() || limit <= 0.0 {
        return 0.0;
    }
    let mag = 10f64.powf(limit.log10().floor());
    [5.0, 2.0, 1.0].iter().map(|m| m * mag).find(|v| *v <= limit).unwrap_or(mag)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeUnit {
    Days,
    Weeks,
    Months,
    Years,
}

impl AgeUnit {
    pub const ALL: [AgeUnit; 4] = [AgeUnit::Days, AgeUnit::Weeks, AgeUnit::Months, AgeUnit::Years];

    pub fn label(self) -> &'static str {
        match self {
            AgeUnit::Days => "days",
            AgeUnit::Weeks => "weeks",
            AgeUnit::Months => "months",
            AgeUnit::Years => "years",
        }
    }

    fn letter(self) -> char {
        match self {
            AgeUnit::Days => 'D',
            AgeUnit::Weeks => 'W',
            AgeUnit::Months => 'M',
            AgeUnit::Years => 'Y',
        }
    }
}

/// `P90D` ↔ (90, days). `None` for anything else (kept as text by the form).
pub fn parse_age(iso: &str) -> Option<(u32, AgeUnit)> {
    let rest = iso.strip_prefix('P')?;
    let unit = AgeUnit::ALL.into_iter().find(|u| rest.ends_with(u.letter()))?;
    let n = rest[..rest.len() - 1].parse().ok()?;
    Some((n, unit))
}

pub fn format_age(n: u32, unit: AgeUnit) -> String {
    format!("P{n}{}", unit.letter())
}

/// UTC offsets with example places (standard time; daylight saving shifts by one hour).
pub const TIME_ZONES: [(&str, &str); 38] = [
    ("-12:00", "Baker Island"),
    ("-11:00", "American Samoa"),
    ("-10:00", "Hawaii"),
    ("-09:30", "Marquesas"),
    ("-09:00", "Alaska"),
    ("-08:00", "Los Angeles, Vancouver"),
    ("-07:00", "Denver, Phoenix; Los Angeles in summer"),
    ("-06:00", "Chicago, Mexico City"),
    ("-05:00", "New York, Toronto, Bogotá, Lima; Chicago in summer"),
    ("-04:00", "Santiago, Caracas; New York in summer"),
    ("-03:30", "Newfoundland"),
    ("-03:00", "São Paulo, Buenos Aires"),
    ("-02:00", "South Georgia"),
    ("-01:00", "Azores"),
    ("Z", "UTC, London, Lisbon, Reykjavík"),
    ("+01:00", "Berlin, Paris, Madrid, Lagos; London in summer"),
    ("+02:00", "Athens, Cairo, Johannesburg; Berlin in summer"),
    ("+03:00", "Moscow, Istanbul, Nairobi"),
    ("+03:30", "Tehran"),
    ("+04:00", "Dubai"),
    ("+04:30", "Kabul"),
    ("+05:00", "Karachi, Tashkent"),
    ("+05:30", "India"),
    ("+05:45", "Nepal"),
    ("+06:00", "Dhaka"),
    ("+06:30", "Yangon"),
    ("+07:00", "Bangkok, Jakarta"),
    ("+08:00", "Beijing, Singapore, Perth"),
    ("+08:45", "Eucla"),
    ("+09:00", "Tokyo, Seoul"),
    ("+09:30", "Adelaide, Darwin"),
    ("+10:00", "Sydney, Brisbane"),
    ("+10:30", "Lord Howe"),
    ("+11:00", "Solomon Islands; Sydney in summer"),
    ("+12:00", "Auckland, Fiji"),
    ("+12:45", "Chatham Islands"),
    ("+13:00", "Tonga; Auckland in summer"),
    ("+14:00", "Line Islands"),
];

/// `-05:00 · New York, …` for a zone list entry.
pub fn zone_label(offset: &str, places: &str) -> String {
    let shown = if offset == "Z" { "±00:00" } else { offset };
    format!("UTC{shown} · {places}")
}

/// Common species (Latin binomial, common name), as DANDI expects them.
pub const SPECIES: [(&str, &str); 10] = [
    ("Rattus norvegicus", "rat"),
    ("Mus musculus", "mouse"),
    ("Macaca mulatta", "rhesus macaque"),
    ("Macaca fascicularis", "crab-eating macaque"),
    ("Callithrix jacchus", "marmoset"),
    ("Homo sapiens", "human"),
    ("Felis catus", "cat"),
    ("Sus scrofa domesticus", "pig"),
    ("Danio rerio", "zebrafish"),
    ("Drosophila melanogaster", "fruit fly"),
];

/// Common strains of `species`.
pub fn strains(species: &str) -> &'static [&'static str] {
    match species {
        "Rattus norvegicus" => &["Sprague Dawley", "Long Evans", "Wistar", "Fischer 344", "Lister Hooded"],
        "Mus musculus" => &["C57BL/6J", "C57BL/6N", "BALB/c", "129S", "CD-1", "FVB/N"],
        _ => &[],
    }
}

/// Recording sites often used as electrode locations.
pub const LOCATIONS: [&str; 16] = [
    "M1", "M2", "S1", "V1", "PFC", "mPFC", "ACC", "CA1", "CA3", "DG", "striatum", "thalamus", "STN", "cerebellum", "spinal cord", "muscle",
];

pub const UNITS: [&str; 6] = ["V", "mV", "µV", "A", "°C", "a.u."];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_path() {
        let home = Path::new("/home/me");
        assert_eq!(short_path(Path::new("/home/me/data/x"), Some(home), 40), "~/data/x");
        let s = short_path(Path::new("/home/me/dev/rustProjects/neuro-kitchen/data/raw/15-25-33_meps"), Some(home), 30);
        assert_eq!(s.chars().count(), 30);
        assert!(s.starts_with("~/dev") && s.ends_with("15-25-33_meps") && s.contains('…'), "{s}");
    }

    #[test]
    fn test_si_and_ticks() {
        assert_eq!(si(0.0001, "V"), "100 µV");
        assert_eq!(si(0.0025, "V"), "2.5 mV");
        assert_eq!(si(1500.0, "Hz"), "1.5 kHz");
        assert_eq!(si(12.5, "a.u."), "12.5 a.u.");
        assert_eq!(nice_step(1.0, 4), 0.2);
        assert_eq!(ticks(0.3, 1.3, 4), vec![0.4, 0.6000000000000001, 0.8, 1.0, 1.2000000000000002]);
        assert_eq!(nice_below(0.00037), 0.0002);
        assert_eq!(nice_below(7.0), 5.0);
    }

    #[test]
    fn test_age() {
        assert_eq!(parse_age("P90D"), Some((90, AgeUnit::Days)));
        assert_eq!(parse_age("P3M"), Some((3, AgeUnit::Months)));
        assert_eq!(parse_age("P1Y2M"), None);
        assert_eq!(format_age(12, AgeUnit::Weeks), "P12W");
    }

    #[test]
    fn test_plain_issue() {
        assert_eq!(super::plain_issue("session.description is required (a sentence describing the session)"), "The description is required (a sentence describing the session)");
        assert_eq!(
            super::plain_issue("the recorded start time 2025-02-26T15:25:56 has no time zone: set session.timezone (e.g. -05:00) or session.start_time"),
            "The recorded start time 2025-02-26T15:25:56 has no time zone: set the time zone (e.g. -05:00) or the start time"
        );
        assert_eq!(super::plain_issue("Set streams.MonA.conversion (and unit)"), "Set the scale factor of MonA (and unit)");
    }
}
