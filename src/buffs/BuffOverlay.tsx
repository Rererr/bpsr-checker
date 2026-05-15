import { createSignal, onCleanup, onMount, For } from "solid-js";
import type { JSX } from "solid-js";
import { selfBuffs, startBuffPolling, stopBuffPolling } from "../stores/buffs";
import type { SelfBuffSnapshot } from "../stores/buffs";
import { TinaIcon, AlunaIcon, TartaIcon, BasiliskIcon } from "./icons";

type CharKind = "Tina" | "Aluna" | "Tarta" | "Basilisk";

const CHARS: { kind: CharKind; label: string; Icon: (props: { size?: number }) => JSX.Element }[] = [
  { kind: "Tina",     label: "Tina",     Icon: TinaIcon },
  { kind: "Aluna",    label: "Aluna",    Icon: AlunaIcon },
  { kind: "Tarta",    label: "Tarta",    Icon: TartaIcon },
  { kind: "Basilisk", label: "Basilisk", Icon: BasiliskIcon },
];

function barColor(ratio: number): string {
  if (ratio > 0.5) return "#4caf50";
  if (ratio > 0.2) return "#ffc107";
  return "#f44336";
}

function findSnap(kind: CharKind): SelfBuffSnapshot | null {
  return selfBuffs().buffs.find((b) => b.kind === kind) ?? null;
}

export function BuffOverlay(): JSX.Element {
  const [tick, setTick] = createSignal(0);

  onMount(() => {
    startBuffPolling(200);
    const id = setInterval(() => setTick((n) => n + 1), 50);
    onCleanup(() => {
      clearInterval(id);
      stopBuffPolling();
    });
  });

  return (
    <div
      data-tauri-drag-region
      style={{
        background: "rgba(10, 10, 18, 0.82)",
        "border-radius": "6px",
        padding: "6px 8px",
        display: "flex",
        "flex-direction": "column",
        gap: "4px",
        "min-height": "100vh",
        "font-family": '"Segoe UI", "Meiryo", sans-serif',
      }}
    >
      <For each={CHARS}>
        {(char) => {
          const snap = () => {
            void tick();
            return findSnap(char.kind);
          };
          const remainingMs = (): number => {
            const s = snap();
            if (!s) return 0;
            const elapsed = performance.now() - s.receivedAtMs;
            return Math.max(0, s.remainingMs - elapsed);
          };
          const ratio = (): number => {
            const s = snap();
            if (!s || s.durationMs <= 0) return 0;
            return remainingMs() / s.durationMs;
          };
          const hasSnap = (): boolean => snap() !== null && remainingMs() > 0;

          return (
            <div
              style={{
                display: "flex",
                "align-items": "center",
                gap: "6px",
                opacity: hasSnap() ? "1" : "0.35",
              }}
            >
              <div style={{ width: "20px", height: "20px", flex: "0 0 20px", color: "#ccc" }}>
                <char.Icon size={20} />
              </div>
              <span style={{
                width: "56px",
                "font-size": "11px",
                color: "#ddd",
                "white-space": "nowrap",
                flex: "0 0 56px",
              }}>
                {char.label}
              </span>
              <div
                style={{
                  flex: "1",
                  height: "6px",
                  background: "rgba(255,255,255,0.12)",
                  "border-radius": "3px",
                  overflow: "hidden",
                  cursor: "default",
                }}
              >
                {hasSnap() && (
                  <div
                    style={{
                      width: `${ratio() * 100}%`,
                      height: "100%",
                      background: barColor(ratio()),
                      "border-radius": "3px",
                      transition: "width 50ms linear, background 200ms",
                    }}
                  />
                )}
              </div>
              <span style={{
                width: "36px",
                "text-align": "right",
                "font-size": "11px",
                color: "#bbb",
                "font-variant-numeric": "tabular-nums",
                "white-space": "nowrap",
                flex: "0 0 36px",
                cursor: "default",
              }}>
                {hasSnap() ? `${(remainingMs() / 1000).toFixed(1)}s` : "--"}
              </span>
            </div>
          );
        }}
      </For>
    </div>
  );
}
