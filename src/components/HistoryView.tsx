import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import { t } from "../lib/i18n";
import { history, fetchHistory, clearHistory, type EncounterSnapshot, type PlayerRow } from "../stores/encounter";
import { formatNumber, formatDps, formatElapsed, getClassColor, maskPlayerName } from "../utils";
import { privacyMaskNames } from "../stores/settings";

export function HistoryView() {
  const [expandedId, setExpandedId] = createSignal<number | null>(null);

  createEffect(() => {
    fetchHistory();
    const interval = setInterval(fetchHistory, 1000);
    onCleanup(() => clearInterval(interval));
  });

  return (
    <div style={{ flex: "1", overflow: "auto", "font-size": "11px" }}>
      <div style={{
        display: "flex",
        "align-items": "center",
        "justify-content": "space-between",
        padding: "4px 8px",
        background: "rgba(0,0,0,0.2)",
        "border-bottom": "1px solid rgba(255,255,255,0.06)",
      }}>
        <span style={{ color: "#aaa" }}>{t("tab_history")}</span>
        <button
          onClick={() => clearHistory()}
          style={{
            padding: "2px 8px",
            border: "1px solid rgba(255,255,255,0.2)",
            "border-radius": "3px",
            background: "transparent",
            color: "#ccc",
            cursor: "pointer",
            "font-size": "10px",
          }}
        >
          {t("clear_history")}
        </button>
      </div>

      <Show when={history().length > 0} fallback={
        <div style={{ padding: "16px", color: "#555", "text-align": "center" }}>
          {t("no_history")}
        </div>
      }>
        <For each={history()}>
          {(snap) => (
            <HistoryItem
              snap={snap}
              expanded={expandedId() === snap.id}
              onToggle={() => setExpandedId(expandedId() === snap.id ? null : snap.id)}
            />
          )}
        </For>
      </Show>
    </div>
  );
}

function HistoryItem(props: { snap: EncounterSnapshot; expanded: boolean; onToggle: () => void }) {
  const snap = () => props.snap;

  return (
    <div style={{ "border-bottom": "1px solid rgba(255,255,255,0.05)" }}>
      <div
        onClick={props.onToggle}
        style={{
          display: "flex",
          "align-items": "center",
          gap: "10px",
          padding: "5px 8px",
          cursor: "pointer",
          background: props.expanded ? "rgba(255,255,255,0.05)" : "transparent",
        }}
      >
        <span style={{ color: "#888", "min-width": "36px" }}>{formatElapsed(snap().durationMs)}</span>
        <span style={{ color: "#4fc3f7", "min-width": "60px" }}>{formatDps(snap().totalDps)} DPS</span>
        <span style={{ color: "#aaa", "min-width": "60px" }}>{formatNumber(snap().totalDmg)}</span>
        <span style={{ color: "#666", flex: "1" }}>
          {snap().playerRows.length} {t("players_count")}
        </span>
        <span style={{ color: "#555" }}>{props.expanded ? "▲" : "▼"}</span>
      </div>

      <Show when={props.expanded}>
        <div style={{ padding: "0 8px 6px 8px", background: "rgba(0,0,0,0.15)" }}>
          <For each={snap().playerRows.slice(0, 5)}>
            {(row, i) => <SnapshotPlayerRow row={row} rank={i() + 1} topValue={snap().playerRows[0]?.totalValue ?? 1} />}
          </For>
          <Show when={snap().playerRows.length > 5}>
            <div style={{ color: "#555", "font-size": "10px", padding: "2px 0" }}>
              +{snap().playerRows.length - 5} {t("players_count")}
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
}

function SnapshotPlayerRow(props: { row: PlayerRow; rank: number; topValue: number }) {
  const row = () => props.row;
  const barPct = () => Math.max(0, Math.min(100, (row().totalValue / props.topValue) * 100));

  return (
    <div style={{
      position: "relative",
      display: "flex",
      "align-items": "center",
      gap: "6px",
      padding: "2px 0",
      overflow: "hidden",
    }}>
      <div style={{
        position: "absolute",
        left: "0",
        top: "0",
        bottom: "0",
        width: `${barPct()}%`,
        background: `${getClassColor(row().className)}22`,
        "border-radius": "2px",
        "pointer-events": "none",
      }} />
      <span style={{ color: "#555", "min-width": "14px", "z-index": "1" }}>{props.rank}.</span>
      <span style={{ color: getClassColor(row().className), "min-width": "80px", "z-index": "1", overflow: "hidden", "text-overflow": "ellipsis", "white-space": "nowrap" }}>
        {privacyMaskNames() ? maskPlayerName(row().name, row().uid) : row().name}
      </span>
      <span style={{ color: "#666", "min-width": "50px", "z-index": "1", "font-size": "10px" }}>{row().className}</span>
      <span style={{ color: "#4fc3f7", "min-width": "50px", "z-index": "1" }}>{formatDps(row().valuePerSec)}</span>
      <span style={{ color: "#aaa", "z-index": "1" }}>{row().valuePct.toFixed(1)}%</span>
    </div>
  );
}
