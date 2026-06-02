import { createSignal, createEffect, For, Show, onCleanup } from "solid-js";
import { t } from "../lib/i18n";
import { fetchSkills } from "../stores/encounter";
import { showCrit, showLucky, showHpm, showCritValue, showLuckyValue, showHits, pollIntervalMs, privacyMaskNames, showElement, showDamageMode } from "../stores/settings";
import { formatNumber, formatDps, formatPct, getClassColor, maskPlayerName, elementLabel, damageModeLabel } from "../utils";
import type { SkillsWindow, SkillRow } from "../stores/encounter";

interface SkillTableProps {
  playerUid: number;
  onBack: () => void;
}

export function SkillTable(props: SkillTableProps) {
  const [data, setData] = createSignal<SkillsWindow | null>(null);

  createEffect(() => {
    const uid = props.playerUid;
    const fetchLoop = async () => {
      const result = await fetchSkills(uid);
      if (result) setData(result);
    };
    fetchLoop();
    const interval = setInterval(fetchLoop, pollIntervalMs());
    onCleanup(() => clearInterval(interval));
  });

  const gridCols = () => {
    let cols = "minmax(80px, 1.5fr)";
    if (showElement()) cols += " 36px";
    if (showDamageMode()) cols += " 40px";
    cols += " 70px 65px 45px";
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
      {/* Header with back button and player info */}
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
                  style={{ color: !privacyMaskNames() && !p().nameResolved ? "#777" : undefined }}
                  title={!privacyMaskNames() && !p().nameResolved ? "名前未取得 — ゾーン移動で再取得できます" : undefined}
                >{privacyMaskNames() ? maskPlayerName(p().name, p().uid) : p().name}</span>
                <span style={{ color: "#4fc3f7" }}>{formatDps(p().valuePerSec)} DPS</span>
                <span style={{ color: "#aaa" }}>{formatPct(p().valuePct)}</span>
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
        <span>{t("skill")}</span>
        <Show when={showElement()}>
          <span style={{ "text-align": "center" }}>{t("element")}</span>
        </Show>
        <Show when={showDamageMode()}>
          <span style={{ "text-align": "center" }}>{t("damage_mode")}</span>
        </Show>
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

      {/* Skill rows */}
      <Show when={data()}>
        {(d) => (
          <For each={d().skillRows}>
            {(row) => (
              <SkillRowItem row={row} topValue={d().topValue} gridCols={gridCols()} />
            )}
          </For>
        )}
      </Show>
    </div>
  );
}

interface SkillRowItemProps {
  row: SkillRow;
  topValue: number;
  gridCols: string;
}

function SkillRowItem(props: SkillRowItemProps) {
  const barWidth = () =>
    props.topValue > 0 ? (props.row.totalValue / props.topValue) * 100 : 0;

  return (
    <div
      style={{
        position: "relative",
        display: "grid",
        "grid-template-columns": props.gridCols,
        padding: "3px 8px",
        "font-size": "12px",
        "border-bottom": "1px solid rgba(255,255,255,0.03)",
      }}
    >
      <div
        style={{
          position: "absolute",
          left: "0",
          top: "0",
          bottom: "0",
          width: `${barWidth()}%`,
          background: "linear-gradient(90deg, rgba(79,195,247,0.2), transparent)",
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
      <Show when={showElement()}>
        {() => {
          const el = elementLabel(props.row.element);
          return (
            <span style={{ "text-align": "center", "z-index": "1", color: el.color, "font-size": "11px" }}>
              {el.icon}{el.name}
            </span>
          );
        }}
      </Show>
      <Show when={showDamageMode()}>
        {() => {
          const md = damageModeLabel(props.row.damageMode);
          return (
            <span style={{ "text-align": "center", "z-index": "1", color: md.color, "font-size": "11px" }}>
              {md.name}
            </span>
          );
        }}
      </Show>
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
