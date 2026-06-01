import type { JSX } from "solid-js";
import { pollReceivedAt, tick } from "../stores/buffs";
import type { CharKind, SelfBuffSnapshot } from "./types";

export type { CharKind };

export const CHAR_KINDS: CharKind[] = ["Tina", "Aluna", "Tarta", "Basilisk"];

export const KIND_COLORS: Record<CharKind, string> = {
  Tina: "#ff4d6d",
  Aluna: "#5fd35f",
  Tarta: "#b98bff",
  Basilisk: "#d9a05b",
};

const SIZE = 32;
const STROKE = 3;
const R = SIZE / 2 - STROKE / 2; // 14.5
const CIRCUMFERENCE = 2 * Math.PI * R; // ≈ 91.11

interface CircularBuffProps {
  kind: CharKind;
  snap: SelfBuffSnapshot | null;
}

export function CircularBuff(props: CircularBuffProps): JSX.Element {
  const remainingMs = () => {
    void tick(); // 50ms 補間トリガー
    const s = props.snap;
    if (!s) return 0;
    const elapsed = performance.now() - pollReceivedAt();
    return Math.max(0, s.remainingMs - elapsed);
  };

  const ratio = () => {
    const s = props.snap;
    if (!s || s.durationMs <= 0) return 0;
    return Math.min(1, remainingMs() / s.durationMs);
  };

  const dashOffset = () => CIRCUMFERENCE * (1 - ratio());

  const active = () => props.snap !== null && remainingMs() > 0;

  const secText = () => {
    const s = props.snap;
    if (!s) return "";
    if (s.durationMs <= 0) return "∞";
    const rem = remainingMs();
    if (rem <= 0) return "OK";
    const sec = Math.ceil(rem / 1000);
    return sec > 999 ? "999+" : String(sec);
  };

  const textColor = () => {
    const rem = remainingMs();
    if (!active() || rem <= 0) return "#888";
    if (rem < 3000) return "#ff5252";
    return "#ddd";
  };

  const shouldPulse = () => active() && remainingMs() > 0 && remainingMs() < 1500;

  return (
    <div
      style={{
        position: "relative",
        width: `${SIZE}px`,
        height: `${SIZE}px`,
        "flex-shrink": "0",
        opacity: active() ? "1" : "0.25",
        animation: shouldPulse() ? "buff-pulse 0.5s ease-in-out infinite" : "none",
      }}
    >
      <svg
        width={SIZE}
        height={SIZE}
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        style={{ display: "block" }}
        aria-hidden="true"
      >
        {/* track */}
        <circle
          cx={SIZE / 2}
          cy={SIZE / 2}
          r={R}
          stroke="rgba(255,255,255,0.12)"
          stroke-width={STROKE}
          fill="none"
        />
        {/* progress */}
        <circle
          cx={SIZE / 2}
          cy={SIZE / 2}
          r={R}
          stroke={KIND_COLORS[props.kind]}
          stroke-width={STROKE}
          fill="none"
          stroke-dasharray={`${CIRCUMFERENCE}`}
          stroke-dashoffset={`${dashOffset()}`}
          stroke-linecap="round"
          transform={`rotate(-90 ${SIZE / 2} ${SIZE / 2})`}
          style={{ transition: "stroke-dashoffset 50ms linear" }}
        />
      </svg>
      {/* center text */}
      <div
        style={{
          position: "absolute",
          inset: "0",
          display: "flex",
          "align-items": "center",
          "justify-content": "center",
          "font-size": "9px",
          "font-variant-numeric": "tabular-nums",
          color: textColor(),
          "pointer-events": "none",
          "line-height": "1",
          "letter-spacing": "-0.5px",
          "white-space": "nowrap",
        }}
      >
        {secText()}
      </div>
    </div>
  );
}
