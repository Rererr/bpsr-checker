import { Show } from "solid-js";
import type { TimeSeriesPoint } from "../stores/encounter";

interface SparklineProps {
  points: TimeSeriesPoint[];
  width?: number;
  height?: number;
  color?: string;
}

export function Sparkline(props: SparklineProps) {
  const w = () => props.width ?? 120;
  const h = () => props.height ?? 24;
  const color = () => props.color ?? "#4fc3f7";

  const polyline = () => {
    const pts = props.points;
    if (pts.length < 2) return "";
    const max = Math.max(1, ...pts.map((p) => p.totalDps));
    const stepX = w() / (pts.length - 1);
    return pts
      .map((p, i) => {
        const x = (i * stepX).toFixed(1);
        const y = (h() - (p.totalDps / max) * h()).toFixed(1);
        return `${x},${y}`;
      })
      .join(" ");
  };

  return (
    <Show when={props.points.length >= 2}>
      <svg width={w()} height={h()} style={{ display: "block" }}>
        <polyline
          points={polyline()}
          fill="none"
          stroke={color()}
          stroke-width="1.2"
          stroke-linejoin="round"
          stroke-linecap="round"
        />
      </svg>
    </Show>
  );
}
