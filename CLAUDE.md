# bpsr-checker

Blue Protocol: Star Resonance 向け軽量 DPS チェッカー。
**Slint（Rust ネイティブ GUI）** の Windows デスクトップアプリ。
（旧 Tauri v2 + SolidJS 版から移行済み。)

## 技術スタック

| レイヤー | 技術 |
|---|---|
| GUI | Slint 1.x（winit + femtovg レンダラ） |
| 言語 | Rust 2024 edition |
| パケット観測 | WinDivert（Windows 管理者権限が必要） |
| シリアライズ | Protocol Buffers（prost）|
| クリップボード | arboard（text専用） |
| 配布 | NSIS standalone インストーラ＋ポータブル zip |

## ディレクトリ構成（cargo workspace）

```
core/                  共有ロジックライブラリ（bpsr-core、Tauri/Slint 非依存）
  src/engine/          ゲームロジック（エンカウンター・バフ追跡・DPS計算）
  src/capture/         パッシブパケット観測・TCP再組み立て（WinDivert）
  src/protocol/        パケット解析・opcode テーブル・Protobuf 定義
  src/compute.rs       集計 API（&EncounterMutex を受け値を返す。emit/Tauri 非依存）
  data/json/           埋め込みデータ（SkillName.json / MonsterNameBoss.json）

slint-app/             本体アプリ（bpsr-app）
  src/main.rs          起動・capture配線・UIポーリング・全コールバック
  ui/app.slint         全 UI（MainWindow＋2オーバーレイ）
  translations/        bundled translations（ja は msgid フォールバック / en の .po）
  data/                BuffName.ja.json（オーバーレイのバフ名）
  app.rc / icon.ico    exe アイコン埋め込み

installer/installer.nsi  NSIS standalone インストーラ
scripts/package-slint.ps1 release ビルド→zip＋インストーラ生成
windivert/             WinDivert.dll / WinDivert64.sys（DL物・gitignore・配布同梱用）
slint-poc/             使い捨て検証 PoC（workspace exclude）
```

## 開発コマンド

```bash
# ワークスペース型チェック
cargo check --workspace

# 本体アプリ起動（Windows 管理者権限が必要・UAC）
cargo run -p bpsr-app

# 配布物生成（release exe→bpsr-checker.exe＋WinDivert同梱→zip、makensis があればインストーラも）
pwsh scripts/package-slint.ps1
```

## 重要な制約

### WinDivert は Windows 専用・exe 隣に必要
`core/src/capture/windivert.rs` は `#[cfg(target_os = "windows")]` でガード済み。
実行には `WinDivert.dll` / `WinDivert64.sys` が exe と同じディレクトリに必要
（dev は `target/debug/` に配置、配布は `windivert/` から同梱）。

### WinDivert は共有資源（他アプリとの共存設計）
"WinDivert" サービス／ドライバは**マシン全体で1つの共有資源**。本アプリは姉妹アプリ
`bpsr-module-optimizer` や他の WinDivert 利用ツールと**同時起動**され得るため、
ドライバの所有者ではなく**善良な利用者**として振る舞う:
- **起動時にサービスを停止・削除しない**。`open` 失敗時のみ、`recover_stale_service` が
  **STOPPED（＝誰も使っていない）と確認できた壊れた残留サービス**だけを delete して再試行する
  （RUNNING＝他アプリ使用中の可能性、には触れない）。
- ハンドルは **distinct・非0 priority** ＋ `sniff` + `recv_only` で開く。WinDivert は同一優先度・
  重複フィルタのハンドルにパケットを一度しか配送しないため。**checker=`-1000` / optimizer=`-1100`**
  （`CAPTURE_PRIORITY`）。両者で必ず別値にすること。
- インストーラ（`installer/installer.nsi`）も `sc delete WinDivert` を**実行しない**（best-effort
  `sc stop` のみ。`.sys` 削除は `/REBOOTOK`）。終了時も共有サービスを削除しない。
- **バージョン lockstep**: 同梱ドライバ版がアプリ間で食い違うと `WinDivertOpen` が
  `IncompatibleVersion (654)` で失敗する。`windivert`(0.6)/`windivert-sys`(0.10) は
  `bpsr-module-optimizer` と**同一に保つ**。バージョンを上げる場合は両アプリ同時に行う。
- **dev の `.sys` ロック**: 駆動中は `target/**/WinDivert64.sys` がロックされ次回ビルドの再コピーが
  失敗する。rebuild 前に `pwsh scripts/reset-windivert.ps1`（要管理者）で停止して解放する
  （Slint 本体に終了フックが無いため。optimizer は終了時に debug 限定 STOP で自動解放）。

### ローカルテスト（Windows 開発環境）
開発のメイン環境は Windows。`cargo run -p bpsr-app` を管理者権限で起動して実機テストする。
Slint femtovg はブラウザ（旧 WebView2）と描画特性が異なる（ClearType 非対応）。
既定フォントは可読性のため `Yu Gothic UI` を指定済み。

### MCP 経由の UI 検査・操作（Slint 埋め込み MCP サーバー）
Slint テストバックエンド同梱の MCP サーバーで、起動中の UI を Claude Code 等から検査/操作できる
（UIツリー探索・スクショ・クリック・入力）。`pwsh scripts/run-mcp.ps1` で起動（既定 8080・デモモード）。
内部的には slint-app の opt-in feature `mcp`（`set_platform` 後に `mcp_server::init()` を自前呼出）+
`SLINT_EMIT_DEBUG_INFO=1`（内省メタ埋め込み・ビルド時必須）+ `SLINT_MCP_PORT`（設定時のみ起動）。
エンドポイントは `http://127.0.0.1:<port>/mcp`（localhost 限定）。
クライアント登録は `claude mcp add --transport http -s user slint-ui http://127.0.0.1:8080/mcp`。
> デバッグ起動専用。release ビルド・`package-slint.ps1` には絶対に付与しない。

**Slint の UI/UX を調査・レビューする際は、推測やソース読みだけで済ませず、この MCP（`slint-ui`）で
実際に動作中の UI を検査すること**（要素ツリー・実寸/配置・`take_screenshot` での見た目確認、
クリック/入力での挙動確認）。femtovg はブラウザと描画特性が異なり静的読みでは判断を誤りやすいため、
実機 UI の観測を一次情報とする。MCP 接続は Claude Code 起動時に確立するので、`scripts/run-mcp.ps1` で
アプリを起動した状態でセッションを開始する（起動済みアプリに後から接続したい場合はセッション再起動）。

#### セッション再起動せずに使う（推奨・現セッションでそのまま観測する手順）
セッション途中でアプリを起動した等で `slint-ui` ツールが当セッションに無くても、**セッション再起動は不要**。
MCP サーバーは素の HTTP JSON-RPC なので Bash/PowerShell から直接叩ける（クライアント登録不要・2026-06-20 実証）。
1. `pwsh scripts/run-mcp.ps1` でアプリ起動（既定 8080・デモ）。`http://127.0.0.1:8080/mcp` を待受け。
2. `POST http://127.0.0.1:8080/mcp` に順に投げる: (1) `initialize`（ヘッダ `Accept: application/json, text/event-stream`、
   **ステートレス＝`Mcp-Session-Id` 不要**） (2) `notifications/initialized` (3) `tools/call`。
3. レスポンスが SSE のときは `data:` 行を結合して JSON 化。`take_screenshot` は base64 PNG →
   `[IO.File]::WriteAllBytes` で保存して Read で確認。
- 引数キーは `windowHandle` / `elementHandle`（`window` ではない）。`get_element_tree` は**フラットな `elements[]`**
  （nested children でない。`handle.index` / `typeNamesAndIds[].typeName` / `id` で識別）。窓ハンドルは index 0=`{}`,1,2…。
- 設定セクションは `if settings-open` ガード＝開く前はツリーに出ない（`MainWindow::settings-btn` を click で開閉）。
- **注意: `click_element` は実 handler を通り `settings::save` まで走る**＝トグルした設定が実 `settings.json` に
  永続化される。検証後は必ず元へ戻す。座標は introspection がズレるので `take_screenshot` を一次情報に
  （詳細・合成入力の限界は memory `slint-mcp-introspection-coords` 参照）。

### バージョン同期（git tag 前に必ず実施）
バージョンの正典は **`slint-app/Cargo.toml` の `version`**。タグと同じ値へ更新し、
コード変更と同一コミットで行う。配布物・梱包スクリプトもこの値を参照する。

> 背景: 旧 Tauri 版で tauri.conf.json と Cargo.toml の version 放置により全リリースが
> 0.6.0 として配布された実例がある。単一ソースに集約して再発を防ぐ。

### リリース手順（必ずセットで実施）
1. `slint-app/Cargo.toml` の version 更新 → コミット → `v*` タグ push
   （CI `release.yml` が Slint をビルドし zip＋インストーラを GitHub Releases へ）
2. GitHub Releases ページにリリースノートを日本語で記載（CHANGELOG.md ではない）
   - 形式: `## 変更内容` 見出しの下に箇条書き（追加/修正/変更を区別）
3. ユーザー向けの機能変化を伴う場合は README の「主な機能」も同タイミングで更新
> 署名(SignPath)は現状 continue-on-error で未署名公開が続いている（要修正）。

## プロトコル・実装メモ

### バフ関連（core/src/engine/buff_tracker.rs, buff_source.rs, buff_dictionary.rs）
- バフ付与/更新/解除の通知オペコード: `NotifyBuffChange = 0x3003 (12291)`
- UUID 構造: `player_uid << 16 | entity_type_code`（プレイヤー=640、モンスター=64）
  - `get_player_uid(uuid) = uuid >> 16`
- kind 分類は `buff_source.rs` の `classify(base_id)` と `classify_buff(buff_config_id)` の2系統
- バフの優先度/表示色は `buff_dictionary.rs` の `BuffMeta(category, DisplayPriority)`

### Slint 実装メモ（slint-app）
- core は専用スレッドの tokio で起動し、共有 `Arc<EncounterMutex>` を UI が `slint::Timer`
  でポーリング。全状態は main.rs の Rc/RefCell＋Slint プロパティ/VecModel。
- watchlist の `excluded` で手動削除の巻き戻りを防止。削除は `removeFromWatchlist` 相当経由。
- 新規 .slint 文字列は必ず `@tr(...)`。ja は msgid（日本語ソース）フォールバック、en は
  `translations/en/LC_MESSAGES/bpsr-app.po` に対訳を追記。
- Slint 注意: 外側プロパティから `if` 内の要素 id は参照不可（TouchArea は無条件配置＋
  `enabled` で制御）。Slint 生成 struct は `..Default::default()` 可。特殊記号は
  フォント依存（Dingbats ✕✓ は欠落環境あり→ Path 描画か Latin-1 ×/ASCII を使う）。

## コーディング規則

- Rust: `thiserror` で型付きエラー定義、`?` で伝播。`unwrap()`/`expect()` は起動時の不変条件のみ。
- 厳密な型付け。エラーは握り潰さず意味あるメッセージ付きで処理。
- コミットメッセージは Conventional Commits 形式・日本語・簡潔に（例: `fix: バフタイマーのリセット漏れを修正`）。

## 作業スタイル

- 独立した複数タスクは `isolation: "worktree"` 付きの `developer` エージェントを並列実行して効率化する。
- Slint の UI/UX 調査・`ui-ux-reviewer` でのレビュー時は、`slint-ui` MCP で動作中 UI を観測してから判断する（詳細は「MCP 経由の UI 検査・操作」節）。
