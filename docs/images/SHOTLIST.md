# README 用スクリーンショット

README が参照する画像一覧。全てデモモード（下記）で撮影したもので、実プレイヤーの
名前・データは一切含まない（プレイヤー名は架空、対象は訓練用ダミー）。

| ファイル名 | 内容 |
|---|---|
| `main.png` | メイン画面（DPS タブ・8人パーティ・スパークライン・自分強調） |
| `result-3min.png` | 3 分計測の結果画面（DPS 推移折れ線＋スキル円グラフ） |
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
- ウィンドウ単位の撮影は暗背景の上で行うと透明部分が締まる。Win11 では枠なし全画面の
  背景フォームがフルスクリーン昇格して topmost 窓を覆うため、背景は画面より 1px 小さく
  手動配置する（Maximized にしない）。リサイズ直後は再描画前で透明になるので
  2 秒以上待ってからキャプチャする。
