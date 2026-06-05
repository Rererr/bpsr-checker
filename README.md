# bpsr-checker

**Blue Protocol: Star Resonance 向けの軽量 DPS チェッカー (Windows 専用)**

[![Latest release](https://img.shields.io/github/v/release/Rererr/bpsr-checker?display_name=tag&sort=semver)](https://github.com/Rererr/bpsr-checker/releases)
[![License](https://img.shields.io/github/license/Rererr/bpsr-checker)](./LICENSE)
[![Downloads](https://img.shields.io/github/downloads/Rererr/bpsr-checker/total)](https://github.com/Rererr/bpsr-checker/releases)
![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-blue)
[![Discord](https://img.shields.io/badge/Discord-参加する-5865F2?logo=discord&logoColor=white)](https://discord.gg/exU3gPBx3)

Tauri 2 + SolidJS + Rust で実装。低 CPU・低メモリで、ゲーム画面の上に半透明オーバーレイ表示できる軽量設計です。**外部サーバへのデータ送信は一切ありません。**

## 主な機能

- **DPS / 回復 / 履歴タブ** — 戦闘終了後に自動でエンカウントを保存し、過去ログを参照可能
- **スキル別内訳** — プレイヤーをクリックすると、そのプレイヤーのスキルごとのダメージ・命中数・クリ率を表示
- **3 分間計測モード** — 模擬戦やボス練習向けに、戦闘開始から固定時間で集計する専用モード (デフォルト 180 秒、変更可)
- **イマジン (バトルイマジン) デバフタイマー** — ティナ / アルーナ / タータ / バジリスクの免疫デバフ残時間を独立オーバーレイで表示。DPS 一覧のピンアイコンからウォッチリストに登録した複数プレイヤーを個別追跡。カラムヘッダーにカタカナ名・キャラ別識別色（ティナ赤 / アルーナ緑 / タータ紫 / バジリスク茶）付きリングで視認性を確保
- **自キャラ バフ/デバフ表示** — 自分のキャラクターに現在かかっているバフ・デバフを別ウインドウで日本語表示。残時間バー付きアイコン形式でひと目で把握可能。スタック制バフは重ね掛け数 (×N) とタイマー更新にも追従
- **デバフタイマー専用モード (軽量)** — DPS/回復の集計を停止してデバフタイマーのみ動作させる省リソースモード
- **2 列コンパクト表示** — DPS テーブルを 2 列に分割して横幅を節約するレイアウトモード
- **ヘッダースパークライン** — 全体 DPS の推移を小グラフで可視化
- **常に最前面表示・クリックスルー** — オーバーレイ運用に必須の機能を標準搭載
- **コピーテンプレート** — 集計結果を任意フォーマット (Discord 貼り付け等) でクリップボードへ
- **多言語対応** — 日本語 / English
- **キャラ指定** — 自キャラの UID を指定すると、ヘッダーに名前バッジが常時表示

## インストール

[Releases](https://github.com/Rererr/bpsr-checker/releases) から最新の `bpsr-checker_x.x.x_x64-setup.exe` をダウンロードして実行してください。

- アップデート時は起動中のアプリを終了しなくてもインストール可能です。
- 設定・履歴は再インストール後も保持されます。

### 動作要件

- Windows 10 / 11 (x64)
- 管理者権限 (WinDivert カーネルドライバのロードに必要)

## 安全性・プライバシーについて

本ツールに対するよくある懸念に回答します。

### このツールを使うと BAN されますか?

**ゲーム側ファイル・メモリ・通信内容のいずれも改変しません。** 受信パケットを受動的に観測してダメージ表示文字列を再構築しているだけで、ゲームクライアントへの注入・パッチ・自動操作は一切行いません。

ただし、本ソフトウェアは**個人開発の非公式ツール**であり、運営の規約変更により将来的に黙認されなくなる可能性は否定できません。**最終的な使用判断は利用者ご自身の責任でお願いします。** (詳細は[ライセンス](#ライセンス)末尾の免責条項を参照)

### ウイルスではないですか? ウイルス対策ソフトに検出されました

**誤検知です。** カーネルレベルでパケットをキャプチャする [WinDivert](https://github.com/basil00/WinDivert) ドライバを同梱しているため、一部のウイルス対策ソフトが「ネットワーク監視ツール」として警告を出すことがあります。

対処:
- WinDivert ドライバ (`WinDivert.dll`, `WinDivert64.sys`) およびインストールフォルダをウイルス対策ソフトの除外設定に追加してください。
- 不安な場合は[ソースコード](https://github.com/Rererr/bpsr-checker)を確認し、自分で[ビルド](#ソースからのビルド)することも可能です (GPL-3.0)。

### Windows SmartScreen で「WindowsによってPCが保護されました」と表示されます

アプリへの署名 (コードサイニング、[SignPath](https://signpath.org/) 経由) を進めていますが、新規署名はレピュテーション (実行実績) が蓄積されるまでの過渡期に SmartScreen 警告が表示されることがあります。

回避手順:
1. ダイアログの「詳細情報」をクリック
2. 表示された「実行」ボタンをクリック

### 外部にデータを送信しますか?

**送信しません。** アプリ本体に HTTP クライアントライブラリは組み込まれておらず、テレメトリ・アナリティクス・クラッシュレポートの自動送信は一切行いません。すべての処理はローカルで完結します (アップデート確認のための GitHub Releases 参照を除く)。

### 動作原理 (簡略版)

1. WinDivert を **SNIFF モード** (受動観測のみ) で起動
2. ゲームサーバ宛/から流れる TCP パケットを観測
3. ペイロードを [protobuf](https://protobuf.dev/) としてデコードし、`SyncNearDeltaInfo` 等のメッセージからダメージ・回復イベントを抽出
4. UID 単位で集計し、UI に表示

詳細は [`src-tauri/src/capture/windivert.rs`](./src-tauri/src/capture/windivert.rs) を参照してください。

## 使い方

1. アプリを起動 (UAC でゲーム同様に管理者権限を要求します)
2. ゲームを起動して戦闘を開始すると、ダメージが自動検出されます
3. プレイヤー行をクリックするとスキル別の内訳を表示
4. 戦闘終了 (デフォルト 10 秒間ダメージなし) で履歴に自動保存

### キーボードショートカット

| キー | 動作 |
| --- | --- |
| `Ctrl+Shift+Z` | クリックスルーを **無効化** (操作可能状態に戻す) |

### タスクトレイ

トレイアイコンを右クリック → メニューから終了などの操作が可能。

### 設定パネル

ヘッダーの **S** ボタンから開きます。主な項目:

- 自キャラ UID の固定 / 候補からの選択
- 透明度・フォントサイズ・列の表示切替
- コピーテンプレート (`{name} {dmg} {dps}` 等のプレースホルダ)
- 3 分計測モードの時間設定
- イマジンデバフタイマーの表示切替 / デバフタイマー専用モード (DPS 集計を停止して軽量化) / 2 列コンパクト表示の ON/OFF
- 自キャラ バフ/デバフ表示の ON/OFF
- ウォッチリストへの追加は DPS 一覧のプレイヤー行横のピンアイコンから操作
- 起動時タブ (DPS / 回復 / 履歴)

## 既知の制約

- **起動直後 / リセット直後の周囲キャラ表示について**
  本ツールはゲームクライアントが受信したパケットをパッシブに観測する方式のため、起動・リセットの時点ですでに視界内にいるキャラについて、サーバから一度しか送られない名前・職業・装備力の情報を取得できないことがあります。
  このようなキャラは「プレイヤー#XXXX」と薄く表示され、職業はスキルから自動推定されます。過去に観測したことがある UID は 30 日間の名前キャッシュから自動復元されます。ゾーン移動や再ログインで視界に再入場すると、正しい情報が取得されます。

## トラブルシューティング

| 症状 | 対処 |
| --- | --- |
| ダメージが検出されない | 管理者権限で起動しているか確認。VPN や ping reducer (ExitLag / NoPing 等) を有効にしている場合は無効化して再試行。 |
| ウイルス対策ソフトに検出される | [上記項目](#ウイルスではないですか-ウイルス対策ソフトに検出されました)を参照。 |
| 起動時に黒画面 | WebView2 ランタイムが必要です。[Microsoft 公式](https://developer.microsoft.com/microsoft-edge/webview2/) からインストール。 |
| クリックスルー中にウィンドウ操作したい | `Ctrl+Shift+Z` で解除。 |
| 過去のリリースとライセンスが違う | v0.7.8 以降は GPL-3.0、それ以前は MIT ライセンスでした。([詳細](#ライセンス)) |

不具合報告・要望は [Issues](https://github.com/Rererr/bpsr-checker/issues) または [Discord](https://discord.gg/exU3gPBx3) へお寄せください。

## ソースからのビルド

```bash
# 前提: Node.js 22+, Rust stable, Visual Studio Build Tools (Windows)

git clone https://github.com/Rererr/bpsr-checker.git
cd bpsr-checker

# WinDivert を取得 (Windows のみ)
# https://github.com/basil00/WinDivert/releases から v2.2.2 A 版を取得し、
# WinDivert.dll / WinDivert64.sys を src-tauri/ に配置

npm install
npm run tauri build
```

成果物は `src-tauri/target/release/bundle/nsis/` 配下に生成されます。

## 関連プロジェクト

同じゲーム向けに開発されている DPS メーターは他にもあります。本プロジェクトはそれらの良い点を参考にしています。

- [winjwinj/bpsr-logs](https://github.com/winjwinj/bpsr-logs) — Rust + Tauri + Svelte、Discord コミュニティが活発
- [anying1073/StarResonanceDps](https://github.com/anying1073/StarResonanceDps) — .NET + WPF、機能豊富
- [dmlgzs/StarResonanceDamageCounter](https://github.com/dmlgzs/StarResonanceDamageCounter) — 多くの派生実装の原点

## 利用にあたって (お願い)

本ツールは**プレイヤー個人の振り返り**を目的としています。以下の用途には使用しないでください。

- 他プレイヤーのスコアを晒して中傷・煽る用途
- 野良パーティでの装備強要 / 同行拒否の根拠としての利用

DPS は装備・スキル回し・状況・ロールにより大きく変動します。数値はあくまで参考値としてご活用ください。

## ライセンス

本ソフトウェアは [**GNU General Public License v3.0 only (GPL-3.0-only)**](./LICENSE) の下で配布されます。

- 改変版を配布する場合は、ソースコードを同じ GPL-3.0 ライセンスで公開する必要があります。
- 著作権表示・ライセンス全文・改変内容の明示を保持してください。

> **注**: v0.7.7 以前は MIT ライセンスで配布していましたが、v0.7.8 から GPL-3.0 に変更しました。

### 免責事項

本ソフトウェアは現状のまま提供され、**明示または黙示を問わずいかなる保証もありません**。本ソフトウェアの使用または使用不能から生じる一切の損害について、作者は責任を負いません。利用は自己責任でお願いします。

Copyright (C) 2025 Rererr
