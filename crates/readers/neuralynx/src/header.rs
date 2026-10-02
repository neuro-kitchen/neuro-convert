//! The 16 KiB text header of every Neuralynx file: `## …` comment lines (file name, times opened
//! and closed) and `-Key value` properties (sample rate, AD bit volts per channel, channel names
//! and ids, input range and inversion, DSP filters, waveform length, …).

use std::collections::BTreeMap;

pub const SIZE: usize = 16 * 1024;

/// How the recording system timed its records (neo's `AcqType`): decides gap tolerance and how
/// the true sample rate is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquisition {
    /// Cheetah before 4 (`NLX_Base_Class_Type CscAcqEnt`): whole microseconds per sample.
    Pre4,
    /// BML (`BmlAcq`) and Atlas: the stated rate, no gap tolerance.
    Stated,
    /// Digital Lynx (SX), Cheetah 64 / 5.6, raw data files: the rate measured from the record
    /// timestamps; gaps tolerated up to 0.2 sample periods.
    Measured,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Header {
    pub props: BTreeMap<String, String>,
    /// `## Time Opened …` / `-TimeCreated`, as an ISO-8601 local date-time.
    pub opened: Option<String>,
}

fn words(s: &str) -> Vec<&str> {
    s.split_whitespace().collect()
}

/// `m/d/y` + `h:m:s.ms` (`## Time Opened (m/d/y): 11/28/2016  (h:m:s.ms) 21:50:33.322`) or
/// `y/m/d h:m:s` (`-TimeCreated 2019/07/12 13:21:32`) → `YYYY-MM-DDTHH:MM:SS[.mmm]`.
fn iso(date: &str, time: &str, year_first: bool) -> Option<String> {
    let d: Vec<u32> = date.split(['/', '-']).map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    let (y, m, day) = if year_first { (*d.first()?, *d.get(1)?, *d.get(2)?) } else { (*d.get(2)?, *d.first()?, *d.get(1)?) };
    let (hms, ms) = time.split_once('.').map_or((time, None), |(a, b)| (a, Some(b)));
    let t: Vec<u32> = hms.split(':').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    let ms = ms.and_then(|m| m.trim().parse::<u32>().ok());
    Some(format!("{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}{}", t.first()?, t.get(1).unwrap_or(&0), t.get(2).unwrap_or(&0), ms.map_or_else(String::new, |m| format!(".{m:03}"))))
}

impl Header {
    pub fn parse(bytes: &[u8]) -> Self {
        let raw = &bytes[..bytes.len().min(SIZE)];
        // Latin-1 (the µ of `DspFilterDelay_µs`)
        let text: String = raw.iter().filter(|&&b| b != 0).map(|&b| b as char).collect();
        let mut props = BTreeMap::new();
        let mut opened = None;
        for line in text.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix('-') {
                let (k, v) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                props.entry(k.to_string()).or_insert_with(|| v.trim().to_string());
            } else if line.starts_with("## Time Opened") || line.starts_with("## Date Opened") {
                // `(m/d/y): 11/28/2016  (h:m:s.ms) 21:50:33.322` or `… At Time: 21:50:33`
                let w = words(line);
                let date = w.iter().position(|x| x.ends_with("):")).and_then(|i| w.get(i + 1));
                let time = w.last();
                if let (Some(d), Some(t)) = (date, time) {
                    opened = iso(d, t, false);
                }
            }
        }
        if opened.is_none()
            && let Some(v) = props.get("TimeCreated")
        {
            let w = words(v);
            if w.len() >= 2 {
                opened = iso(w[0], w[1], true);
            }
        }
        Self { props, opened }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.props.get(key).map(String::as_str)
    }

    pub fn sample_rate(&self) -> Option<f64> {
        self.get("SamplingFrequency")?.parse().ok()
    }

    /// Per-channel values of a property (`-ADBitVolts a b c d`; one value applies to all).
    pub fn per_channel(&self, key: &str, n: usize) -> Vec<String> {
        let w: Vec<String> = self.get(key).map(|v| words(v).into_iter().map(str::to_string).collect()).unwrap_or_default();
        match w.len() {
            0 => Vec::new(),
            1 => vec![w[0].clone(); n],
            _ => w,
        }
    }

    pub fn channel_ids(&self) -> Vec<i64> {
        self.get("ADChannel").map(|v| words(v).iter().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default()
    }

    pub fn name(&self) -> Option<&str> {
        self.get("AcqEntName").filter(|n| !n.is_empty())
    }

    pub fn inverted(&self) -> bool {
        self.get("InputInverted") == Some("True")
    }

    /// Volts per bit of each of `n` channels (sign flipped for inverted inputs, as neo).
    pub fn volts_per_bit(&self, n: usize) -> Vec<Option<f64>> {
        let sign = if self.inverted() { -1.0 } else { 1.0 };
        let v = self.per_channel("ADBitVolts", n);
        (0..n).map(|i| v.get(i).and_then(|x| x.parse::<f64>().ok()).map(|g| g * sign)).collect()
    }

    pub fn acquisition(&self) -> Acquisition {
        if let Some(t) = self.get("NLX_Base_Class_Type") {
            return if t == "CscAcqEnt" { Acquisition::Pre4 } else { Acquisition::Stated };
        }
        if self.get("HardwareSubSystemType").is_some() {
            return Acquisition::Measured;
        }
        if self.get("FileType").is_some() && matches!(self.get("FileVersion"), Some("3.2" | "3.3" | "3.4"))
            && self.get("AcquisitionSystem").is_some_and(|a| a.to_lowercase().contains("atlas"))
        {
            return Acquisition::Stated;
        }
        Acquisition::Measured
    }

    /// `Cheetah 6.3.2`, `Pegasus 2.1.1`, `BML`, … for the provenance.
    pub fn application(&self) -> String {
        if let Some(r) = self.get("CheetahRev") {
            return format!("Cheetah {}", r.trim());
        }
        if let Some(a) = self.get("ApplicationName") {
            return a.replace('"', "").split_whitespace().collect::<Vec<_>>().join(" ");
        }
        if self.get("NLX_Base_Class_Type").is_some() {
            return "BML".into();
        }
        "Neuraview".into()
    }

    /// Stream key of a CSC file (neo): rate, input range and DSP filter settings.
    pub fn stream_key(&self) -> String {
        const FILTERS: [&str; 9] = [
            "DSPLowCutFilterEnabled",
            "DspLowCutFrequency",
            "DspLowCutFilterType",
            "DspLowCutNumTaps",
            "DSPHighCutFilterEnabled",
            "DspHighCutFrequency",
            "DspHighCutFilterType",
            "DspHighCutNumTaps",
            "DspDelayCompensation",
        ];
        let mut key = format!("{}|{}", self.get("SamplingFrequency").unwrap_or(""), self.get("InputRange").unwrap_or(""));
        for f in FILTERS {
            key.push('|');
            key.push_str(self.get(f).unwrap_or(""));
        }
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header() {
        let text = "######## Neuralynx Data File Header\n## Time Opened (m/d/y): 11/28/2016  (h:m:s.ms) 21:50:33.322\n\
-FileType CSC\n-HardwareSubSystemType DigitalLynxSX\n-SamplingFrequency 2000\n-ADBitVolts 0.000000030518510385491027\n\
-AcqEntName CSC1\n-ADChannel 58\n-InputInverted True\n-CheetahRev 5.6.3 \n";
        let h = Header::parse(text.as_bytes());
        assert_eq!((h.sample_rate(), h.name(), h.channel_ids()), (Some(2000.0), Some("CSC1"), vec![58]));
        assert_eq!(h.volts_per_bit(1), vec![Some(-0.000000030518510385491027)]);
        assert_eq!(h.opened.as_deref(), Some("2016-11-28T21:50:33.322"));
        assert_eq!((h.acquisition(), h.application()), (Acquisition::Measured, "Cheetah 5.6.3".to_string()));
        let h = Header::parse(b"-TimeCreated 2019/07/12 13:21:32\n-ApplicationName Cheetah \"6.3.2 \"\n-NLX_Base_Class_Type CscAcqEnt\n");
        assert_eq!((h.opened.as_deref(), h.application().as_str(), h.acquisition()), (Some("2019-07-12T13:21:32"), "Cheetah 6.3.2", Acquisition::Pre4));
    }
}
