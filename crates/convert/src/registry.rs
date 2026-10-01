//! The readers available to a program, and how a path is matched to one.

use std::path::Path;

use nc_core::{Detection, Error, OpenOptions, Reader, Result, Session};

/// An ordered set of readers. [`Registry::builtin`] holds the readers compiled into this build;
/// [`Registry::with`] adds more (e.g. a reader from another crate).
pub struct Registry {
    readers: Vec<Box<dyn Reader>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Registry {
    /// No readers.
    pub fn empty() -> Self {
        Self { readers: Vec::new() }
    }

    /// Every reader enabled by this build's cargo features.
    pub fn builtin() -> Self {
        #[allow(unused_mut)]
        let mut r = Self::empty();
        #[cfg(feature = "tdt")]
        {
            r = r.with(nc_tdt::Tdt);
        }
        r
    }

    /// Adds a reader; on a tie in detection confidence the earlier reader wins.
    pub fn with(mut self, reader: impl Reader + 'static) -> Self {
        self.readers.push(Box::new(reader));
        self
    }

    pub fn readers(&self) -> impl Iterator<Item = &dyn Reader> {
        self.readers.iter().map(|r| r.as_ref())
    }

    pub fn get(&self, name: &str) -> Option<&dyn Reader> {
        self.readers().find(|r| r.name() == name)
    }

    /// Readers claiming `path`, best first.
    pub fn detect(&self, path: &Path) -> Vec<Detection> {
        let mut found: Vec<Detection> = self.readers().filter_map(|r| r.detect(path)).collect();
        found.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
        found
    }

    /// Opens `path` with the reader that claims it most confidently.
    pub fn open(&self, path: &Path, options: &OpenOptions) -> Result<Session> {
        let best = self.detect(path).into_iter().next().ok_or_else(|| Error::UnknownFormat(path.to_path_buf()))?;
        let reader = self.get(best.format).expect("detected by a registered reader");
        reader.open(path, options)
    }
}

/// [`Registry::detect`] with the built-in readers.
pub fn detect(path: &Path) -> Vec<Detection> {
    Registry::builtin().detect(path)
}

/// [`Registry::open`] with the built-in readers.
pub fn open(path: &Path, options: &OpenOptions) -> Result<Session> {
    Registry::builtin().open(path, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    impl Reader for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn description(&self) -> &'static str {
            "test reader"
        }
        fn versions(&self) -> &'static [&'static str] {
            &[]
        }
        fn detect(&self, path: &Path) -> Option<Detection> {
            (path.extension()? == "fake").then_some(Detection { format: "fake", version: None, confidence: 1.0 })
        }
        fn open(&self, _: &Path, _: &OpenOptions) -> Result<Session> {
            Ok(Session::default())
        }
    }

    #[test]
    fn test_custom_reader_plugs_in() {
        let r = Registry::empty().with(Fake);
        assert_eq!(r.detect(Path::new("x.fake"))[0].format, "fake");
        assert!(r.open(Path::new("x.fake"), &OpenOptions::default()).is_ok());
        assert!(matches!(r.open(Path::new("x.other"), &OpenOptions::default()), Err(Error::UnknownFormat(_))));
    }
}
