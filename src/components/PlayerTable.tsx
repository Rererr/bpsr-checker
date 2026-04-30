import { For, Show } from "solid-js";
import { t } from "../lib/i18n";
import { dpsPlayers, healPlayers, bossPlayers } from "../stores/encounter";
import { showCrit, showLucky, showHpm, showScore } from "../stores/settings";
import { formatNumber, formatDps, formatPct, getClassColor } from "../utils";
import type { Tab } from "../App";
import type { PlayerRow, PlayersWindow } from "../stores/encounter";

interface PlayerTableProps {
  tab: Tab;
  onSelectPlayer: (uid: number) => void;
}

export function PlayerTable(props: PlayerTableProps) {
  const data = (): PlayersWindow => {
    switch (props.tab) {
      case "heal": return healPlayers();
      default: return dpsPlayers();
    }
  };

  return (
    <div style={{ flex: "1", overflow: "auto" }}>
      {/* Column headers */}
      <div
        style={{
          display: "grid",
          "grid-template-columns": gridCols(),
          padding: "2px 8px",
          "font-size": "10px",
          color: "#888",
          "border-bottom": "1px solid rgba(255,255,255,0.05)",
          "user-select": "none",
        }}
      >
        <span>{t("player")}</span>
        <span style={{ "text-align": "right" }}>{t("damage")}</span>
        <span style={{ "text-align": "right" }}>{t("dps")}</span>
        <span style={{ "text-align": "right" }}>{t("pct")}</span>
        <Show when={showCrit()}>
          <span style={{ "text-align": "right" }}>{t("crit_rate")}</span>
        </Show>
        <Show when={showLucky()}>
          <span style={{ "text-align": "right" }}>{t("lucky_rate")}</span>
        </Show>
        <Show when={showHpm()}>
          <span style={{ "text-align": "right" }}>{t("hpm")}</span>
        </Show>
        <Show when={showScore()}>
          <span style={{ "text-align": "right" }}>{t("score")}</span>
        </Show>
      </div>

      {/* Rows */}
      <Show
        when={data().playerRows.length > 0}
        fallback={
          <div style={{ padding: "20px", "text-align": "center", color: "#666" }}>
            {t("no_data")}
          </div>
        }
      >
        <For each={data().playerRows}>
          {(row) => (
            <PlayerRowItem
              row={row}
              topValue={data().topValue}
              onClick={() => props.onSelectPlayer(row.uid)}
            />
          )}
        </For>
      </Show>
    </div>
  );
}

interface PlayerRowItemProps {
  row: PlayerRow;
  topValue: number;
  onClick: () => void;
}

function PlayerRowItem(props: PlayerRowItemProps) {
  const barWidth = () =>
    props.topValue > 0 ? (props.row.totalValue / props.topValue) * 100 : 0;

  const classColor = () => getClassColor(props.row.className);

  return (
    <div
      onClick={props.onClick}
      style={{
        position: "relative",
        display: "grid",
        "grid-template-columns": gridCols(),
        padding: "3px 8px",
        "font-size": "12px",
        cursor: "pointer",
        "border-bottom": "1px solid rgba(255,255,255,0.03)",
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.background = "rgba(255,255,255,0.05)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = "transparent";
      }}
    >
      {/* Background bar */}
      <div
        style={{
          position: "absolute",
          left: "0",
          top: "0",
          bottom: "0",
          width: `${barWidth()}%`,
          background: `linear-gradient(90deg, ${classColor()}33, transparent)`,
          "pointer-events": "none",
          "z-index": "0",
        }}
      />

      {/* Content */}
      <div style={{ "z-index": "1", display: "flex", "align-items": "center", gap: "6px", "min-width": "0" }}>
        <span
          style={{
            width: "3px",
            height: "14px",
            "border-radius": "1px",
            background: classColor(),
            "flex-shrink": "0",
          }}
        />
        <span style={{
          overflow: "hidden",
          "text-overflow": "ellipsis",
          "white-space": "nowrap",
        }}>
          {props.row.name}
        </span>
        <span style={{ color: "#666", "font-size": "10px", "flex-shrink": "0" }}>
          {props.row.classSpecName !== "不明" ? props.row.classSpecName : ""}
        </span>
      </div>

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
      <Show when={showLucky()}>
        <span style={{ "text-align": "right", "z-index": "1", color: "#2ecc71" }}>
          {formatPct(props.row.luckyRate)}
        </span>
      </Show>
      <Show when={showHpm()}>
        <span style={{ "text-align": "right", "z-index": "1" }}>
          {props.row.hitsPerMinute.toFixed(1)}
        </span>
      </Show>
      <Show when={showScore()}>
        <span style={{ "text-align": "right", "z-index": "1", color: "#888" }}>
          {props.row.abilityScore > 0 ? formatNumber(props.row.abilityScore) : "-"}
        </span>
      </Show>
    </div>
  );
}

function gridCols(): string {
  let cols = "minmax(100px, 1.5fr) 70px 65px 45px";
  if (showCrit()) cols += " 50px";
  if (showLucky()) cols += " 50px";
  if (showHpm()) cols += " 50px";
  if (showScore()) cols += " 50px";
  return cols;
}
