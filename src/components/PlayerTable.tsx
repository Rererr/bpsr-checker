import { createSignal, For, Show } from "solid-js";
import { t } from "../lib/i18n";
import { dpsPlayers, healPlayers, bossPlayers, takenPlayers } from "../stores/encounter";
import { isWatched, toggleWatch } from "../stores/watchlist";
import {
  showCrit, showLucky, showHpm, showScore,
  showCritValue, showLuckyValue, showHits,
  nameTemplate, privacyMaskNames, highlightLocalPlayer,
  graphPlayerCount, graphForLocalPlayer,
  selectedUid, abbreviateScores, compactSplitMode,
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
      <Show
        when={compactSplitMode()}
        fallback={
          <NormalLayout
            data={data()}
            hideSparkline={hideSparkline()}
            onSelectPlayer={props.onSelectPlayer}
          />
        }
      >
        <CompactSplitLayout
          data={data()}
          onSelectPlayer={props.onSelectPlayer}
        />
      </Show>
    </div>
  );
}

// ─── Normal layout ────────────────────────────────────────────────────────────

interface NormalLayoutProps {
  data: PlayersWindow;
  hideSparkline: boolean;
  onSelectPlayer: (uid: number) => void;
}

function NormalLayout(props: NormalLayoutProps) {
  return (
    <>
      {/* Column headers */}
      <div
        style={{
          display: "grid",
          "grid-template-columns": gridCols(props.hideSparkline),
          padding: "2px 8px",
          "font-size": "10px",
          color: "#888",
          "border-bottom": "1px solid rgba(255,255,255,0.05)",
          "user-select": "none",
        }}
      >
        <span>{t("player")}</span>
        <Show when={!props.hideSparkline && hasSparklineColumn()}>
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
        <span /> {/* ウォッチボタン列 */}
      </div>

      {/* Rows */}
      <Show
        when={props.data.playerRows.length > 0}
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
        <For each={props.data.playerRows}>
          {(row, index) => {
            const isLocal = row.uid === props.data.localPlayerUid;
            const nonLocalRankAbove = props.data.playerRows
              .slice(0, index())
              .filter((r) => r.uid !== props.data.localPlayerUid).length;
            const showSparkline = !props.hideSparkline && (isLocal
              ? graphForLocalPlayer()
              : nonLocalRankAbove < graphPlayerCount());
            return (
              <PlayerRowItem
                row={row}
                rank={index() + 1}
                topValue={props.data.topValue}
                isLocal={isLocal}
                showSparkline={showSparkline}
                hideSpark={props.hideSparkline}
                compact={false}
                onClick={() => props.onSelectPlayer(row.uid)}
              />
            );
          }}
        </For>
      </Show>
    </>
  );
}

// ─── Compact split layout ─────────────────────────────────────────────────────

const COMPACT_GRID = "minmax(60px, 1fr) 62px 58px";

interface CompactSplitLayoutProps {
  data: PlayersWindow;
  onSelectPlayer: (uid: number) => void;
}

function CompactColumnHeader() {
  return (
    <div
      style={{
        display: "grid",
        "grid-template-columns": COMPACT_GRID,
        padding: "2px 6px",
        "font-size": "10px",
        color: "#888",
        "border-bottom": "1px solid rgba(255,255,255,0.05)",
        "user-select": "none",
      }}
    >
      <span>{t("player")}</span>
      <span style={{ "text-align": "right" }}>{t("damage")}</span>
      <span style={{ "text-align": "right" }}>{t("dps")}</span>
    </div>
  );
}

function CompactSplitLayout(props: CompactSplitLayoutProps) {
  const rows = () => props.data.playerRows;
  const half = () => Math.ceil(rows().length / 2);
  const leftRows = () => rows().slice(0, half());
  const rightRows = () => rows().slice(half());

  const fallback = (
    <div style={{ padding: "20px", "text-align": "center", color: "#666" }}>
      <Show when={selectedUid() != null} fallback={t("no_data")}>
        {`UID #${String(selectedUid()).slice(-4)} ${t("selected_uid_filter_waiting")}`}
      </Show>
    </div>
  );

  return (
    <Show when={rows().length > 0} fallback={fallback}>
      <div style={{ display: "grid", "grid-template-columns": "1fr 1fr", gap: "0 4px" }}>
        {/* Left column */}
        <div>
          <CompactColumnHeader />
          <For each={leftRows()}>
            {(row, index) => (
              <PlayerRowItem
                row={row}
                rank={index() + 1}
                topValue={props.data.topValue}
                isLocal={row.uid === props.data.localPlayerUid}
                showSparkline={false}
                hideSpark={true}
                compact={true}
                onClick={() => props.onSelectPlayer(row.uid)}
              />
            )}
          </For>
        </div>
        {/* Right column */}
        <div>
          <CompactColumnHeader />
          <For each={rightRows()}>
            {(row, index) => (
              <PlayerRowItem
                row={row}
                rank={half() + index() + 1}
                topValue={props.data.topValue}
                isLocal={row.uid === props.data.localPlayerUid}
                showSparkline={false}
                hideSpark={true}
                compact={true}
                onClick={() => props.onSelectPlayer(row.uid)}
              />
            )}
          </For>
        </div>
      </div>
    </Show>
  );
}

// ─── Row item ─────────────────────────────────────────────────────────────────

interface PlayerRowItemProps {
  row: PlayerRow;
  rank: number;
  topValue: number;
  isLocal: boolean;
  showSparkline: boolean;
  hideSpark: boolean;
  compact: boolean;
  onClick: () => void;
}

function PlayerRowItem(props: PlayerRowItemProps) {
  const barWidth = () =>
    props.topValue > 0 ? (props.row.totalValue / props.topValue) * 100 : 0;

  const classColor = () => getClassColor(props.row.className);

  const rowGridCols = () =>
    props.compact ? COMPACT_GRID : gridCols(props.hideSpark);

  const rowPadding = () => props.compact ? "2px 6px" : "3px 8px";

  const [hovered, setHovered] = createSignal(false);
  const watched = () => isWatched(props.row.uid);

  return (
    <div
      class="player-row"
      role="button"
      tabIndex={0}
      onClick={props.onClick}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); props.onClick(); } }}
      style={{
        position: "relative",
        display: "grid",
        "grid-template-columns": rowGridCols(),
        padding: rowPadding(),
        "font-size": "12px",
        cursor: "pointer",
        "border-bottom": "1px solid rgba(255,255,255,0.03)",
        "box-shadow": [
          props.isLocal && highlightLocalPlayer() ? "inset 3px 0 0 #ffd700" : "",
          watched() ? "inset -3px 0 0 #4fc3f7" : "",
        ].filter(Boolean).join(", ") || "none",
      }}
      onMouseEnter={(e) => { e.currentTarget.style.background = "rgba(255,255,255,0.05)"; setHovered(true); }}
      onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; setHovered(false); }}
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

      {/* Name cell */}
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

      {/* Sparkline (normal mode only) */}
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

      {/* Damage */}
      <span style={{ "text-align": "right", "z-index": "1" }}>
        {formatNumber(props.row.totalValue)}
      </span>

      {/* DPS */}
      <span style={{ "text-align": "right", "z-index": "1", color: "#4fc3f7" }}>
        {formatDps(props.row.valuePerSec)}
      </span>

      {/* Normal-only columns */}
      <Show when={!props.compact}>
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
        {/* ウォッチボタン — グリッド末尾列に配置 */}
        <div
          style={{
            display: "flex",
            "align-items": "center",
            "justify-content": "center",
            "z-index": "1",
          }}
        >
          <button
            onClick={(e) => { e.stopPropagation(); toggleWatch(props.row.uid); }}
            title={watched() ? t("unwatch") : "バフタイマーでウォッチ"}
            style={{
              background: "none",
              border: "none",
              padding: "2px",
              cursor: "pointer",
              color: watched() ? "#4fc3f7" : "rgba(255,255,255,0.4)",
              opacity: watched() ? 1 : (hovered() ? 1 : 0.3),
              display: "flex",
              "align-items": "center",
              transition: "opacity 0.15s",
            }}
          >
            <WatchIcon pinned={watched()} />
          </button>
        </div>
      </Show>
    </div>
  );
}

// ─── Icons ────────────────────────────────────────────────────────────────────

function WatchIcon(props: { pinned: boolean }) {
  return (
    <svg
      width="11"
      height="11"
      viewBox="0 0 16 16"
      fill="currentColor"
      style={{
        display: "block",
        transform: props.pinned ? "rotate(0deg)" : "rotate(45deg)",
        transition: "transform 0.15s",
      }}
      aria-hidden="true"
    >
      <path d="M9.828.722a.5.5 0 0 1 .354.146l4.95 4.95a.5.5 0 0 1 0 .707c-.48.48-1.072.588-1.503.588-.177 0-.335-.018-.46-.039l-3.134 3.134a5.927 5.927 0 0 1 .16 1.013c.046.702-.032 1.687-.72 2.375a.5.5 0 0 1-.707 0l-2.829-2.828-3.182 3.182c-.195.195-1.219.902-1.414.707-.195-.195.512-1.22.707-1.414l3.182-3.182-2.828-2.829a.5.5 0 0 1 0-.707c.688-.688 1.673-.767 2.375-.72a5.922 5.922 0 0 1 1.013.16l3.134-3.133a2.772 2.772 0 0 1-.04-.461c0-.43.108-1.022.589-1.503a.5.5 0 0 1 .353-.146z" />
    </svg>
  );
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

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
  cols += " 16px"; // ウォッチボタン列
  return cols;
}
