use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);

    // Generate protobuf code
    prost_build::Config::new()
        .type_attribute(".pb", "#[derive(specta::Type)]")
        .out_dir(manifest_dir.join("src/protocol"))
        .compile_protos(&["src/protocol/pb.proto"], &["src/protocol/"])?;

    println!("cargo:rerun-if-changed=src/protocol/pb.proto");

    // On Windows, embed an application manifest that requests admin
    // privileges so packet capture (WinDivert) works without the user
    // having to right-click → "Run as administrator". Triggers UAC.
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        use embed_manifest::manifest::ExecutionLevel;
        use embed_manifest::{embed_manifest, new_manifest};
        embed_manifest(
            new_manifest("BpsrChecker")
                .requested_execution_level(ExecutionLevel::RequireAdministrator),
        )?;
    }

    tauri_build::build();

    Ok(())
}
