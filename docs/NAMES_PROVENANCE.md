# 名前辞書の出自（多言語名データ）

スキル/モンスター/バフの英語表示名は **BPSR-ZDPS**（MIT ライセンス）の抽出テーブルを
ローカルで派生して生成している。元ファイルはコピーせず、`scripts/gen-names.mjs` が
bpsr-checker の id 空間に合わせた薄い `id → 名前` 辞書を出力する。

## ソース

- **BPSR-ZDPS** — https://github.com/Blue-Protocol-Source/BPSR-ZDPS
  - License: MIT — Copyright (c) 2025 Blue-Protocol-Source
  - 使用テーブル: `BPSR-ZDPS/Data/{SkillTable,SkillOverrides.en,BuffTable,BuffOverrides.en,MonsterTable}.json`

> AGPL-3 の姉妹リポ（StarResonanceDps / resonance-logs-cn）は本プロジェクト（GPL-3.0-only）
> へ取り込めないため、名前データの派生ソースには**使用しない**。

- **公式 JA スキル名** — 所有する JA ゲームビルドから派生（`core/data/json/SkillName.ja.json`）。
  EN→JA が一意に定まる確実分のみを収録（曖昧な推定は除外）。**派生に用いた抽出ツール・調査メモは
  リポジトリに含めずローカル管理**（生成物の辞書 JSON のみ同梱）。

## 生成物と言語カバレッジ

| ファイル | 言語 | 由来 |
|---|---|---|
| `core/data/json/SkillName.json` | EN（基準辞書） | `SkillOverrides.en.Name` > `SkillTable.Name`、未収録 id は従来値を保持 |
| `core/data/json/SkillName.ja.json` | JA（公式・一部） | 所有 JA ゲームビルド由来。JA 表示時に収録 id のみ優先 |
| `core/data/json/MonsterNameBoss.json` | JA（既存・手当て） | 本リポ既存（変更しない） |
| `core/data/json/MonsterNameBoss.en.json` | EN | `MonsterTable.Name` |
| `slint-app/data/BuffName.ja.json` | JA（既存・手当て） | 本リポ既存（変更しない） |
| `slint-app/data/BuffName.en.json` | EN | `BuffOverrides.en.Name` > `BuffTable.Name` |

### MIT のみ制約による限界

BPSR-ZDPS は実質 **EN フォーク**で、我々の表示 id 集合では各テーブルの `Name` は英語
（簡体字は内部コードネーム id のみ）。よって **clean な簡体字（CN）/ 公式日本語（JP）の
ゲーム名ソースは存在しない**。表示言語ごとの解決は次のフォールバックで行う:

- **ja**: ja（既存 curated）→ en → id
- **en**: en → ja → id
- **zh**: en → ja → id （簡体字ゲーム名が無いため英語ゲーム名を使用。UI 文字列のみ中国語）

スキル名は EN を基準辞書（`SkillName.json`）とし、JA 表示時のみ `SkillName.ja.json` に
収録のある id で公式 JA 名を優先する（未収録 id・en/zh は EN）。

## 再生成

```bash
node scripts/gen-names.mjs            # 既定: ../BPSR-ZDPS/BPSR-ZDPS/Data を参照
node scripts/gen-names.mjs --zdps <path>
```

ゲーム更新で BPSR-ZDPS が更新されたら再実行する。
