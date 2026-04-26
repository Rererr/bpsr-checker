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
  Stormblade: "#e74c3c",
  "Frost Mage": "#3498db",
  "Wind Knight": "#2ecc71",
  "Verdant Oracle": "#27ae60",
  "Heavy Guardian": "#e67e22",
  Marksman: "#9b59b6",
  "Shield Knight": "#f1c40f",
  "Beat Performer": "#e91e63",
  "Unknown Class": "#95a5a6",
  "Unimplemented Class": "#7f8c8d",
};

export function getClassColor(className: string): string {
  return CLASS_COLORS[className] ?? "#95a5a6";
}
