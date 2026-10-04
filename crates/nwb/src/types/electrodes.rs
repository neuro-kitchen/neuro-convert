//! `/general/extracellular_ephys`: electrode groups and the electrodes table, one row per
//! `Session::electrodes` entry in order (so a session electrode index is its table row).

use serde_json::json;

use super::{column, typed, typed_with};
use crate::backend::{Attrs, Backend};
use crate::mapping::NwbPlan;
use nc_core::{Result, Session};

/// Path of the electrodes table.
pub const TABLE_PATH: &str = "/general/extracellular_ephys/electrodes";

/// Writes the electrode groups and the electrodes table.
pub fn write(b: &dyn Backend, plan: &NwbPlan, session: &Session) -> Result<()> {
    if plan.groups.is_empty() && session.electrodes.is_empty() {
        return Ok(());
    }
    b.group("/general/extracellular_ephys", Attrs::new())?;
    for g in &plan.groups {
        let links = json!([{ "source": ".", "path": format!("/general/devices/{}", g.device), "name": "device" }]);
        let a = typed_with("core", "ElectrodeGroup", &[("description", json!(g.description)), ("location", json!(g.location)), ("_LINKS", links)]);
        b.group(&format!("/general/extracellular_ephys/{}", g.name), a)?;
    }
    let electrodes = &session.electrodes;
    if electrodes.is_empty() {
        return Ok(());
    }

    let group_of = |name: &str| plan.groups.iter().find(|g| g.name == name);
    let location: Vec<String> = electrodes
        .iter()
        .map(|e| e.location.clone().or_else(|| group_of(&e.group).map(|g| g.location.clone())).unwrap_or_else(|| "unknown".into()))
        .collect();
    let group: Vec<String> = electrodes.iter().map(|e| format!("/general/extracellular_ephys/{}", e.group)).collect();
    let group_name: Vec<String> = electrodes.iter().map(|e| e.group.clone()).collect();
    let channel_name: Vec<String> = electrodes.iter().map(|e| e.name.clone()).collect();
    let with_imp = electrodes.iter().any(|e| e.impedance_ohms.is_some());
    let with_position = electrodes.iter().any(|e| e.position_um.is_some());

    let mut colnames = vec!["location", "group", "group_name", "channel_name"];
    if with_imp {
        colnames.push("imp");
    }
    if with_position {
        colnames.extend(["rel_x", "rel_y", "rel_z"]);
    }
    let mut a = typed("core", "ElectrodesTable");
    a.insert("description".into(), json!("metadata about extracellular electrodes"));
    a.insert("colnames".into(), json!(colnames));
    b.group(TABLE_PATH, a)?;

    b.strings(&format!("{TABLE_PATH}/location"), &location, "dim0", column("Location of the electrode (channel)."))?;
    let mut group_attrs = column("Reference to the ElectrodeGroup.");
    group_attrs.insert("_DTYPE".into(), json!("object_reference"));
    b.strings(&format!("{TABLE_PATH}/group"), &group, "dim0", group_attrs)?;
    b.strings(&format!("{TABLE_PATH}/group_name"), &group_name, "dim0", column("Name of the ElectrodeGroup this electrode is a part of."))?;
    b.strings(&format!("{TABLE_PATH}/channel_name"), &channel_name, "dim0", column("Channel name in the source recording."))?;
    if with_imp {
        let imp: Vec<f32> = electrodes.iter().map(|e| e.impedance_ohms.unwrap_or(f32::NAN)).collect();
        b.f32s(&format!("{TABLE_PATH}/imp"), &imp, "dim0", column("Impedance of the channel, in ohms."))?;
    }
    if with_position {
        for (axis, name) in ["x", "y", "z"].iter().enumerate() {
            let v: Vec<f32> = electrodes.iter().map(|e| e.position_um.map_or(f32::NAN, |p| p[axis])).collect();
            let desc = format!("{name} coordinate in electrode group, in micrometers.");
            b.f32s(&format!("{TABLE_PATH}/rel_{name}"), &v, "dim0", column(&desc))?;
        }
    }
    let ids: Vec<i64> = (0..electrodes.len() as i64).collect();
    b.i64s(&format!("{TABLE_PATH}/id"), &ids, "num_rows", typed("hdmf-common", "ElementIdentifiers"))
}
