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
