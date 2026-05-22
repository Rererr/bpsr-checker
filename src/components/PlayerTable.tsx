import { For, Show, createSignal } from "solid-js";
import { t } from "../lib/i18n";
import { dpsPlayers, healPlayers, bossPlayers, takenPlayers } from "../stores/encounter";
import {
  showCrit, showLucky, showHpm, showScore,
  showCritValue, showLuckyValue, showHits,
  copyTemplate, nameTemplate, privacyMaskNames, highlightLocalPlayer,
  graphPlayerCount, graphForLocalPlayer,
  selectedUid, abbreviateScores,
} from "../stores/settings";
import { formatNumber, formatDps, formatPct, formatScore, getClassColor, formatRowAsText, maskPlayerName } from "../utils";
import { Sparkline } from "./Sparkline";
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
      case "taken": return takenPlayers();
      default: return dpsPlayers();
    }
  };

  const hideSparkline = () => props.tab === "taken";

  return (
    <div style={{ flex: "1", overflow: "auto" }}>
      {/* Column headers */}
      <div
        style={{
          display: "grid",
          "grid-template-columns": gridCols(hideSparkline()),
          padding: "2px 8px",
          "font-size": "10px",
          color: "#888",
          "border-bottom": "1px solid rgba(255,255,255,0.05)",
          "user-select": "none",
        }}
      >
        <span>{t("player")}</span>
        <Show when={!hideSparkline() && hasSparklineColumn()}>
          <span />
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
        <Show when={showScore()}>
          <span style={{ "text-align": "right" }}>{t("score")}</span>
        </Show>
      </div>

      {/* Rows */}
      <Show
        when={data().playerRows.length > 0}
        fallback={
          <div style={{ padding: "20px", "text-align": "center", color: "#666" }}>
            <Show
              when={selectedUid() != null}
              fallback={t("no_data")}
            >
              {`UID #${String(selectedUid()).slice(-4)} ${t("selected_uid_filter_waiting")}`}
            </Show>
          </div>
        }
      >
        <For each={data().playerRows}>
          {(row, index) => {
            const isLocal = row.uid === data().localPlayerUid;
            const nonLocalRankAbove = data().playerRows
              .slice(0, index())
              .filter((r) => r.uid !== data().localPlayerUid).length;
            const showSparkline = !hideSparkline() && (isLocal
              ? graphForLocalPlayer()
              : nonLocalRankAbove < graphPlayerCount());
            return (
              <PlayerRowItem
                row={row}
                rank={index() + 1}
                topValue={data().topValue}
                isLocal={isLocal}
                showSparkline={showSparkline}
                hideSpark={hideSparkline()}
                onClick={() => props.onSelectPlayer(row.uid)}
              />
            );
          }}
        </For>
      </Show>
    </div>
  );
}

interface PlayerRowItemProps {
  row: PlayerRow;
  rank: number;
  topValue: number;
  isLocal: boolean;
  showSparkline: boolean;
  hideSpark: boolean;
  onClick: () => void;
}

function PlayerRowItem(props: PlayerRowItemProps) {
  const barWidth = () =>
    props.topValue > 0 ? (props.row.totalValue / props.topValue) * 100 : 0;

  const classColor = () => getClassColor(props.row.className);

  const [copied, setCopied] = createSignal(false);

  const handleCopy = async (e: MouseEvent) => {
    e.stopPropagation();
    const text = formatRowAsText(props.row, props.rank, copyTemplate(), abbreviateScores());
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 800);
  };

  return (
    <div
      class="player-row"
      onClick={props.onClick}
      style={{
        position: "relative",
        display: "grid",
        "grid-template-columns": gridCols(props.hideSpark),
        padding: "3px 8px",
        "font-size": "12px",
        cursor: "pointer",
        "border-bottom": "1px solid rgba(255,255,255,0.03)",
        "box-shadow": props.isLocal && highlightLocalPlayer() ? "inset 3px 0 0 #ffd700" : "none",
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.background = "rgba(255,255,255,0.05)";
        const btn = e.currentTarget.querySelector<HTMLElement>(".copy-btn");
        if (btn) btn.style.opacity = "1";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = "transparent";
        const btn = e.currentTarget.querySelector<HTMLElement>(".copy-btn");
        if (btn) btn.style.opacity = "0";
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
          {formatRowAsText(
            privacyMaskNames()
              ? { ...props.row, name: maskPlayerName(props.row.name, props.row.uid) }
              : props.row,
            props.rank,
            nameTemplate(),
            abbreviateScores(),
          )}
        </span>
      </div>

      <Show when={!props.hideSpark && hasSparklineColumn()}>
        <span style={{ "z-index": "1", display: "flex", "align-items": "center", "justify-content": "center" }}>
          <Show when={props.showSparkline}>
            <Sparkline
              points={props.row.timeSeries}
              width={60}
              height={14}
              color={classColor()}
            />
          </Show>
        </span>
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
      <Show when={showScore()}>
        <span style={{ "text-align": "right", "z-index": "1", color: "#888" }}>
          {props.row.abilityScore > 0 ? formatScore(props.row.abilityScore, abbreviateScores()) : "-"}
        </span>
      </Show>

      {/* Hover copy button */}
      <button
        class="copy-btn"
        onClick={handleCopy}
        title={t("copy_row")}
        style={{
          position: "absolute",
          right: "4px",
          top: "50%",
          transform: "translateY(-50%)",
          "z-index": "2",
          opacity: "0",
          transition: "opacity 0.15s",
          padding: "1px 5px",
          border: "1px solid rgba(255,255,255,0.2)",
          "border-radius": "3px",
          background: "rgba(0,0,0,0.6)",
          color: copied() ? "#2ecc71" : "#ccc",
          cursor: "pointer",
          "font-size": "10px",
          "line-height": "1.4",
        }}
      >
        {copied() ? "✓" : "C"}
      </button>
    </div>
  );
}

const hasSparklineColumn = () => graphPlayerCount() > 0 || graphForLocalPlayer();

function gridCols(hideSpark: boolean): string {
  let cols = "minmax(80px, 1.2fr)";
  if (!hideSpark && hasSparklineColumn()) cols += " 64px";
  cols += " 70px 65px 45px";
  if (showCrit()) cols += " 50px";
  if (showCritValue()) cols += " 50px";
  if (showLucky()) cols += " 50px";
  if (showLuckyValue()) cols += " 50px";
  if (showHits()) cols += " 50px";
  if (showHpm()) cols += " 50px";
  if (showScore()) cols += " 50px";
  return cols;
}
