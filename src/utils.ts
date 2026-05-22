export function formatNumber(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return Math.round(n).toString();
}

export function formatDps(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return Math.round(n).toString();
}

export function formatScore(n: number, abbreviate: boolean): string {
  if (abbreviate) return formatNumber(n);
  return Math.round(n).toString();
}

export function formatPct(n: number): string {
  return n.toFixed(1) + "%";
}

export function formatElapsed(ms: number): string {
  const totalSecs = Math.floor(ms / 1000);
  const mins = Math.floor(totalSecs / 60);
  const secs = totalSecs % 60;
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

const CLASS_COLORS: Record<string, string> = {
  "ストームブレイド": "#fd7cff",
  "フロストメイジ": "#3498db",
  "ゲイルランサー": "#c6ffd8",
  "ヴァーダントオラクル": "#139348",
  "ヘビーガーディアン": "#724d2d",
  "ディバインアーチャー": "#fff090",
  "シールドファイター": "#d1a700",
  "ビートパフォーマー": "#e91e63",
  "不明クラス": "#95a5a6",
  "未実装クラス": "#7f8c8d",
};

export function getClassColor(className: string): string {
  return CLASS_COLORS[className] ?? "#95a5a6";
}

interface ElementInfo { icon: string; name: string; color: string }
const ELEMENT_TABLE: ElementInfo[] = [
  { icon: "⚔", name: "物", color: "#aaa" },
  { icon: "🔥", name: "炎", color: "#e74c3c" },
  { icon: "❄", name: "氷", color: "#4fc3f7" },
  { icon: "⚡", name: "雷", color: "#f1c40f" },
  { icon: "🌿", name: "森", color: "#2ecc71" },
  { icon: "💨", name: "風", color: "#1abc9c" },
  { icon: "⛰", name: "岩", color: "#a0522d" },
  { icon: "✨", name: "光", color: "#ecf0f1" },
  { icon: "🌑", name: "闇", color: "#9b59b6" },
];
export function elementLabel(e: number): ElementInfo {
  return ELEMENT_TABLE[e] ?? { icon: "?", name: "-", color: "#666" };
}

interface ModeInfo { name: string; color: string }
const DAMAGE_MODE_TABLE: ModeInfo[] = [
  { name: "-",   color: "#666" },
  { name: "物理", color: "#e67e22" },
  { name: "魔法", color: "#3498db" },
];
export function damageModeLabel(m: number): ModeInfo {
  return DAMAGE_MODE_TABLE[m] ?? { name: "-", color: "#666" };
}

export function maskPlayerName(name: string, uid: number): string {
  return `Player#${(uid & 0xffff).toString(16).padStart(4, "0").toUpperCase()}`;
}

import type { PlayerRow } from "./stores/encounter";

const MISSING = "—";

export function formatRowAsText(row: PlayerRow, rank: number, template: string, abbreviateScores = false): string {
  const spec = row.classSpecName && row.classSpecName !== "不明" ? row.classSpecName : "";
  const map: Record<string, string> = {
    rank: rank.toString(),
    name: row.name,
    class: row.className,
    spec,
    dmg: formatNumber(row.totalValue),
    dps: formatDps(row.valuePerSec),
    pct: formatPct(row.valuePct),
    crit: formatPct(row.critRate),
    critV: formatPct(row.critValueRate),
    lucky: formatPct(row.luckyRate),
    luckyV: formatPct(row.luckyValueRate),
    hits: row.hits.toString(),
    hpm: row.hitsPerMinute.toFixed(1),
    score: row.abilityScore > 0 ? formatScore(row.abilityScore, abbreviateScores) : MISSING,
    seasonLv: row.seasonLevel > 0 ? row.seasonLevel.toString() : MISSING,
    seasonStr: row.seasonStrength > 0 ? formatScore(row.seasonStrength, abbreviateScores) : MISSING,
  };
  return template.replace(/\{(\w+)\}/g, (_m, k) => map[k] ?? `{${k}}`);
}
