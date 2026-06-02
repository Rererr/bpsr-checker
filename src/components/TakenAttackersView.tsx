import { createEffect, For, Show, onCleanup } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { t } from "../lib/i18n";
import { fetchTakenAttackers } from "../stores/encounter";
import { showCrit, showLucky, showHpm, showCritValue, showLuckyValue, showHits, pollIntervalMs } from "../stores/settings";
import { formatNumber, formatDps, formatPct, getClassColor } from "../utils";
import type { SkillsWindow, SkillRow } from "../stores/encounter";

interface TakenAttackersViewProps {
  playerUid: number;
  onSelectAttacker: (attackerUid: number, attackerName: string) => void;
  onBack: () => void;
}

export function TakenAttackersView(props: TakenAttackersViewProps) {
  // ポーリング毎に丸ごと差し替えると <For> が全行 DOM を作り直し、クリックの
  // mousedown→mouseup の間にノードが消えて行クリックが発火しない。reconcile(uid)
  // で行の同一性を維持する。
  const [state, setState] = createStore<{ data: SkillsWindow | null }>({ data: null });
  const data = () => state.data;

  createEffect(() => {
    const uid = props.playerUid;
    const fetchLoop = async () => {
      const result = await fetchTakenAttackers(uid);
      if (result) setState("data", reconcile(result, { key: "uid" }));
    };
    fetchLoop();
    const interval = setInterval(fetchLoop, pollIntervalMs());
    onCleanup(() => clearInterval(interval));
  });

  const gridCols = () => {
    let cols = "minmax(80px, 1.5fr) 70px 65px 45px";
    if (showCrit()) cols += " 50px";
    if (showCritValue()) cols += " 50px";
    if (showLucky()) cols += " 50px";
    if (showLuckyValue()) cols += " 50px";
    if (showHits()) cols += " 50px";
    if (showHpm()) cols += " 50px";
    return cols;
  };

  return (
    <div style={{ flex: "1", display: "flex", "flex-direction": "column", overflow: "auto" }}>
      {/* Header */}
      <div
        style={{
          display: "flex",
          "align-items": "center",
          gap: "8px",
          padding: "4px 8px",
          "border-bottom": "1px solid rgba(255,255,255,0.08)",
        }}
      >
        <button
          onClick={props.onBack}
          style={{
            padding: "2px 8px",
            border: "1px solid rgba(255,255,255,0.2)",
            "border-radius": "3px",
            background: "transparent",
            color: "#ccc",
            cursor: "pointer",
            "font-size": "11px",
          }}
        >
          {t("back")}
        </button>
        <Show when={data()}>
          {(d) => {
            const p = () => d().inspectedPlayer;
            return (
              <div style={{ display: "flex", gap: "8px", "align-items": "center", "font-size": "12px" }}>
                <span
                  style={{
                    width: "3px",
                    height: "14px",
                    "border-radius": "1px",
                    background: getClassColor(p().className),
                  }}
                />
                <span
                  style={{ color: !p().nameResolved ? "#777" : undefined }}
                  title={!p().nameResolved ? "名前未取得 — ゾーン移動で再取得できます" : undefined}
                >{p().name}</span>
                <span style={{ color: "#e67e22" }}>{formatNumber(p().totalValue)}</span>
                <span style={{ color: "#aaa" }}>{formatDps(p().valuePerSec)}/s</span>
                <span style={{ color: "#888", "font-size": "10px" }}>{t("attacker")}</span>
              </div>
            );
          }}
        </Show>
      </div>

      {/* Column headers */}
      <div
        style={{
          display: "grid",
          "grid-template-columns": gridCols(),
          padding: "2px 8px",
          "font-size": "10px",
          color: "#888",
          "border-bottom": "1px solid rgba(255,255,255,0.05)",
        }}
      >
        <span>{t("attacker")}</span>
        <span style={{ "text-align": "right" }}>{t("damage")}</span>
        <span style={{ "text-align": "right" }}>{t("dps")}</span>
        <span style={{ "text-align": "right" }}>{t("pct")}</span>
        <Show when={showCrit()}>
          <span style={{ "text-align": "right" }}>{t("crit_rate")}</span>
        </Show>
        <Show when={showCritValue()}>
          <span style={{ "text-align": "right" }}>{t("crit_value")}</span>
        </Show>
        <Show when={showLucky()}>
          <span style={{ "text-align": "right" }}>{t("lucky_rate")}</span>
        </Show>
        <Show when={showLuckyValue()}>
          <span style={{ "text-align": "right" }}>{t("lucky_value")}</span>
        </Show>
        <Show when={showHits()}>
          <span style={{ "text-align": "right" }}>{t("hits")}</span>
        </Show>
        <Show when={showHpm()}>
          <span style={{ "text-align": "right" }}>{t("hpm")}</span>
        </Show>
      </div>

      {/* Attacker rows */}
      <Show when={data()}>
        {(d) => (
          <For each={d().skillRows}>
            {(row) => (
              <AttackerRowItem
                row={row}
                topValue={d().topValue}
                gridCols={gridCols()}
                onClick={() => props.onSelectAttacker(row.uid, row.name)}
              />
            )}
          </For>
        )}
      </Show>
    </div>
  );
}

interface AttackerRowItemProps {
  row: SkillRow;
  topValue: number;
  gridCols: string;
  onClick: () => void;
}

function AttackerRowItem(props: AttackerRowItemProps) {
  const barWidth = () =>
    props.topValue > 0 ? (props.row.totalValue / props.topValue) * 100 : 0;

  return (
    <div
      onClick={props.onClick}
      style={{
        position: "relative",
        display: "grid",
        "grid-template-columns": props.gridCols,
        padding: "3px 8px",
        "font-size": "12px",
        cursor: "pointer",
        "border-bottom": "1px solid rgba(255,255,255,0.03)",
      }}
      onMouseEnter={(e) => { e.currentTarget.style.background = "rgba(255,255,255,0.05)"; }}
      onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}
    >
      <div
        style={{
          position: "absolute",
          left: "0",
          top: "0",
          bottom: "0",
          width: `${barWidth()}%`,
          background: "linear-gradient(90deg, rgba(230,126,34,0.2), transparent)",
          "pointer-events": "none",
        }}
      />
      <span style={{
        "z-index": "1",
        overflow: "hidden",
        "text-overflow": "ellipsis",
        "white-space": "nowrap",
      }}>
        {props.row.name}
      </span>
      <span style={{ "text-align": "right", "z-index": "1" }}>
        {formatNumber(props.row.totalValue)}
      </span>
      <span style={{ "text-align": "right", "z-index": "1", color: "#4fc3f7" }}>
        {formatDps(props.row.valuePerSec)}
      </span>
      <span style={{ "text-align": "right", "z-index": "1" }}>
        {formatPct(props.row.valuePct)}
      </span>
      <Show when={showCrit()}>
        <span style={{ "text-align": "right", "z-index": "1", color: "#f39c12" }}>
          {formatPct(props.row.critRate)}
        </span>
      </Show>
      <Show when={showCritValue()}>
        <span style={{ "text-align": "right", "z-index": "1", color: "#f39c12" }}>
          {formatPct(props.row.critValueRate)}
        </span>
      </Show>
      <Show when={showLucky()}>
        <span style={{ "text-align": "right", "z-index": "1", color: "#2ecc71" }}>
          {formatPct(props.row.luckyRate)}
        </span>
      </Show>
      <Show when={showLuckyValue()}>
        <span style={{ "text-align": "right", "z-index": "1", color: "#2ecc71" }}>
          {formatPct(props.row.luckyValueRate)}
        </span>
      </Show>
      <Show when={showHits()}>
        <span style={{ "text-align": "right", "z-index": "1" }}>
          {props.row.hits}
        </span>
      </Show>
      <Show when={showHpm()}>
        <span style={{ "text-align": "right", "z-index": "1" }}>
          {props.row.hitsPerMinute.toFixed(1)}
        </span>
      </Show>
    </div>
  );
}
