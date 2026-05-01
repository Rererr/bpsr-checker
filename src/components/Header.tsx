import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { t } from "../lib/i18n";
import {
  header,
  fetchDpsData,
  fetchHealData,
  fetchBossData,
  dpsPlayers,
  healPlayers,
  resetEncounter,
  togglePause,
  timeSeries,
  fetchTimeSeries,
} from "../stores/encounter";
import { Sparkline } from "./Sparkline";
import { formatDps, formatNumber, formatElapsed, formatRowAsText } from "../utils";
import { pollIntervalMs, copyTemplate, showHeaderSparkline } from "../stores/settings";
import type { Tab } from "../App";

interface HeaderProps {
  tab: Tab;
  onTabChange: (tab: Tab) => void;
  onToggleSettings: () => void;
}

export function Header(props: HeaderProps) {
  createEffect(() => {
    const tab = props.tab;
    const ms = pollIntervalMs();
    const interval = setInterval(() => {
      fetchDpsData();
      fetchTimeSeries();
      if (tab === "heal") fetchHealData();
      if (tab === "dps") fetchBossData(); // prefetch for boss tab
    }, ms);
    onCleanup(() => clearInterval(interval));
  });

  const h = () => header();

  const [copiedAll, setCopiedAll] = createSignal(false);
  const handleCopyAll = async () => {
    const rows = props.tab === "heal" ? healPlayers().playerRows : dpsPlayers().playerRows;
    if (rows.length === 0) return;
    const tpl = copyTemplate();
    const text = rows.map((r, i) => formatRowAsText(r, i + 1, tpl)).join("\n");
    await navigator.clipboard.writeText(text);
    setCopiedAll(true);
    setTimeout(() => setCopiedAll(false), 800);
  };

  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        gap: "8px",
        padding: "4px 8px",
        background: "rgba(0,0,0,0.3)",
        "border-bottom": "1px solid rgba(255,255,255,0.1)",
        "font-size": "12px",
        "user-select": "none",
        cursor: "default",
      }}
      data-tauri-drag-region
    >
      {/* Tabs */}
      <div style={{ display: "flex", gap: "2px" }}>
        {(["dps", "heal", "history"] as Tab[]).map((tab) => (
          <button
            onClick={() => props.onTabChange(tab)}
            style={{
              padding: "2px 8px",
              border: "none",
              "border-radius": "3px",
              background: props.tab === tab ? "rgba(255,255,255,0.15)" : "transparent",
              color: props.tab === tab ? "#fff" : "#aaa",
              cursor: "pointer",
              "font-size": "11px",
            }}
          >
            {tab === "dps" ? t("tab_dps") : tab === "heal" ? t("tab_heal") : t("tab_history")}
          </button>
        ))}
      </div>

      {/* Stats */}
      <div data-tauri-drag-region style={{ flex: "1", display: "flex", gap: "12px", "align-items": "center" }}>
        <span data-tauri-drag-region style={{ color: "#4fc3f7" }}>
          {formatDps(h().totalDps)} DPS
        </span>
        <span data-tauri-drag-region style={{ color: "#aaa" }}>
          {formatNumber(h().totalDmg)}
        </span>
        <span data-tauri-drag-region style={{ color: "#888" }}>
          {formatElapsed(h().elapsedMs)}
        </span>
        <Show when={showHeaderSparkline()}>
          <span data-tauri-drag-region style={{ "margin-left": "4px", display: "flex", "align-items": "center" }}>
            <Sparkline points={timeSeries()} width={100} height={18} />
          </span>
        </Show>
      </div>

      {/* Controls */}
      <div style={{ display: "flex", gap: "4px" }}>
        <button
          onClick={togglePause}
          style={controlBtnStyle()}
          title={t("pause")}
        >
          ||
        </button>
        <button
          onClick={resetEncounter}
          style={controlBtnStyle()}
          title={t("reset")}
        >
          R
        </button>
        <button
          onClick={handleCopyAll}
          style={{ ...controlBtnStyle(), color: copiedAll() ? "#2ecc71" : "#ccc" }}
          title={t("copy_all")}
        >
          {copiedAll() ? "✓" : "C"}
        </button>
        <button
          onClick={props.onToggleSettings}
          style={controlBtnStyle()}
          title={t("settings")}
        >
          S
        </button>
        <button
          onClick={() => invoke("quit_app")}
          style={controlBtnStyle()}
          title={t("quit")}
        >
          ×
        </button>
      </div>
    </div>
  );
}

function controlBtnStyle() {
  return {
    padding: "2px 6px",
    border: "1px solid rgba(255,255,255,0.2)",
    "border-radius": "3px",
    background: "transparent",
    color: "#ccc",
    cursor: "pointer",
    "font-size": "10px",
    "line-height": "1",
  };
}
