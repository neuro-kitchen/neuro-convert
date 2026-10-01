pub fn run() -> anyhow::Result<()> {
    println!("Input formats:");
    for f in nc_convert::Registry::builtin().readers() {
        println!("  {:10} {}", f.name(), f.description());
        for v in f.versions() {
            println!("  {:10}   - {v}", "");
        }
    }
    println!("\nOutput formats:");
    println!("  nwb        Neurodata Without Borders {} (Zarr v3 store, hdmf-zarr layout)", nc_convert::nwb::schema::NWB_VERSION);
    Ok(())
}
