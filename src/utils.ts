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
  "ストームブレイド": "#e74c3c",
  "フロストメイジ": "#3498db",
  "ウィンドナイト": "#2ecc71",
  "ヴァーダントオラクル": "#27ae60",
  "ヘビーガーディアン": "#e67e22",
  "マークスマン": "#9b59b6",
  "シールドナイト": "#f1c40f",
  "ビートパフォーマー": "#e91e63",
  "不明クラス": "#95a5a6",
  "未実装クラス": "#7f8c8d",
};

export function getClassColor(className: string): string {
  return CLASS_COLORS[className] ?? "#95a5a6";
}

export function maskPlayerName(name: string, uid: number): string {
  return `Player#${(uid & 0xffff).toString(16).padStart(4, "0").toUpperCase()}`;
}

import type { PlayerRow } from "./stores/encounter";

const MISSING = "—";

export function formatRowAsText(row: PlayerRow, rank: number, template: string): string {
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
    score: row.abilityScore > 0 ? formatNumber(row.abilityScore) : MISSING,
    seasonLv: row.seasonLevel > 0 ? row.seasonLevel.toString() : MISSING,
    seasonStr: row.seasonStrength > 0 ? formatNumber(row.seasonStrength) : MISSING,
  };
  return template.replace(/\{(\w+)\}/g, (_m, k) => map[k] ?? `{${k}}`);
}
