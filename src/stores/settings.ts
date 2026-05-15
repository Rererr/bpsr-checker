import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createEffect, createSignal } from "solid-js";
import { persisted } from "../lib/persisted";

const [opacity, setOpacity] = persisted<number>("opacity", 0.85);
const [showCrit, setShowCrit] = persisted<boolean>("showCrit", true);
const [showLucky, setShowLucky] = persisted<boolean>("showLucky", true);
const [showHpm, setShowHpm] = persisted<boolean>("showHpm", false);
const [showScore, setShowScore] = persisted<boolean>("showScore", false);
const [showCritValue, setShowCritValue] = persisted<boolean>("showCritValue", false);
const [showLuckyValue, setShowLuckyValue] = persisted<boolean>("showLuckyValue", false);
const [showHits, setShowHits] = persisted<boolean>("showHits", false);
const [copyTemplate, setCopyTemplate] = persisted<string>("copyTemplate", "{rank}. {name} ({class}) {dmg} / {dps} DPS ({pct})");
export const DEFAULT_NAME_TEMPLATE = "{name} {spec}({score} - {seasonLv} - {seasonStr})";
const [nameTemplate, setNameTemplate] = persisted<string>("nameTemplate", DEFAULT_NAME_TEMPLATE);
const [copySeparator, setCopySeparator] = persisted<string>("copySeparator", "\t");
const [combatExitSec, setCombatExitSec] = persisted<number>("combatExitSec", 8);
const [pollIntervalMs, setPollIntervalMs] = persisted<number>("pollIntervalMs", 200);
const [historyLimit, setHistoryLimit] = persisted<number>("historyLimit", 20);
const [timeSeriesSamples, setTimeSeriesSamples] = persisted<number>("timeSeriesSamples", 60);
const [timeSeriesIntervalMs, setTimeSeriesIntervalMs] = persisted<number>("timeSeriesIntervalMs", 1000);
const [alwaysOnTop, setAlwaysOnTop] = persisted<boolean>("alwaysOnTop", true);
try { localStorage.removeItem("bpsr.settings.clickThrough"); } catch {}
const [clickThrough, setClickThrough] = createSignal<boolean>(false);
const [fontSize, setFontSize] = persisted<number>("fontSize", 12);
const [highlightLocalPlayer, setHighlightLocalPlayer] = persisted<boolean>("highlightLocalPlayer", true);
const [privacyMaskNames, setPrivacyMaskNames] = persisted<boolean>("privacyMaskNames", false);
const [startupTab, setStartupTab] = persisted<string>("startupTab", "dps");
const [rememberWindowPos, setRememberWindowPos] = persisted<boolean>("rememberWindowPos", true);
const [showHeaderSparkline, setShowHeaderSparkline] = persisted<boolean>("showHeaderSparkline", false);
const [graphPlayerCount, setGraphPlayerCount] = persisted<number>("graphPlayerCount", 3);
const [graphForLocalPlayer, setGraphForLocalPlayer] = persisted<boolean>("graphForLocalPlayer", true);
const [selectedUid, setSelectedUid] = persisted<number | null>("selectedUid", null);
const [threeMinDurationSec, setThreeMinDurationSec] = persisted<number>("threeMinDurationSec", 180);
const [threeMinAutoOpen, setThreeMinAutoOpen] = persisted<boolean>("threeMinAutoOpen", true);
const [abbreviateScores, setAbbreviateScores] = persisted<boolean>("abbreviateScores", false);
const [showBuffOverlay, setShowBuffOverlay] = persisted<boolean>("showBuffOverlay", false);

export {
  opacity, setOpacity,
  showCrit, setShowCrit,
  showLucky, setShowLucky,
  showHpm, setShowHpm,
  showScore, setShowScore,
  showCritValue, setShowCritValue,
  showLuckyValue, setShowLuckyValue,
  showHits, setShowHits,
  copyTemplate, setCopyTemplate,
  nameTemplate, setNameTemplate,
  copySeparator, setCopySeparator,
  combatExitSec, setCombatExitSec,
  pollIntervalMs, setPollIntervalMs,
  historyLimit, setHistoryLimit,
  timeSeriesSamples, setTimeSeriesSamples,
  timeSeriesIntervalMs, setTimeSeriesIntervalMs,
  alwaysOnTop, setAlwaysOnTop,
  clickThrough, setClickThrough,
  fontSize, setFontSize,
  highlightLocalPlayer, setHighlightLocalPlayer,
  privacyMaskNames, setPrivacyMaskNames,
  startupTab, setStartupTab,
  rememberWindowPos, setRememberWindowPos,
  showHeaderSparkline, setShowHeaderSparkline,
  graphPlayerCount, setGraphPlayerCount,
  graphForLocalPlayer, setGraphForLocalPlayer,
  selectedUid, setSelectedUid,
  threeMinDurationSec, setThreeMinDurationSec,
  threeMinAutoOpen, setThreeMinAutoOpen,
  abbreviateScores, setAbbreviateScores,
  showBuffOverlay, setShowBuffOverlay,
};

export function wireBackendSettings() {
  const [selectedUidReady, setSelectedUidReady] = createSignal(false);
  invoke<number | null>("get_selected_uid")
    .then((v) => {
      if (v !== selectedUid()) setSelectedUid(v);
      setSelectedUidReady(true);
    })
    .catch(() => setSelectedUidReady(true));
  createEffect(() => {
    const uid = selectedUid();
    if (!selectedUidReady()) return;
    invoke("set_selected_uid", { uid }).catch(() => {});
  });
  createEffect(() => { invoke("set_combat_exit_timeout", { secs: combatExitSec() }).catch(() => {}); });
  createEffect(() => { invoke("set_history_limit", { limit: historyLimit() }).catch(() => {}); });
  createEffect(() => {
    invoke("set_time_series_config", {
      samples: timeSeriesSamples(),
      intervalMs: timeSeriesIntervalMs(),
    }).catch(() => {});
  });
  createEffect(() => { invoke("set_always_on_top", { enabled: alwaysOnTop() }).catch(() => {}); });
  createEffect(() => { invoke("set_click_through", { enabled: clickThrough() }).catch(() => {}); });
  listen("click-through-disabled", () => setClickThrough(false));
}
