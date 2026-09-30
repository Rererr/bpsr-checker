# ドキュメント用スクリーンショット

README と機能ドキュメントで使う実機 UI の撮影台帳です。すべて隔離したデモ用 `APPDATA` と合成データで撮影しており、実プレイヤーの情報は含みません。通常利用では使わない「軽量分割表示」は、その機能ページの2枚だけに限定しています。

| ファイル名 | 内容 |
|---|---|
| `feature-main-overview.png` | 通常表示の DPS 一覧。8人、職業アイコン、イマジン名、食事/シロップ、推移、行バー、ヘッダー、フッター |
| `feature-skill-breakdown.png` | 通常一覧から開いたプレイヤーのスキル別ダメージ・DPS・構成比 |
| `result-measurement.png` | 計測結果。順位、推移グラフ、スキル一覧、円グラフ、コピー操作、v1.25.0 |
| `feature-history.png` | 実際に完了した計測結果が1件入った履歴タブ |
| `feature-history-expanded.png` | 履歴のエンカウントを展開した状態。日時とコンテンツ名の見出し、順位付きプレイヤー行 |
| `feature-total-row.png` | 通常一覧の合計行（コンテンツ名・総ダメージ・合計DPS・経過） |
| `feature-effective-dps.png` | 有効 DPS 列を表示した通常一覧 |
| `feature-effective-dps-settings.png` | 有効 DPS 列を ON にした設定 |
| `feature-food-syrup.png` | 食事/シロップの残時間アイコンを表示した通常一覧 |
| `feature-compact-layout.png` | 軽量分割表示を ON にした2列一覧 |
| `feature-compact-layout-settings.png` | 軽量分割表示を ON にした設定 |
| `feature-imagine-debuff-timer.png` | イマジンデバフタイマーの独立オーバーレイ |
| `feature-imagine-debuff-timer-settings.png` | タイマーを ON にした設定 |
| `feature-imagine-only.png` | 通常レイアウト上のイマジン専用モード案内 |
| `feature-imagine-only-settings.png` | イマジン専用モードを ON にした設定 |
| `feature-self-buffs-debuffs.png` | 自キャラのバフ/デバフ名、残時間バー、スタック数 |
| `feature-self-status.png` | 自キャラステータスの独立オーバーレイ |
| `feature-self-status-settings.png` | 自キャラステータスと項目グループの設定 |
| `feature-settings-overview.png` | 設定上部。キャラ、列、戦闘、表示、タイマー、ステータス |
| `feature-settings-bottom.png` | オーバーレイ外観とテンプレート |
| `feature-settings-actions.png` | テンプレート、イマジン名編集、ショートカット設定、更新、バージョン |
| `feature-update.png` | バージョン選択を開いた更新セクション |
| `feature-update-notice.png` | 新しいバージョンが見つかったときのお知らせ |
| `feature-shortcuts.png` | グローバルショートカット設定ダイアログ |
| `feature-overlay-appearance.png` | オーバーレイ表示とステータス項目の設定 |
| `feature-overlay-appearance-detail.png` | 不透明度、縁取り、影、フォント、文字色の設定 |
| `feature-language-english.png` | 英語表示へ切り替えた通常一覧 |
| `feature-language-english-settings.png` | 英語表示へ切り替えた設定 |

## 再撮影手順

`scripts/run-mcp.ps1` を使い、Slint の埋め込み MCP からウィンドウ単位で撮影します。

```powershell
$env:APPDATA = "<repo>\target\demo-appdata"
$env:BPSR_DEMO_3MIN = "30" # 結果画面を撮る場合だけ指定
pwsh scripts/run-mcp.ps1
```

- 画像サイズは、起動時に `SLINT_SCALE_FACTOR=1.5`、設定 `fontSize` 15（コンパクトは 12）、`window_layout.json` の main を 1380x434（コンパクトは 780x353）にして揃える。
- 通常機能は `compactSplitMode: false` の通常表示で撮影する。
- コンパクト表示は `compact-layout.md` 用の2枚だけ撮影する。
- 設定、履歴、各オーバーレイは、対象機能を実際に ON にしてから撮影する。
- 履歴は計測を完了させ、空表示ではなく記録が入った状態を撮影する。
- 言語変更は再起動後に反映されるため、英語画像は `language: "en"` で再起動して撮影する。
- 撮影後は画像を目視し、機能名・説明・表示内容が一致することを確認する。
