//! Pin the selected semantic search manifest into the build.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=OINO_ZVEC_MANIFEST");
    println!("cargo:rerun-if-changed=src/zvec/zvec-manifest.json");
    let source = std::env::var_os("OINO_ZVEC_MANIFEST")
        .map_or_else(|| "src/zvec/zvec-manifest.json".into(), std::path::PathBuf::from);
    println!("cargo:rerun-if-changed={}", source.display());
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    std::fs::copy(source, out.join("zvec-manifest.json"))?;
    Ok(())
}
