import { Show, For, onMount, onCleanup } from "solid-js";
import { t } from "../lib/i18n";
import { threeMinResult, closeThreeMinResult, start3Min } from "../stores/measureMode";
import { threeMinDurationSec, copyTemplate } from "../stores/settings";
import { Sparkline } from "./Sparkline";
import { formatDps, formatNumber, formatElapsed, formatRowAsText } from "../utils";

export function ThreeMinResultModal() {
  const snap = () => threeMinResult();

  let closeButtonRef: HTMLButtonElement | undefined;

  const handleCopyResult = async () => {
    const s = snap();
    if (!s) return;
    const tpl = copyTemplate();
    const text = s.playerRows
      .slice(0, 10)
      .map((r, i) => formatRowAsText(r, i + 1, tpl))
      .join("\n");
    await navigator.clipboard.writeText(text).catch(() => {});
  };

  const handleRestart = async () => {
    closeThreeMinResult();
    await start3Min(threeMinDurationSec());
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") closeThreeMinResult();
  };

  onMount(() => {
    closeButtonRef?.focus();
    window.addEventListener("keydown", handleKeyDown);
    onCleanup(() => window.removeEventListener("keydown", handleKeyDown));
  });

  return (
    <Show when={snap()}>
      {(s) => (
        <div
          style={{
            position: "fixed",
            inset: "0",
            background: "rgba(0,0,0,0.75)",
            display: "flex",
            "align-items": "center",
            "justify-content": "center",
            "z-index": "1000",
            padding: "16px",
          }}
          onClick={(e) => { if (e.target === e.currentTarget) closeThreeMinResult(); }}
        >
          <div
            style={{
              background: "rgba(20,20,30,0.98)",
              border: "1px solid rgba(255,255,255,0.15)",
              "border-radius": "8px",
              padding: "16px 20px",
              "max-width": "560px",
              width: "100%",
              display: "flex",
              "flex-direction": "column",
              gap: "12px",
              "font-size": "12px",
              color: "#ddd",
            }}
          >
            {/* Title */}
            <div style={{ display: "flex", "align-items": "center", "justify-content": "space-between" }}>
              <span style={{ color: "#4fc3f7", "font-weight": "bold", "font-size": "13px" }}>
                {t("mode_3min_result_title")}
              </span>
              <button
                ref={closeButtonRef}
                onClick={closeThreeMinResult}
                style={{ background: "transparent", border: "none", color: "#888", cursor: "pointer", "font-size": "16px", padding: "0 4px" }}
              >
                ×
              </button>
            </div>

            {/* Summary stats */}
            <div style={{ display: "flex", gap: "20px" }}>
              <div>
                <div style={{ color: "#888", "font-size": "10px" }}>DPS</div>
                <div style={{ color: "#4fc3f7", "font-size": "14px" }}>{formatDps(s().totalDps)}</div>
              </div>
              <div>
                <div style={{ color: "#888", "font-size": "10px" }}>{t("total_dmg")}</div>
                <div>{formatNumber(s().totalDmg)}</div>
              </div>
              <div>
                <div style={{ color: "#888", "font-size": "10px" }}>{t("duration")}</div>
                <div>{formatElapsed(s().durationMs)}</div>
              </div>
            </div>

            {/* Sparkline */}
            <Show when={s().timeSeries.length > 0}>
              <Sparkline points={s().timeSeries} width={Math.min(520, window.innerWidth - 48)} height={36} />
            </Show>

            {/* Player rows */}
            <Show
              when={s().playerRows.length > 0}
              fallback={<div style={{ color: "#555", "text-align": "center", padding: "8px" }}>{t("mode_3min_no_damage")}</div>}
            >
              <div style={{ display: "flex", "flex-direction": "column", gap: "2px" }}>
                <For each={s().playerRows.slice(0, 8)}>
                  {(row, i) => (
                    <div style={{ display: "flex", gap: "8px", padding: "3px 4px", background: "rgba(255,255,255,0.04)", "border-radius": "3px" }}>
                      <span style={{ color: "#666", width: "16px", "text-align": "right" }}>{i() + 1}.</span>
                      <span style={{ flex: "1", color: "#ddd" }}>{row.name}</span>
                      <span style={{ color: "#4fc3f7" }}>{formatDps(row.valuePerSec)}</span>
                      <span style={{ color: "#aaa" }}>{formatNumber(row.totalValue)}</span>
                      <span style={{ color: "#777", width: "36px", "text-align": "right" }}>{row.valuePct.toFixed(1)}%</span>
                    </div>
                  )}
                </For>
              </div>
            </Show>

            {/* Buttons */}
            <div style={{ display: "flex", gap: "8px", "justify-content": "flex-end", "margin-top": "4px" }}>
              <button
                onClick={handleCopyResult}
                style={modalBtnStyle()}
              >
                {t("mode_3min_copy_result")}
              </button>
              <button
                onClick={handleRestart}
                style={modalBtnStyle()}
              >
                {t("mode_3min_restart")}
              </button>
              <button
                onClick={closeThreeMinResult}
                style={{ ...modalBtnStyle(), background: "rgba(79,195,247,0.15)", "border-color": "rgba(79,195,247,0.45)", color: "#4fc3f7" }}
              >
                {t("mode_3min_close")}
              </button>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}

function modalBtnStyle() {
  return {
    padding: "4px 12px",
    border: "1px solid rgba(255,255,255,0.2)",
    "border-radius": "4px",
    background: "transparent",
    color: "#ccc",
    cursor: "pointer",
    "font-size": "11px",
  };
}
