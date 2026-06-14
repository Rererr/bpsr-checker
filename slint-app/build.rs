use std::env;

fn main() {
    // bundled translations（ja/en）。.po は translations/<lang>/LC_MESSAGES/<crate>.po。
    let config =
        slint_build::CompilerConfiguration::new().with_bundled_translations("translations");
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint compilation failed");

    // Windows: 管理者権限要求マニフェストを埋め込む（WinDivert に必要・UAC 起動）。
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        // 開発専用の逃げ道: BPSR_SKIP_MANIFEST=1 のときは管理者要求マニフェストを埋め込まない。
        // デモ/UI 確認を UAC 昇格なしで起動するためのローカル用途で、未設定なら従来通り
        // RequireAdministrator を埋め込む（リリースビルドへの影響なし）。
        println!("cargo:rerun-if-env-changed=BPSR_SKIP_MANIFEST");
        let skip_manifest = env::var("BPSR_SKIP_MANIFEST").is_ok_and(|v| v == "1");
        if !skip_manifest {
            use embed_manifest::manifest::ExecutionLevel;
            use embed_manifest::{embed_manifest, new_manifest};
            embed_manifest(
                new_manifest("BpsrApp")
                    .requested_execution_level(ExecutionLevel::RequireAdministrator),
            )
            .expect("embed manifest failed");
        }

        // exe アイコンを埋め込む（マニフェストは embed-manifest 側で処理済）。
        embed_resource::compile("app.rc", embed_resource::NONE);
    }
}
