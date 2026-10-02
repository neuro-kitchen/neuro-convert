//! What neuro-convert takes from `settings.xml` (the GUI's signal chain): the date, the GUI
//! version, and Neuropixels probes with their site positions. Read by scanning for the few
//! elements needed rather than parsing the whole document.

/// One Neuropixels probe of the signal chain, in file order.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    /// `probe_name` (model), e.g. `Neuropixels 1.0`.
    pub model: String,
    pub serial: String,
    /// Site position (x, y in µm) per channel, by channel index.
    pub positions: Vec<Option<[f32; 2]>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// `<DATE>`: local time when the settings were saved, e.g. `30 Aug 2023 23:41:36`.
    pub date: Option<String>,
    pub version: Option<String>,
    pub probes: Vec<Probe>,
}

fn element<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find("</")? + start;
    Some(text[start..end].trim())
}

/// `name="value"` attributes of the first `<tag …>` element in `text`.
fn attributes<'a>(text: &'a str, tag: &str) -> Option<Vec<(&'a str, &'a str)>> {
    let start = text.find(&format!("<{tag} "))? + tag.len() + 2;
    let end = text[start..].find('>')? + start;
    let mut out = Vec::new();
    let mut rest = &text[start..end];
    while let Some(eq) = rest.find("=\"") {
        let name = rest[..eq].trim();
        let after = &rest[eq + 2..];
        let close = after.find('"')?;
        out.push((name, &after[..close]));
        rest = &after[close + 1..];
    }
    Some(out)
}

/// `CH0="11" CH1="59" …` → values by channel index.
fn per_channel(attrs: &[(&str, &str)]) -> Vec<Option<f32>> {
    let mut out: Vec<Option<f32>> = Vec::new();
    for (name, value) in attrs {
        let (Some(i), Ok(v)) = (name.strip_prefix("CH").and_then(|n| n.parse::<usize>().ok()), value.parse::<f32>()) else { continue };
        if out.len() <= i {
            out.resize(i + 1, None);
        }
        out[i] = Some(v);
    }
    out
}

pub fn parse(text: &str) -> Settings {
    let mut s = Settings { date: element(text, "DATE").map(str::to_string), version: element(text, "VERSION").map(str::to_string), probes: Vec::new() };
    for part in text.split("<NP_PROBE ").skip(1) {
        let part = &format!("<NP_PROBE {}", part.split("</NP_PROBE>").next().unwrap_or(part));
        let attrs = attributes(part, "NP_PROBE").unwrap_or_default();
        let get = |k: &str| attrs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()).unwrap_or_default();
        let xs = attributes(part, "ELECTRODE_XPOS").map(|a| per_channel(&a)).unwrap_or_default();
        let ys = attributes(part, "ELECTRODE_YPOS").map(|a| per_channel(&a)).unwrap_or_default();
        let positions = (0..xs.len().max(ys.len())).map(|i| Some([xs.get(i).copied().flatten()?, ys.get(i).copied().flatten()?])).collect();
        let serial = [get("probe_serial_number"), get("custom_probe_name")].into_iter().find(|v| !v.is_empty()).unwrap_or_default();
        s.probes.push(Probe { model: get("probe_name"), serial, positions });
    }
    s
}

/// `30 Aug 2023 23:41:36` → `2023-08-30T23:41:36`.
pub fn iso_date(date: &str) -> Option<String> {
    let p: Vec<&str> = date.split_whitespace().collect();
    let [d, mon, y, time] = p.as_slice() else { return None };
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let m = MONTHS.iter().position(|x| x.eq_ignore_ascii_case(mon))? + 1;
    let d: u32 = d.parse().ok()?;
    let y: u32 = y.parse().ok()?;
    (time.len() == 8).then(|| format!("{y:04}-{m:02}-{d:02}T{time}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_settings() {
        let xml = r#"<SETTINGS><INFO><VERSION>0.6.0</VERSION><DATE>30 Aug 2023 23:41:36</DATE></INFO>
          <NP_PROBE slot="5" probe_name="Neuropixels 1.0" probe_serial_number="1234">
            <ELECTRODE_XPOS CH0="11" CH1="59" CH2="27"/>
            <ELECTRODE_YPOS CH0="0" CH1="0" CH2="20"/>
          </NP_PROBE></SETTINGS>"#;
        let s = parse(xml);
        assert_eq!((s.version.as_deref(), s.date.as_deref()), (Some("0.6.0"), Some("30 Aug 2023 23:41:36")));
        assert_eq!(s.probes.len(), 1);
        assert_eq!((s.probes[0].model.as_str(), s.probes[0].serial.as_str()), ("Neuropixels 1.0", "1234"));
        assert_eq!(s.probes[0].positions, vec![Some([11.0, 0.0]), Some([59.0, 0.0]), Some([27.0, 20.0])]);
        assert_eq!(iso_date("30 Aug 2023 23:41:36").as_deref(), Some("2023-08-30T23:41:36"));
        assert_eq!(iso_date("garbage"), None);
    }
}
