# bpsr-checker

Blue Protocol: Star Resonance 向け軽量 DPS チェッカー。
Tauri v2（Rust バックエンド + SolidJS フロントエンド）の Windows デスクトップアプリ。

## 技術スタック

| レイヤー | 技術 |
|---|---|
| フロントエンド | SolidJS + TypeScript、Vite |
| バックエンド | Rust 2024 edition、Tauri v2 |
| パケット観測 | WinDivert（Windows 管理者権限が必要） |
| シリアライズ | Protocol Buffers（prost）|
| Tauri 型生成 | specta + tauri-specta |

## ディレクトリ構成

```
src/
  components/          UI コンポーネント（ヘッダー・テーブル・設定パネル等）
  stores/              SolidJS ストア（encounter, buffs, settings, watchlist）
  buffs/               バトルイマジン デバフタイマーウィンドウ
  self_status/         自キャラステータス・バフ表示ウィンドウ
  lib/i18n/            多言語対応（ja/en）

src-tauri/src/
  capture/             パッシブパケット観測・TCP 再組み立て（WinDivert）
  protocol/            パケット解析・opcode テーブル・Protobuf 定義
  engine/              ゲームロジック（エンカウンター管理、バフ追跡、DPS 計算）
  bridge/              Tauri コマンド定義（specta で TypeScript 型を自動生成）
```

## 開発コマンド

```bash
# TypeScript 型チェック
npx tsc --noEmit

# Rust チェック（macOS では --lib が必要。WinDivert は Windows 専用）
cargo check --manifest-path src-tauri/Cargo.toml

# フロントエンドビルド
npm run build

# Tauri 開発サーバー起動（Windows 管理者権限が必要）
npm run tauri dev
```

## 重要な制約

### WinDivert は Windows 専用
`capture/windivert.rs` は `#[cfg(target_os = "windows")]` でガード済み。macOS での `cargo check` は `--lib` を付けるか環境変数でスキップする。

### ローカルテスト（Windows 開発環境）
開発のメイン環境は Windows。リリース前にローカルの開発ビルドで実機テストを行う。WinDivert ドライバが使用可能な環境であれば `npm run tauri dev` でも動作確認できる。

### バージョン同期（git tag 前に必ず実施）
`src-tauri/tauri.conf.json` と `src-tauri/Cargo.toml` の `version` フィールドをタグと同じ値に更新する。パッチバンプでも漏らさない。バージョン更新とコード変更は同一コミット内で行う。

> 背景: v0.6.1〜v0.6.8 まで設定ファイルを放置した結果、全リリースのインストーラーが 0.6.0 として配布された実例がある。

### リリース時の手順（必ずセットで実施）
1. `tauri.conf.json` / `Cargo.toml` のバージョン更新 → コミット → タグ
2. GitHub Releases ページにリリースノートを日本語で記載（CHANGELOG.md ではない）
   - `gh release create <tag> --notes "..."` または `gh release edit <tag> --notes "..."`
   - 形式: `## 変更内容` 見出しの下に箇条書き（追加/修正/変更を区別）
3. ユーザー向けの機能変化を伴う場合は README の「主な機能」「設定パネル」も同タイミングで更新

## プロトコル・実装メモ

### バフ関連（engine/buff_tracker.rs, engine/buff_source.rs）
- バフ付与/更新/解除の通知オペコード: `NotifyBuffChange = 0x3003 (12291)`
- UUID 構造: `player_uid << 16 | entity_type_code`（プレイヤー=640、モンスター=64）
  - `get_player_uid(uuid) = uuid >> 16`
- kind 分類は `buff_source.rs` の `classify(base_id)` と `classify_buff(buff_config_id)` の2系統
- watchlist の `excluded` リストで手動削除の巻き戻りを防止している。手動削除は必ず `removeFromWatchlist` 経由で行うこと

## コーディング規則

- Rust: `thiserror` で型付きエラー定義、`?` でエラー伝播。`unwrap()`/`expect()` は起動時の不変条件のみ許容。
- TypeScript: `strict: true`、型アサーション（`as`）は最小限。
- フロントエンド状態は `src/stores/` の SolidJS ストアで一元管理。
- コミットメッセージは Conventional Commits 形式・日本語・簡潔に（例: `fix: バフタイマーのリセット漏れを修正`）。

## 作業スタイル

- 独立した複数タスクは `isolation: "worktree"` 付きの `developer` エージェントを並列実行して効率化する。
