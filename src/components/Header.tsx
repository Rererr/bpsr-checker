import { createEffect, createResource, createSignal, onCleanup, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
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
import {
  pollIntervalMs,
  copyTemplate,
  showHeaderSparkline,
  alwaysOnTop,
  setAlwaysOnTop,
  opacity,
  selectedUid,
} from "../stores/settings";
import type { Tab } from "../App";

interface NameCacheEntry {
  name: string;
  classId: number | null;
  abilityScore: number | null;
}

const PIN_LABEL_BREAKPOINT = 560;

interface HeaderProps {
  tab: Tab;
  onTabChange: (tab: Tab) => void;
  onToggleSettings: () => void;
}

export function Header(props: HeaderProps) {
  createEffect(() => {
    const tab = props.tab;
    const ms = pollIntervalMs();
    const showSparkline = showHeaderSparkline();
    const interval = setInterval(() => {
      fetchDpsData();
      if (showSparkline) fetchTimeSeries();
      if (tab === "heal") fetchHealData();
      if (tab === "dps") fetchBossData(); // prefetch for boss tab
    }, ms);
    onCleanup(() => clearInterval(interval));
  });

  createEffect(() => {
    if (showHeaderSparkline()) fetchTimeSeries();
  });

  // selectedUid バッジ用の name_cache lookup
  const [badgeNameSource, setBadgeNameSource] = createSignal(selectedUid());
  const [badgeNameCache] = createResource<NameCacheEntry | null, number | null>(
    badgeNameSource,
    (uid) => {
      if (uid == null || uid === 0) return Promise.resolve(null);
      return invoke<NameCacheEntry | null>("lookup_name_cache", { uid }).catch(() => null);
    }
  );

  // selectedUid 変更時にソース更新、name_cache が空なら 3 秒後に再試行
  createEffect(() => {
    const uid = selectedUid();
    setBadgeNameSource(uid);
    if (uid != null && uid !== 0) {
      const retry = setTimeout(() => { setBadgeNameSource(null); }, 50);
      const retry2 = setTimeout(() => { setBadgeNameSource(uid); }, 3000);
      onCleanup(() => { clearTimeout(retry); clearTimeout(retry2); });
    }
  });

  const [windowWidth, setWindowWidth] = createSignal(window.innerWidth);
  const handleResize = () => setWindowWidth(window.innerWidth);
  window.addEventListener("resize", handleResize);
  onCleanup(() => window.removeEventListener("resize", handleResize));

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
        background: `rgba(0,0,0,${0.3 * opacity()})`,
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
        <Show when={selectedUid() != null && selectedUid() !== 0}>
          <span
            data-tauri-drag-region
            style={{
              color: "#4fc3f7",
              border: "1px solid rgba(79, 195, 247, 0.4)",
              "border-radius": "3px",
              padding: "1px 5px",
              "font-size": "10px",
              background: "rgba(79, 195, 247, 0.08)",
              "white-space": "nowrap",
            }}
          >
            {badgeNameCache()?.name ?? t("selected_uid_badge_unknown")} #{String(selectedUid()).slice(-4)}
          </span>
        </Show>
      </div>

      {/* Controls */}
      <div style={{ display: "flex", gap: "4px", "align-items": "center" }}>
        <Show
          when={windowWidth() >= PIN_LABEL_BREAKPOINT}
          fallback={
            <button
              onClick={() => setAlwaysOnTop(!alwaysOnTop())}
              aria-pressed={alwaysOnTop()}
              style={{
                ...controlBtnStyle(),
                display: "flex",
                "align-items": "center",
                padding: "2px 5px",
                color: alwaysOnTop() ? "#4fc3f7" : "#888",
                background: alwaysOnTop() ? "rgba(79, 195, 247, 0.15)" : "transparent",
                "border-color": alwaysOnTop() ? "rgba(79, 195, 247, 0.45)" : "rgba(255,255,255,0.2)",
              }}
              title={t("always_on_top")}
            >
              <PinIcon active={alwaysOnTop()} />
            </button>
          }
        >
          <label
            style={{
              display: "flex",
              "align-items": "center",
              gap: "4px",
              padding: "2px 6px",
              border: `1px solid ${alwaysOnTop() ? "rgba(79, 195, 247, 0.45)" : "rgba(255,255,255,0.2)"}`,
              "border-radius": "3px",
              background: alwaysOnTop() ? "rgba(79, 195, 247, 0.15)" : "transparent",
              color: alwaysOnTop() ? "#ddd" : "#888",
              cursor: "pointer",
              "font-size": "10px",
              "line-height": "1",
            }}
            title={t("always_on_top")}
          >
            <input
              type="checkbox"
              checked={alwaysOnTop()}
              onChange={(e) => setAlwaysOnTop(e.currentTarget.checked)}
              style={{ width: "11px", height: "11px", margin: "0", cursor: "pointer" }}
            />
            <PinIcon active={alwaysOnTop()} />
            <span>{t("always_on_top")}</span>
          </label>
        </Show>
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
          onClick={() => getCurrentWindow().minimize()}
          style={controlBtnStyle()}
          title={t("minimize")}
        >
          −
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

function PinIcon(props: { active: boolean }) {
  return (
    <svg
      width="11"
      height="11"
      viewBox="0 0 16 16"
      fill="currentColor"
      style={{
        display: "block",
        transform: props.active ? "rotate(0deg)" : "rotate(45deg)",
        transition: "transform 0.15s",
      }}
      aria-hidden="true"
    >
      <path d="M9.828.722a.5.5 0 0 1 .354.146l4.95 4.95a.5.5 0 0 1 0 .707c-.48.48-1.072.588-1.503.588-.177 0-.335-.018-.46-.039l-3.134 3.134a5.927 5.927 0 0 1 .16 1.013c.046.702-.032 1.687-.72 2.375a.5.5 0 0 1-.707 0l-2.829-2.828-3.182 3.182c-.195.195-1.219.902-1.414.707-.195-.195.512-1.22.707-1.414l3.182-3.182-2.828-2.829a.5.5 0 0 1 0-.707c.688-.688 1.673-.767 2.375-.72a5.922 5.922 0 0 1 1.013.16l3.134-3.133a2.772 2.772 0 0 1-.04-.461c0-.43.108-1.022.589-1.503a.5.5 0 0 1 .353-.146z" />
    </svg>
  );
}
