use std::env;

fn main() {
    slint_build::compile("ui/app.slint").expect("Slint compilation failed");

    // Windows: 管理者権限要求マニフェストを埋め込む（WinDivert に必要・UAC 起動）。
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        use embed_manifest::manifest::ExecutionLevel;
        use embed_manifest::{embed_manifest, new_manifest};
        embed_manifest(
            new_manifest("BpsrApp").requested_execution_level(ExecutionLevel::RequireAdministrator),
        )
        .expect("embed manifest failed");
    }
}
