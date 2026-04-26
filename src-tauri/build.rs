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

    tauri_build::build();

    Ok(())
}
