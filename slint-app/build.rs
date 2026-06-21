use std::env;

fn main() {
    // bundled translations（ja/en/zh）。.po は translations/<lang>/LC_MESSAGES/<crate>.po。
    // slint-build は .po への rerun-if-changed を出さないため、ここで明示しないと
    // .po を編集しても再バンドルされない（訳文が反映されない）。各 .po を監視する。
    println!("cargo:rerun-if-changed=translations");
    for lang in ["ja", "en", "zh"] {
        println!("cargo:rerun-if-changed=translations/{lang}/LC_MESSAGES/bpsr-app.po");
    }
    // 相対 "translations" だと解決基準がブレるため絶対パスで渡す。
    let translations_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("translations");
    // 既定の翻訳コンテキストはコンポーネント名（@tr は msgctxt=コンポーネント名で照合される）。
    // 我々の .po は msgctxt 無しのため None にしないと一切照合されず訳文が出ない（既存バグの真因）。
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations(translations_dir)
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
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
