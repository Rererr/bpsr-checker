import { Show } from "solid-js";
import type { JSX } from "solid-js";
import type { StatusEntry, BuffNameEntry } from "./types";
import { tick, pollReceivedAt } from "./store";

const PRIORITY_BORDER: Record<string, string> = {
  alert:  "#ffd54f",
  high:   "rgba(255,255,255,0.25)",
  normal: "rgba(255,255,255,0.12)",
  low:    "rgba(255,255,255,0.06)",
  hidden: "transparent",
};

function formatRemaining(remainingMs: number, durationMs: number): string {
  if (durationMs === 0) return "∞";
  if (remainingMs <= 0) return "0s";
  const sec = remainingMs / 1000;
  return sec > 10 ? `${Math.ceil(sec)}s` : `${sec.toFixed(1)}s`;
}

function barWidth(remainingMs: number, durationMs: number): string {
  if (durationMs === 0) return "100%";
  const ratio = Math.max(0, Math.min(1, remainingMs / durationMs));
  return `${(ratio * 100).toFixed(1)}%`;
}

interface Props {
  entry: StatusEntry;
  nameDict: Record<string, BuffNameEntry>;
}

export function BuffIconCell(props: Props): JSX.Element {
  const interpolatedRemaining = () => {
    void tick();
    const elapsed = performance.now() - pollReceivedAt();
    return Math.max(0, props.entry.remainingMs - elapsed);
  };

  const displayRemaining = () => interpolatedRemaining();
  const isLow = () => props.entry.durationMs > 0 && displayRemaining() < 3000;
  const nameEntry = () => props.nameDict[String(props.entry.baseId)];
  const label = () => nameEntry()?.name ?? `不明 #${props.entry.baseId}`;

  const borderColor = PRIORITY_BORDER[props.entry.priority] ?? PRIORITY_BORDER.normal;
  const barColor = props.entry.category === "debuff" ? "#ef5350" : "#4fc3f7";

  return (
    <div
      style={{
        display: "flex",
        "flex-direction": "row",
        "align-items": "center",
        gap: "5px",
        padding: "2px 5px",
        background: "rgba(0,0,0,0.35)",
        border: `1px solid ${borderColor}`,
        "border-radius": "3px",
        animation: isLow() ? "status-pulse 0.6s ease-in-out infinite" : "none",
      }}
    >
      {/* 名前 */}
      <div
        style={{
          flex: "1",
          "font-size": "9px",
          color: "rgba(255,255,255,0.85)",
          "overflow": "hidden",
          "text-overflow": "ellipsis",
          "white-space": "nowrap",
          "min-width": "0",
        }}
        title={label()}
      >
        {label()}
      </div>

      {/* 残時間バー */}
      <div
        style={{
          width: "64px",
          height: "4px",
          background: "rgba(255,255,255,0.12)",
          "border-radius": "2px",
          overflow: "hidden",
          "flex-shrink": "0",
        }}
      >
        <div
          style={{
            width: barWidth(displayRemaining(), props.entry.durationMs),
            height: "100%",
            background: isLow() ? "#ff7043" : barColor,
            "border-radius": "2px",
            transition: "width 0.1s linear",
          }}
        />
      </div>

      {/* レイヤー数: 常に固定幅スロットを確保して残時間バーの左端を揃える（2以上のみ表示） */}
      <div
        style={{
          "font-size": "8px",
          color: "rgba(255,255,255,0.45)",
          "flex-shrink": "0",
          width: "22px",
          "text-align": "left",
        }}
      >
        <Show when={props.entry.layer > 1}>×{props.entry.layer}</Show>
      </div>

      {/* 残秒数 */}
      <div
        style={{
          "font-size": "9px",
          "font-weight": "600",
          color: isLow() ? "#ff7043" : "rgba(255,255,255,0.9)",
          "text-align": "right",
          width: "36px",
          "flex-shrink": "0",
          "font-variant-numeric": "tabular-nums",
        }}
      >
        {formatRemaining(displayRemaining(), props.entry.durationMs)}
      </div>
    </div>
  );
}
