pub fn run() -> anyhow::Result<()> {
    println!("Input formats:");
    for f in nc_convert::Registry::builtin().readers() {
        println!("  {:10} {} (reader {}, {})", f.name(), f.description(), f.version(), f.maturity().label());
        for v in f.versions() {
            println!("  {:10}   - {v}", "");
        }
    }
    println!("\nOutput formats:");
    println!("  nwb        Neurodata Without Borders {} (Zarr v3 store, hdmf-zarr layout)", nc_convert::nwb::schema::NWB_VERSION);
    if nc_convert::nwb::HDF5 {
        println!("  nwb        Neurodata Without Borders {} (HDF5 file, `-o name.nwb`)", nc_convert::nwb::schema::NWB_VERSION);
    }
    println!("\nVersions (recorded in every conversion report and NWB `source_script`):");
    println!("  neuro-convert {}", env!("CARGO_PKG_VERSION"));
    for v in nc_convert::versions() {
        println!("  {v}");
    }
    Ok(())
}
