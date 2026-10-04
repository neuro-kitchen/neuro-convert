//! The crates compiled into this build and their versions: printed by front ends and recorded in
//! every conversion report, so a problem in a file can be traced to the exact code.

use serde::{Deserialize, Serialize};

/// A crate (or program) and its version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateVersion {
    /// Crate or program name (`nc-tdt`, `neuro-convert`).
    pub name: String,
    /// Its version.
    pub version: String,
}

impl CrateVersion {
    /// `name` at `version`.
    pub fn new(name: &str, version: &str) -> Self {
        Self { name: name.into(), version: version.into() }
    }
}

impl std::fmt::Display for CrateVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.name, self.version)
    }
}

/// Every neuro-convert crate in this build: the API, model and base crates, the NWB writer (with
/// its HDF5 support when enabled) and each enabled reader.
pub fn versions() -> Vec<CrateVersion> {
    let mut v = vec![CrateVersion::new("nc-convert", crate::VERSION), CrateVersion::new("nc-core", nc_core::VERSION), CrateVersion::new("nc-base", nc_base::VERSION)];
    #[cfg(feature = "nwb")]
    v.push(CrateVersion::new(if nc_nwb::HDF5 { "nc-nwb (zarr, hdf5)" } else { "nc-nwb (zarr)" }, nc_nwb::VERSION));
    #[cfg(feature = "tdt")]
    v.push(CrateVersion::new("nc-tdt", nc_tdt::VERSION));
    #[cfg(feature = "spikeglx")]
    v.push(CrateVersion::new("nc-spikeglx", nc_spikeglx::VERSION));
    #[cfg(feature = "intan")]
    v.push(CrateVersion::new("nc-intan", nc_intan::VERSION));
    #[cfg(feature = "openephys")]
    v.push(CrateVersion::new("nc-openephys", nc_openephys::VERSION));
    #[cfg(feature = "blackrock")]
    v.push(CrateVersion::new("nc-blackrock", nc_blackrock::VERSION));
    #[cfg(feature = "neuralynx")]
    v.push(CrateVersion::new("nc-neuralynx", nc_neuralynx::VERSION));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_versions_list_enabled_crates() {
        let v = versions();
        assert_eq!(v[0], CrateVersion::new("nc-convert", crate::VERSION));
        for name in ["nc-core", "nc-base"] {
            assert!(v.iter().any(|c| c.name == name), "{name}");
        }
        #[cfg(feature = "blackrock")]
        assert!(v.iter().any(|c| c.name == "nc-blackrock"));
        assert!(v.iter().all(|c| !c.version.is_empty()));
    }
}
