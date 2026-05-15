import { Show, createResource, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { t, locale, setLocale } from "../lib/i18n";
import type { Locale } from "../lib/i18n";
import {
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
  DEFAULT_NAME_TEMPLATE,
  pollIntervalMs, setPollIntervalMs,
  combatExitSec, setCombatExitSec,
  historyLimit, setHistoryLimit,
  timeSeriesSamples, setTimeSeriesSamples,
  timeSeriesIntervalMs, setTimeSeriesIntervalMs,
  clickThrough, setClickThrough,
  fontSize, setFontSize,
  highlightLocalPlayer, setHighlightLocalPlayer,
  privacyMaskNames, setPrivacyMaskNames,
  startupTab, setStartupTab,
  showHeaderSparkline, setShowHeaderSparkline,
  graphPlayerCount, setGraphPlayerCount,
  graphForLocalPlayer, setGraphForLocalPlayer,
  selectedUid, setSelectedUid,
  threeMinDurationSec, setThreeMinDurationSec,
  threeMinAutoOpen, setThreeMinAutoOpen,
  abbreviateScores, setAbbreviateScores,
  showBuffOverlay, setShowBuffOverlay,
} from "../stores/settings";
import { clearHistory, dpsPlayers, header } from "../stores/encounter";
import { formatRowAsText } from "../utils";
import type { PlayerRow } from "../stores/encounter";

interface NameCacheEntry {
  name: string;
  classId: number | null;
  abilityScore: number | null;
}

const SAMPLE_ROW: PlayerRow = {
  uid: 0,
  name: "Sample",
  className: "ストームブレイド",
  classSpecName: "炎",
  abilityScore: 12345,
  seasonLevel: 38,
  seasonStrength: 8200,
  totalValue: 1234567,
  valuePerSec: 45678,
  valuePct: 35.5,
  critRate: 42.3,
  critValueRate: 18.7,
  luckyRate: 5.5,
  luckyValueRate: 2.1,
  hits: 124,
  hitsPerMinute: 78.5,
  timeSeries: [],
};

const sectionHeaderStyle = {
  color: "#aaa",
  "font-weight": "bold",
  "font-size": "10px",
  "text-transform": "uppercase",
  "letter-spacing": "0.05em",
  cursor: "pointer",
};

const sectionStyle = {
  display: "flex",
  "flex-direction": "column" as const,
  gap: "6px",
};

export function SettingsPanel() {
  const [uidInputValue, setUidInputValue] = createSignal(selectedUid()?.toString() ?? "");

  const [nameCache] = createResource<NameCacheEntry | null, number | null>(
    selectedUid,
    (uid) => {
      if (uid == null || uid === 0) return Promise.resolve(null);
      return invoke<NameCacheEntry | null>("lookup_name_cache", { uid }).catch(() => null);
    }
  );

  const isInCombat = () => header().elapsedMs > 0 && header().totalDmg > 0;

  const commitUidInput = (raw: string) => {
    const trimmed = raw.trim();
    let next: number | null = null;
    if (trimmed !== "") {
      const parsed = Number(trimmed);
      if (isFinite(parsed) && !isNaN(parsed) && parsed > 0) {
        next = parsed;
      }
    }
    if (next === selectedUid()) {
      setUidInputValue(next == null ? "" : String(next));
      return;
    }
    if (isInCombat() && !window.confirm(t("selected_uid_change_confirm"))) {
      setUidInputValue(selectedUid() == null ? "" : String(selectedUid()));
      return;
    }
    setSelectedUid(next);
    setUidInputValue(next == null ? "" : String(next));
  };

  const handleClear = () => {
    if (selectedUid() == null) return;
    if (isInCombat() && !window.confirm(t("selected_uid_change_confirm"))) return;
    setSelectedUid(null);
    setUidInputValue("");
  };

  const handleCandidateClick = (uid: number) => {
    if (uid === selectedUid()) return;
    if (isInCombat() && !window.confirm(t("selected_uid_change_confirm"))) return;
    setSelectedUid(uid);
    setUidInputValue(String(uid));
  };

  return (
    <div
      style={{
        padding: "8px 12px",
        background: "rgba(0,0,0,0.4)",
        "border-bottom": "1px solid rgba(255,255,255,0.1)",
        display: "flex",
        "flex-direction": "column",
        gap: "8px",
        "font-size": "11px",
      }}
    >
      <details open>
        <summary style={sectionHeaderStyle}>{t("settings_character")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>
          {/* UID input */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("selected_uid_label")}</span>
            <input
              type="text"
              inputmode="numeric"
              value={uidInputValue()}
              onInput={(e) => setUidInputValue(e.currentTarget.value)}
              onBlur={(e) => commitUidInput(e.currentTarget.value)}
              onKeyDown={(e) => { if (e.key === "Enter") { e.currentTarget.blur(); } }}
              placeholder="—"
              style={{
                width: "100px",
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            />
            <button
              onClick={handleClear}
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
              {t("selected_uid_clear")}
            </button>
          </div>

          {/* Resolved name */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }} />
            <span style={{ color: nameCache()?.name ? "#ddd" : "#555", "font-size": "11px" }}>
              {nameCache()?.name ?? t("selected_char_name_unknown")}
            </span>
          </div>

          {/* Candidates */}
          <Show when={dpsPlayers().playerRows.length > 0}>
            <div style={{ display: "flex", "align-items": "flex-start", gap: "6px" }}>
              <span style={{ color: "#777", "font-size": "10px", "white-space": "nowrap", "padding-top": "2px" }}>
                {t("selected_uid_candidates_label")}
              </span>
              <div style={{ display: "flex", "flex-wrap": "wrap", gap: "4px" }}>
                {dpsPlayers().playerRows.map((row) => {
                  const isSelected = () => row.uid === selectedUid();
                  const uid4 = String(row.uid).slice(-4);
                  return (
                    <button
                      onClick={() => handleCandidateClick(row.uid)}
                      style={{
                        padding: "1px 6px",
                        border: isSelected() ? "1px solid #4fc3f7" : "1px solid rgba(255,255,255,0.2)",
                        "border-radius": "3px",
                        background: isSelected() ? "rgba(79,195,247,0.15)" : "transparent",
                        color: isSelected() ? "#4fc3f7" : "#bbb",
                        cursor: "pointer",
                        "font-size": "10px",
                        "font-weight": isSelected() ? "bold" : "normal",
                        "white-space": "nowrap",
                      }}
                    >
                      {row.name} #{uid4}
                    </button>
                  );
                })}
              </div>
            </div>
          </Show>

          {/* Hint */}
          <div style={{ color: "#555", "font-size": "10px" }}>
            {t("selected_uid_input_hint")}
          </div>

          {/* Warning */}
          <div style={{ color: "#b07040", "font-size": "10px" }}>
            {t("selected_uid_change_warning")}
          </div>
        </div>
      </details>

      <details open>
        <summary style={sectionHeaderStyle}>{t("settings_display")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>
          {/* Opacity */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "60px" }}>{t("transparency")}</span>
            <input
              type="range"
              min="0.3"
              max="1"
              step="0.05"
              value={opacity()}
              onInput={(e) => setOpacity(parseFloat(e.currentTarget.value))}
              style={{ flex: "1" }}
            />
            <span style={{ color: "#888", width: "30px", "text-align": "right" }}>
              {Math.round(opacity() * 100)}%
            </span>
          </div>

          {/* Language */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "60px" }}>{t("language")}</span>
            <select
              value={locale()}
              onChange={(e) => setLocale(e.currentTarget.value as Locale)}
              style={{
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            >
              <option value="ja">日本語</option>
              <option value="en">English</option>
            </select>
          </div>

          {/* Column toggles */}
          <div style={{ display: "flex", "align-items": "flex-start", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "60px", "padding-top": "1px" }}>{t("columns")}</span>
            <div style={{ display: "flex", "flex-wrap": "wrap", gap: "6px 10px", flex: "1" }}>
              <Toggle label={t("crit_rate")} value={showCrit()} onChange={setShowCrit} />
              <Toggle label={t("crit_value")} value={showCritValue()} onChange={setShowCritValue} />
              <Toggle label={t("lucky_rate")} value={showLucky()} onChange={setShowLucky} />
              <Toggle label={t("lucky_value")} value={showLuckyValue()} onChange={setShowLuckyValue} />
              <Toggle label={t("hits")} value={showHits()} onChange={setShowHits} />
              <Toggle label={t("hpm")} value={showHpm()} onChange={setShowHpm} />
              <Toggle label={t("score")} value={showScore()} onChange={setShowScore} />
              <Toggle label={t("abbreviate_scores")} value={abbreviateScores()} onChange={setAbbreviateScores} />
            </div>
          </div>

          {/* Name column template */}
          <div style={{ display: "flex", "align-items": "flex-start", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "60px", "padding-top": "2px" }}>{t("name_template")}</span>
            <div style={{ flex: "1", display: "flex", "flex-direction": "column", gap: "3px" }}>
              <div style={{ display: "flex", gap: "4px" }}>
                <textarea
                  rows="2"
                  value={nameTemplate()}
                  onInput={(e) => setNameTemplate(e.currentTarget.value)}
                  style={{
                    flex: "1",
                    background: "rgba(255,255,255,0.1)",
                    border: "1px solid rgba(255,255,255,0.2)",
                    color: "#ddd",
                    "border-radius": "3px",
                    padding: "3px 5px",
                    "font-size": "11px",
                    "font-family": "monospace",
                    resize: "vertical",
                  }}
                />
                <button
                  onClick={() => setNameTemplate(DEFAULT_NAME_TEMPLATE)}
                  title={t("name_template_reset")}
                  style={{
                    padding: "2px 8px",
                    border: "1px solid rgba(255,255,255,0.2)",
                    "border-radius": "3px",
                    background: "transparent",
                    color: "#ccc",
                    cursor: "pointer",
                    "font-size": "11px",
                    "white-space": "nowrap",
                    "align-self": "flex-start",
                  }}
                >
                  {t("name_template_reset")}
                </button>
              </div>
              <div style={{ color: "#666", "font-size": "10px", "font-family": "monospace" }}>
                {"{name} {class} {spec} {score} {seasonLv} {seasonStr}"}
              </div>
              <pre style={{
                margin: "0",
                padding: "3px 5px",
                background: "rgba(0,0,0,0.3)",
                "border-radius": "3px",
                color: "#ddd",
                "font-size": "11px",
                "white-space": "pre-wrap",
                "word-break": "break-all",
              }}>
                {formatRowAsText(SAMPLE_ROW, 1, nameTemplate(), abbreviateScores())}
              </pre>
            </div>
          </div>

          {/* Header total sparkline */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "60px" }}>{t("header_sparkline")}</span>
            <Toggle label="" value={showHeaderSparkline()} onChange={setShowHeaderSparkline} />
          </div>

          {/* Graph player count */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "60px" }}>{t("graph_player_count")}</span>
            <input
              type="number"
              min="0"
              max="10"
              step="1"
              value={graphPlayerCount()}
              onInput={(e) => {
                const v = parseInt(e.currentTarget.value, 10);
                if (!isNaN(v) && v >= 0 && v <= 10) setGraphPlayerCount(v);
              }}
              style={{
                width: "60px",
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            />
          </div>

          {/* Graph for local player */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "60px" }}>{t("graph_for_local")}</span>
            <Toggle label="" value={graphForLocalPlayer()} onChange={setGraphForLocalPlayer} />
          </div>
          <div style={{ color: "#555", "font-size": "10px", "padding-left": "68px" }}>
            {t("graph_column_hint")}
          </div>
        </div>
      </details>

      <details>
        <summary style={sectionHeaderStyle}>{t("settings_copy")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>
          {/* Template textarea */}
          <div style={{ display: "flex", "align-items": "flex-start", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "60px", "padding-top": "2px" }}>{t("copy_template")}</span>
            <textarea
              rows="2"
              value={copyTemplate()}
              onInput={(e) => setCopyTemplate(e.currentTarget.value)}
              style={{
                flex: "1",
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "3px 5px",
                "font-size": "11px",
                "font-family": "monospace",
                resize: "vertical",
              }}
            />
          </div>

          {/* Placeholder reference */}
          <div style={{ color: "#666", "font-size": "10px", "padding-left": "68px", "font-family": "monospace" }}>
            {"{rank} {name} {class} {spec} {dmg} {dps} {pct} {crit} {critV} {lucky} {luckyV} {hits} {hpm} {score} {seasonLv} {seasonStr}"}
          </div>

          {/* Live preview */}
          <div style={{ display: "flex", "align-items": "flex-start", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "60px", "padding-top": "2px" }}>{t("copy_template_preview")}</span>
            <pre style={{
              flex: "1",
              margin: "0",
              padding: "4px 6px",
              background: "rgba(0,0,0,0.3)",
              "border-radius": "3px",
              color: "#ddd",
              "font-size": "11px",
              "white-space": "pre-wrap",
              "word-break": "break-all",
            }}>
              {formatRowAsText(SAMPLE_ROW, 1, copyTemplate(), abbreviateScores())}
            </pre>
          </div>
        </div>
      </details>

      <details>
        <summary style={sectionHeaderStyle}>{t("settings_combat")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>
          {/* Combat exit timeout */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("combat_exit_sec")}</span>
            <input
              type="number"
              min="0"
              max="60"
              step="1"
              value={combatExitSec()}
              onInput={(e) => {
                const v = parseInt(e.currentTarget.value, 10);
                if (!isNaN(v) && v >= 0 && v <= 60) setCombatExitSec(v);
              }}
              style={{
                width: "70px",
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            />
          </div>

          {/* Poll interval */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("poll_interval_ms")}</span>
            <input
              type="number"
              min="50"
              max="2000"
              step="50"
              value={pollIntervalMs()}
              onInput={(e) => {
                const v = parseInt(e.currentTarget.value, 10);
                if (!isNaN(v) && v >= 50 && v <= 2000) setPollIntervalMs(v);
              }}
              style={{
                width: "70px",
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            />
          </div>

          {/* 3分計測 duration */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("settings_3min_duration")}</span>
            <input
              type="number"
              min="30"
              max="1800"
              step="30"
              value={threeMinDurationSec()}
              onInput={(e) => {
                const v = parseInt(e.currentTarget.value, 10);
                if (!isNaN(v) && v >= 30 && v <= 1800) setThreeMinDurationSec(v);
              }}
              style={{
                width: "70px",
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            />
          </div>

          {/* 3分計測 auto-open modal */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("settings_3min_auto_open")}</span>
            <Toggle label="" value={threeMinAutoOpen()} onChange={setThreeMinAutoOpen} />
          </div>
        </div>
      </details>

      <details>
        <summary style={sectionHeaderStyle}>{t("settings_history")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>
          {/* History limit */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("history_limit")}</span>
            <input
              type="number"
              min="0"
              max="100"
              step="1"
              value={historyLimit()}
              onInput={(e) => {
                const v = parseInt(e.currentTarget.value, 10);
                if (!isNaN(v) && v >= 0 && v <= 100) setHistoryLimit(v);
              }}
              style={{
                width: "70px",
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            />
          </div>

          {/* Time series samples */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("time_series_samples")}</span>
            <input
              type="number"
              min="10"
              max="200"
              step="10"
              value={timeSeriesSamples()}
              onInput={(e) => {
                const v = parseInt(e.currentTarget.value, 10);
                if (!isNaN(v) && v >= 10 && v <= 200) setTimeSeriesSamples(v);
              }}
              style={{
                width: "70px",
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            />
          </div>

          {/* Time series interval */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("time_series_interval")}</span>
            <input
              type="number"
              min="250"
              max="5000"
              step="250"
              value={timeSeriesIntervalMs()}
              onInput={(e) => {
                const v = parseInt(e.currentTarget.value, 10);
                if (!isNaN(v) && v >= 250 && v <= 5000) setTimeSeriesIntervalMs(v);
              }}
              style={{
                width: "70px",
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            />
          </div>

          {/* Clear history */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <button
              onClick={() => clearHistory()}
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
              {t("clear_history")}
            </button>
          </div>
        </div>
      </details>

      <details>
        <summary style={sectionHeaderStyle}>{t("settings_overlay")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>
          {/* Buff overlay */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>Buff overlay</span>
            <Toggle
              label=""
              value={showBuffOverlay()}
              onChange={(v) => {
                setShowBuffOverlay(v);
                const win = WebviewWindow.getByLabel("buffs");
                if (win) {
                  if (v) {
                    win.show().catch(() => {});
                  } else {
                    win.hide().catch(() => {});
                  }
                }
              }}
            />
          </div>

          {/* Click-through */}
          <div style={{ display: "flex", "flex-direction": "column", gap: "2px" }}>
            <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
              <span style={{ color: "#aaa", width: "80px" }}>{t("click_through")}</span>
              <Toggle label="" value={clickThrough()} onChange={setClickThrough} />
            </div>
            <span style={{ color: "#555", "font-size": "10px", "padding-left": "88px" }}>
              {t("click_through_hint")}
            </span>
          </div>

          {/* Font size */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("font_size")}</span>
            <input
              type="number"
              min="10"
              max="18"
              step="1"
              value={fontSize()}
              onInput={(e) => {
                const v = parseInt(e.currentTarget.value, 10);
                if (!isNaN(v) && v >= 10 && v <= 18) setFontSize(v);
              }}
              style={{
                width: "60px",
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            />
          </div>

          {/* Highlight local player */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("highlight_local")}</span>
            <Toggle label="" value={highlightLocalPlayer()} onChange={setHighlightLocalPlayer} />
          </div>

          {/* Privacy mask */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("privacy_mask")}</span>
            <Toggle label="" value={privacyMaskNames()} onChange={setPrivacyMaskNames} />
          </div>

          {/* Startup tab */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }}>{t("startup_tab")}</span>
            <select
              value={startupTab()}
              onChange={(e) => setStartupTab(e.currentTarget.value)}
              style={{
                background: "rgba(255,255,255,0.1)",
                border: "1px solid rgba(255,255,255,0.2)",
                color: "#ddd",
                "border-radius": "3px",
                padding: "2px 4px",
                "font-size": "11px",
              }}
            >
              <option value="dps">{t("tab_dps")}</option>
              <option value="heal">{t("tab_heal")}</option>
              <option value="history">{t("tab_history")}</option>
            </select>
          </div>
        </div>
      </details>
    </div>
  );
}

function Toggle(props: { label: string; value: boolean; onChange: (v: boolean) => void }) {
  return (
    <label style={{
      display: "flex",
      "align-items": "center",
      gap: "3px",
      cursor: "pointer",
      color: props.value ? "#ddd" : "#666",
    }}>
      <input
        type="checkbox"
        checked={props.value}
        onChange={(e) => props.onChange(e.currentTarget.checked)}
        style={{ width: "12px", height: "12px" }}
      />
      <span>{props.label}</span>
    </label>
  );
}
