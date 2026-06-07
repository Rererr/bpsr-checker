use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Protobuf 生成は core クレートへ移管済み。ここでは行わない。

    // On Windows, embed an application manifest that requests admin
    // privileges so packet capture (WinDivert) works without the user
    // having to right-click → "Run as administrator". Triggers UAC.
    // tauri-build also embeds a default manifest, so we disable it via
    // WindowsAttributes::new_without_app_manifest() to avoid CVT1100
    // (duplicate MANIFEST resource) at link time.
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        use embed_manifest::manifest::ExecutionLevel;
        use embed_manifest::{embed_manifest, new_manifest};
        embed_manifest(
            new_manifest("BpsrChecker")
                .requested_execution_level(ExecutionLevel::RequireAdministrator),
        )?;
    }

    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attributes)?;

    Ok(())
}
