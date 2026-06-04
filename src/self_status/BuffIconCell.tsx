import { Show } from "solid-js";
import type { JSX } from "solid-js";
import type { StatusEntry, BuffNameEntry } from "./types";
import { tick, pollReceivedAt } from "./store";

// バフ種別ごとのカラーチップ色
const CATEGORY_COLOR: Record<string, string> = {
  buff:     "#4fc3f7",
  debuff:   "#ef5350",
  recovery: "#66bb6a",
  item:     "#ffa726",
  unknown:  "#78909c",
};

// 優先度ごとのセル枠線色
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
    void tick(); // 50ms 補間トリガー
    const elapsed = performance.now() - pollReceivedAt();
    return Math.max(0, props.entry.remainingMs - elapsed);
  };

  const displayRemaining = () => interpolatedRemaining();
  const isLow = () => props.entry.durationMs > 0 && displayRemaining() < 3000;
  const nameEntry = () => props.nameDict[String(props.entry.baseId)];
  const label = () => nameEntry()?.name ?? `不明 #${props.entry.baseId}`;

  const chipColor = CATEGORY_COLOR[props.entry.category] ?? CATEGORY_COLOR.unknown;
  const borderColor = PRIORITY_BORDER[props.entry.priority] ?? PRIORITY_BORDER.normal;

  const barColor = props.entry.category === "debuff" ? "#ef5350" : "#4fc3f7";

  return (
    <div
      style={{
        display: "flex",
        "flex-direction": "column",
        "align-items": "center",
        width: "56px",
        padding: "3px 2px",
        background: "rgba(0,0,0,0.45)",
        border: `1px solid ${borderColor}`,
        "border-radius": "4px",
        gap: "2px",
        "flex-shrink": "0",
        opacity: isLow() ? undefined : "1",
        animation: isLow() ? "status-pulse 0.6s ease-in-out infinite" : "none",
      }}
    >
      {/* カラーチップ（アイコン代替） */}
      <div
        style={{
          width: "28px",
          height: "28px",
          "border-radius": "4px",
          background: chipColor,
          opacity: "0.85",
        }}
      />

      {/* 名前 */}
      <div
        style={{
          "font-size": "8px",
          color: "rgba(255,255,255,0.75)",
          "text-align": "center",
          "line-height": "1.1",
          width: "100%",
          "overflow": "hidden",
          "text-overflow": "ellipsis",
          "white-space": "nowrap",
        }}
        title={label()}
      >
        {label()}
      </div>

      {/* 残時間バー */}
      <div
        style={{
          width: "100%",
          height: "3px",
          background: "rgba(255,255,255,0.12)",
          "border-radius": "2px",
          overflow: "hidden",
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

      {/* 残秒数 */}
      <div
        style={{
          "font-size": "9px",
          "font-weight": "600",
          color: isLow() ? "#ff7043" : "rgba(255,255,255,0.9)",
          "line-height": "1",
        }}
      >
        {formatRemaining(displayRemaining(), props.entry.durationMs)}
      </div>

      {/* レイヤー数（2以上の場合のみ） */}
      <Show when={props.entry.layer > 1}>
        <div
          style={{
            "font-size": "8px",
            color: "rgba(255,255,255,0.5)",
          }}
        >
          ×{props.entry.layer}
        </div>
      </Show>
    </div>
  );
}
