# README 用スクリーンショット

README が参照する画像一覧。全てデモモード（下記）で撮影したもので、実プレイヤーの
名前・データは一切含まない（プレイヤー名は架空、対象は訓練用ダミー）。

| ファイル名 | 内容 |
|---|---|
| `main.png` | メイン画面（DPS タブ・8人パーティ・行スパークライン・自分強調。ヘッダーは観測ドット/停止/初期化/計測/コピー/設定/前面ピン/最小化） |
| `result-3min.png` | 3 分計測の結果画面（DPS 推移折れ線＋スキル円グラフ＋画像コピー） |
| `debuff-timer.png` | イマジンデバフタイマー（4人追跡・色リング） |
| `self-status.png` | 自キャラ バフ/デバフ（残時間バー・×N スタック表示） |
| `settings.png` | 設定パネル（3列レスポンシブ） |

## デモモードでの再撮影手順

実ゲーム・WinDivert・管理者権限すべて不要（`core/src/engine/demo.rs`）。

```powershell
# 一時 APPDATA（target/demo-appdata/ に settings.json / watchlist.json を用意）で
# 実環境の設定・キャッシュを汚さずに起動する
$env:APPDATA = "<repo>\target\demo-appdata"
$env:BPSR_DEMO = "1"
# 任意: $env:BPSR_DEMO_OPEN_SETTINGS = "1"  … 設定パネルを開いた状態で起動
# 任意: $env:BPSR_DEMO_3MIN = "180"          … 3分計測を自動開始（結果画面の撮影用）
cargo run -p bpsr-app
```

撮影メモ:
- スパークラインが溜まるまで 2〜3 分待つ。3分計測グラフをフル尺で出すには
  デモ用 settings.json で `"timeSeriesSamples": 200` 以上にする（既定60＝直近60秒のみ）。
- `debuff-timer.png` は auto-add が全員（デバフ無しの空行も）を追加してしまうため、
  デモ用 settings.json で `"autoAddPlayers": false`、watchlist.json の `watched` を
  デバフ保持者の `[90001, 90002, 90003, 90004]`（ソラ/カエデ/ハヤテ/ノクス）に固定する
  （demo.rs の IMAGINE_DEBUFFS がこの4 UID のみ）。`excluded` は空に。
- ウィンドウ単位の撮影は暗背景の上で行うと透明部分が締まる。背景フォームは TopMost 化＋
  撮影直前に topmost 前面へ再アサート済（`target/demo-shot.ps1`）なので、ゲーム本体
  (StarASIA) が起動中でも半透明アプリ背景にゲーム画面が透けない。背景はフルスクリーン昇格を
  避けるため画面より 1px 小さく手動配置（Maximized にしない）。リサイズ直後は再描画前で
  透明になるので 2 秒以上（SettleMs 2500 推奨）待ってからキャプチャする。
- 3分計測の結果は実時間で 180 秒経過後に表示される（`BPSR_DEMO_3MIN=180`）。撮影環境では
  Start-Sleep が実時間を消費しない場合があるため、ツール呼び出しの「間」に実時間を経過させる
  （sleep でアプリの計測時間は止まらない＝実クロックで進む）。
