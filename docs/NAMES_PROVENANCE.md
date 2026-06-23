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

- **JA スキル名** — `core/data/json/SkillName.ja.json`（包括的な日本語辞書・約 1.5 万件）。
  出自はゲーム内日本語名称（CN→JA 変換 / upstream 同期由来）で、公式 loc からの一意抽出ではない。
  > 当初は公式 loc から一意確定した 1084 件のみへ絞ったが、JA 表示で大半が EN へ落ち日英混在となったため、
  > 旧 `SkillName.json` 由来の包括辞書を JA 用に復元した。さらに復元辞書には未翻訳の英語・中国語・"copy"
  > 残骸が残っていたため、公式 loc から high 信頼で一意抽出した JA 名 **373 件**を上書き適用し是正済み
  > （英語/中国語/copy 残骸＋漢字のみの中国語残骸。段階派生技の「基本名＋段数」と公式 loc 実在の確定 JA は温存）。
  > 抽出は override 名＋生 `SkillTable.Name` の二刀流＋アンカー補間（`tools/match-skill-ja2.mjs`・gitignore）。
  > 完全な公式名（残りの曖昧分）は別 pkg 探索による後続タスク（`docs/i18n-game-extraction.md`）。

## 生成物と言語カバレッジ

| ファイル | 言語 | 由来 |
|---|---|---|
| `core/data/json/SkillName.json` | EN（基準辞書） | `SkillOverrides.en.Name` > `SkillTable.Name`、未収録 id は従来値を保持 |
| `core/data/json/SkillName.ja.json` | JA（包括・復元） | 旧 `SkillName.json` 由来の包括日本語辞書を復元。JA 表示時に優先 |
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

スキル名は EN を基準辞書（`SkillName.json`）とし、JA 表示時のみ `SkillName.ja.json`（包括日本語辞書）
を優先する（未収録 id・en/zh は EN）。これにより JA 表示は実スキル名がすべて日本語になる。

## 再生成

```bash
node scripts/gen-names.mjs            # 既定: ../BPSR-ZDPS/BPSR-ZDPS/Data を参照
node scripts/gen-names.mjs --zdps <path>
```

ゲーム更新で BPSR-ZDPS が更新されたら再実行する。
