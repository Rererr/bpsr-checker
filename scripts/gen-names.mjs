// 名前辞書の多言語生成器（dev 専用・再実行可）。
//
// ソースは BPSR-ZDPS（MIT, https://github.com/Blue-Protocol-Source/BPSR-ZDPS）の
// 抽出テーブルのみ。AGPL の姉妹リポ（StarResonanceDps / resonance-logs-cn）は参照しない。
// ファイルはコピーせず、本スクリプトでローカル派生して bpsr-checker の id 空間に合わせた
// 薄い id→ゲーム名 辞書を生成する。出自は docs/NAMES_PROVENANCE.md に明記。
//
// 使い方:  node scripts/gen-names.mjs [--zdps <BPSR-ZDPS/Data へのパス>]
//   既定の ZDPS パス: ../../BPSR-ZDPS/BPSR-ZDPS/Data（bpsr-checker の隣に clone 済み前提）
//
// 生成物:
//   core/data/json/SkillName.json         （上書き・EN品質向上。全言語共通＝言語別ソース無し）
//   core/data/json/MonsterNameBoss.en.json （新規・EN。ja は既存 MonsterNameBoss.json を維持）
//   slint-app/data/BuffName.en.json        （新規・EN。BuffOverrides.en > BuffTable.Name(英語)）
//
// MIT のみ制約のため、我々の表示 id 集合に CN/JP の clean なゲーム名ソースは存在しない
// （ZDPS は EN フォーク。BuffTable/MonsterTable/SkillTable の Name は表示 id では英語、
//  中国語は内部コードネームのみ）。よって:
//   - スキル: EN のみ（ja/zh は実行時 EN フォールバック）
//   - モンスター: ja(既存)＋EN（zh は EN フォールバック）
//   - バフ: ja(既存)＋EN（zh は EN フォールバック）
//   - 簡体字ゲーム名は生成しない（zh ユーザーは中国語UI＋英語ゲーム名）

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repo = resolve(__dirname, "..");

const args = process.argv.slice(2);
const zdpsArgIdx = args.indexOf("--zdps");
const zdpsDir =
  zdpsArgIdx >= 0 && args[zdpsArgIdx + 1]
    ? resolve(args[zdpsArgIdx + 1])
    : resolve(repo, "..", "BPSR-ZDPS", "BPSR-ZDPS", "Data");

const readJson = (p) => JSON.parse(readFileSync(p, "utf8"));
const z = (name) => readJson(join(zdpsDir, name));

console.log(`[gen-names] ZDPS source: ${zdpsDir}`);

// --- ZDPS（MIT）ソース読み込み ---
const skillTable = z("SkillTable.json");
const skillOvr = z("SkillOverrides.en.json");
const buffTable = z("BuffTable.json");
const buffOvr = z("BuffOverrides.en.json");
const monsterTable = z("MonsterTable.json");

// --- bpsr-checker 既存辞書（id 空間の正典） ---
const skillCurrent = readJson(join(repo, "core/data/json/SkillName.json"));
const monsterJa = readJson(join(repo, "core/data/json/MonsterNameBoss.json"));
const buffJa = readJson(join(repo, "slint-app/data/BuffName.ja.json"));

const isLatin = (s) => typeof s === "string" && /[A-Za-z]/.test(s) && !/[一-鿿]/.test(s);
const sortNumeric = (obj) =>
  Object.fromEntries(Object.keys(obj).sort((a, b) => Number(a) - Number(b)).map((k) => [k, obj[k]]));

// =====================================================================
// 1) スキル: 既存 id を維持しつつ EN clean 名で品質向上＋curated override の id を追加。
//    優先: SkillOverrides.en.Name > SkillTable.Name(英語) > 既存値。
// =====================================================================
{
  const out = { ...skillCurrent };
  let upgraded = 0;
  let added = 0;
  const enName = (id) => {
    const o = skillOvr[id]?.Name;
    if (o) return o;
    const t = skillTable[id]?.Name;
    if (isLatin(t)) return t; // Table.Name はコードネーム(中文)のこともあるので英字のみ採用
    return null;
  };
  // 既存 id を upgrade
  for (const id of Object.keys(out)) {
    const en = enName(id);
    if (en && en !== out[id]) {
      out[id] = en;
      upgraded++;
    }
  }
  // curated override の id を追加（未収録のみ）
  for (const id of Object.keys(skillOvr)) {
    if (!(id in out)) {
      const en = enName(id);
      if (en) {
        out[id] = en;
        added++;
      }
    }
  }
  writeFileSync(
    join(repo, "core/data/json/SkillName.json"),
    JSON.stringify(sortNumeric(out), null, 2) + "\n",
  );
  console.log(`[gen-names] SkillName.json: ${Object.keys(out).length} entries (upgraded ${upgraded}, added ${added})`);
}

// =====================================================================
// 2) モンスター EN: 既存 ja の id 集合に対して MonsterTable.Name(英語)を採用。
//    無い分は ja 値を残す（実行時フォールバックの保険）。
// =====================================================================
{
  const out = {};
  let covered = 0;
  for (const id of Object.keys(monsterJa)) {
    const en = monsterTable[id]?.Name;
    if (isLatin(en)) {
      out[id] = en;
      covered++;
    } else {
      out[id] = monsterJa[id]; // フォールバック保険
    }
  }
  writeFileSync(
    join(repo, "core/data/json/MonsterNameBoss.en.json"),
    JSON.stringify(sortNumeric(out), null, 2) + "\n",
  );
  console.log(`[gen-names] MonsterNameBoss.en.json: ${Object.keys(out).length} entries (EN covered ${covered})`);
}

// =====================================================================
// 3) バフ en: 既存 ja の id 集合に対し BuffOverrides.en.Name > BuffTable.Name(英語)。
//    我々の表示 id では BuffTable.Name は全て英語（中国語は内部 id のみ）。
//    形は既存 BuffName.ja.json と同じ { "id": { "name": "..." } }。
// =====================================================================
{
  const en = {};
  let enCovered = 0;
  let fromOvr = 0;
  for (const id of Object.keys(buffJa)) {
    const name = buffOvr[id]?.Name || (isLatin(buffTable[id]?.Name) ? buffTable[id].Name : null);
    if (name) {
      en[id] = { name };
      enCovered++;
      if (buffOvr[id]?.Name) fromOvr++;
    }
  }
  writeFileSync(
    join(repo, "slint-app/data/BuffName.en.json"),
    JSON.stringify(sortNumeric(en), null, 2) + "\n",
  );
  console.log(`[gen-names] BuffName.en.json: ${Object.keys(en).length} entries (en covered ${enCovered}, ${fromOvr} from curated overrides)`);
}

console.log("[gen-names] done.");
