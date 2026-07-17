# 名前辞書の出自（Third-party notices）

本リポジトリが同梱する名前辞書データ（下記ファイル）には、**BPSR-ZDPS**（MIT ライセンス）の
テーブルからローカルで派生したデータが含まれる。

- **BPSR-ZDPS** — https://github.com/Blue-Protocol-Source/BPSR-ZDPS
  - License: MIT — Copyright (c) 2025 Blue-Protocol-Source

## 派生データを含む同梱ファイル

- `core/data/json/SkillName.json` / `SkillName.ja.json`（一部）
- `core/data/json/MonsterNameBoss.en.json`
- `core/data/json/ImagineSkillNames.json`（召喚/分身 id の紐付け）
- `core/data/json/ConsumableBuffIds.json`
- `slint-app/data/BuffName.en.json`
- `slint-app/data/ConsumableBuffNames.ja.json`

元ファイルはコピーせず、bpsr-checker の id 空間に合わせた薄い `id → 名前` 辞書として
ローカル管理の生成スクリプトで導出している。日本語名は自前環境のゲームクライアントに
由来する。AGPL-3 の類似プロジェクトは本プロジェクト（GPL-3.0-only）へ取り込めないため、
派生ソースには使用していない。
