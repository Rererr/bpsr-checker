import { Show, createResource, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
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
  showElement, setShowElement,
  showDamageMode, setShowDamageMode,
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
  imagineOnlyMode, setImagineOnlyMode,
  compactSplitMode, setCompactSplitMode,
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

// ─── ToggleChip ───────────────────────────────────────────────
// ラベル左・ピルスイッチ右の1行カード型トグル
function ToggleChip(props: {
  label: string;
  value: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label
      style={{
        display: "flex",
        "align-items": "center",
        "justify-content": "space-between",
        gap: "6px",
        padding: "4px 7px",
        background: props.value
          ? "rgba(79,195,247,0.1)"
          : "rgba(255,255,255,0.04)",
        border: `1px solid ${props.value
          ? "rgba(79,195,247,0.3)"
          : "rgba(255,255,255,0.08)"}`,
        "border-radius": "5px",
        cursor: props.disabled ? "not-allowed" : "pointer",
        opacity: props.disabled ? "0.45" : "1",
        "pointer-events": props.disabled ? "none" : "auto",
        "min-width": "0",
      }}
    >
      <input
        type="checkbox"
        checked={props.value}
        onChange={(e) => props.onChange(e.currentTarget.checked)}
        style={{ display: "none" }}
      />
      <span
        style={{
          color: props.value ? "#b8dff0" : "#888",
          "font-size": "10px",
          overflow: "hidden",
          "text-overflow": "ellipsis",
          "white-space": "nowrap",
          "line-height": "1.3",
        }}
      >
        {props.label}
      </span>
      {/* ピルインジケーター */}
      <div
        style={{
          width: "24px",
          height: "13px",
          background: props.value ? "#4fc3f7" : "rgba(255,255,255,0.12)",
          "border-radius": "7px",
          position: "relative",
          "flex-shrink": "0",
        }}
      >
        <div
          style={{
            position: "absolute",
            top: "2.5px",
            left: props.value ? "12.5px" : "2.5px",
            width: "8px",
            height: "8px",
            background: props.value ? "#fff" : "rgba(255,255,255,0.5)",
            "border-radius": "50%",
          }}
        />
      </div>
    </label>
  );
}

// ─── NumCell ──────────────────────────────────────────────────
// ラベル上・input全幅のコンパクトセル（2列グリッドに収める）
function NumCell(props: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (v: number) => void;
}) {
  return (
    <div>
      <div
        style={{
          color: "#777",
          "font-size": "9px",
          "margin-bottom": "2px",
          overflow: "hidden",
          "text-overflow": "ellipsis",
          "white-space": "nowrap",
        }}
      >
        {props.label}
      </div>
      <input
        type="number"
        min={props.min}
        max={props.max}
        step={props.step ?? 1}
        value={props.value}
        onInput={(e) => {
          const v = parseInt(e.currentTarget.value, 10);
          if (!isNaN(v) && v >= props.min && v <= props.max) props.onChange(v);
        }}
        style={{
          width: "100%",
          "box-sizing": "border-box",
          background: "rgba(255,255,255,0.08)",
          border: "1px solid rgba(255,255,255,0.15)",
          color: "#ddd",
          "border-radius": "3px",
          padding: "2px 5px",
          "font-size": "11px",
        }}
      />
    </div>
  );
}

// ─── SelectCell ───────────────────────────────────────────────
// ラベル上・select全幅のコンパクトセル
function SelectCell(props: {
  label: string;
  children: any;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div>
      <div
        style={{
          color: "#777",
          "font-size": "9px",
          "margin-bottom": "2px",
          overflow: "hidden",
          "text-overflow": "ellipsis",
          "white-space": "nowrap",
        }}
      >
        {props.label}
      </div>
      <select
        value={props.value}
        onChange={(e) => props.onChange(e.currentTarget.value)}
        style={{
          width: "100%",
          "box-sizing": "border-box",
          background: "rgba(255,255,255,0.08)",
          border: "1px solid rgba(255,255,255,0.15)",
          color: "#ddd",
          "border-radius": "3px",
          padding: "2px 4px",
          "font-size": "11px",
        }}
      >
        {props.children}
      </select>
    </div>
  );
}

// ─── グリッドスタイル ─────────────────────────────────────────
// auto-fill: 140px 以上取れる限りカラムを自動増加
//   ~300px → 2列   / ~560px → 4列 / ~1024px → 7列+
const autoFillGrid = {
  display: "grid",
  "grid-template-columns": "repeat(auto-fill, minmax(140px, 1fr))",
  gap: "4px 6px",
  "align-items": "end",
} as const;

// 意味的に対になっている2項目は常に2列固定
const pairGrid = {
  display: "grid",
  "grid-template-columns": "1fr 1fr",
  gap: "4px",
} as const;

// ─── メインコンポーネント ─────────────────────────────────────
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
        "min-height": "0",
        "overflow-y": "auto",
      }}
    >

      {/* ── キャラクター ── */}
      <details open>
        <summary style={sectionHeaderStyle}>{t("settings_character")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>

          {/* UID入力 */}
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

          {/* 解決済み名前 */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "80px" }} />
            <span style={{ color: nameCache()?.name ? "#ddd" : "#555", "font-size": "11px" }}>
              {nameCache()?.name ?? t("selected_char_name_unknown")}
            </span>
          </div>

          {/* 候補 */}
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

          <div style={{ color: "#555", "font-size": "10px" }}>
            {t("selected_uid_input_hint")}
          </div>
          <div style={{ color: "#b07040", "font-size": "10px" }}>
            {t("selected_uid_change_warning")}
          </div>
        </div>
      </details>

      {/* ── 表示 ── */}
      <details open>
        <summary style={sectionHeaderStyle}>{t("settings_display")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>

          {/* 透明度スライダー */}
          <div style={{ display: "flex", "align-items": "center", gap: "8px" }}>
            <span style={{ color: "#aaa", "font-size": "10px", "flex-shrink": "0" }}>{t("transparency")}</span>
            <input
              type="range"
              min="0.3"
              max="1"
              step="0.05"
              value={opacity()}
              onInput={(e) => setOpacity(parseFloat(e.currentTarget.value))}
              style={{ flex: "1" }}
            />
            <span style={{ color: "#888", width: "28px", "text-align": "right", "font-size": "10px" }}>
              {Math.round(opacity() * 100)}%
            </span>
          </div>

          {/* 言語 + グラフ表示人数 */}
          <div style={pairGrid}>
            <SelectCell
              label={t("language")}
              value={locale()}
              onChange={(v) => setLocale(v as Locale)}
            >
              <option value="ja">日本語</option>
              <option value="en">English</option>
            </SelectCell>
            <NumCell
              label={t("graph_player_count")}
              value={graphPlayerCount()}
              min={0}
              max={10}
              onChange={setGraphPlayerCount}
            />
          </div>

          {/* Compact split mode */}
          <div style={{ display: "flex", "align-items": "flex-start", gap: "8px" }}>
            <span style={{ color: "#777", width: "60px", "padding-top": "1px", "font-size": "10px", "text-transform": "uppercase", "letter-spacing": "0.04em" }}>{t("view")}</span>
            <div style={{ display: "flex", "flex-wrap": "wrap", gap: "6px 10px", flex: "1" }}>
              <Toggle label={t("compact_split_mode")} value={compactSplitMode()} onChange={setCompactSplitMode} />
            </div>
          </div>

          {/* 列表示: 統計 */}
          <div style={{ display: "flex", "align-items": "flex-start", gap: "8px" }}>
            <span style={{ color: "#777", width: "40px", "padding-top": "1px", "font-size": "10px", "text-transform": "uppercase", "letter-spacing": "0.04em" }}>
              {t("columns_stats")}
            </span>
            <div style={{ display: "flex", "flex-wrap": "wrap", gap: "6px 10px", flex: "1" }}>
              <Toggle label={t("crit_rate")} value={showCrit()} onChange={setShowCrit} />
              <Toggle label={t("crit_value")} value={showCritValue()} onChange={setShowCritValue} />
              <Toggle label={t("lucky_rate")} value={showLucky()} onChange={setShowLucky} />
              <Toggle label={t("lucky_value")} value={showLuckyValue()} onChange={setShowLuckyValue} />
              <Toggle label={t("hits")} value={showHits()} onChange={setShowHits} />
              <Toggle label={t("hpm")} value={showHpm()} onChange={setShowHpm} />
            </div>
          </div>

          {/* 列表示: メタ */}
          <div style={{ display: "flex", "align-items": "flex-start", gap: "8px" }}>
            <span style={{ color: "#777", width: "40px", "padding-top": "1px", "font-size": "10px", "text-transform": "uppercase", "letter-spacing": "0.04em" }}>
              {t("columns_meta")}
            </span>
            <div style={{ display: "flex", "flex-wrap": "wrap", gap: "6px 10px", flex: "1" }}>
              <Toggle label={t("element")} value={showElement()} onChange={setShowElement} />
              <Toggle label={t("damage_mode")} value={showDamageMode()} onChange={setShowDamageMode} />
              <Toggle label={t("score")} value={showScore()} onChange={setShowScore} />
              <Toggle label={t("abbreviate_scores")} value={abbreviateScores()} onChange={setAbbreviateScores} />
            </div>
          </div>

          {/* 名前列テンプレ */}
          <div style={{ display: "flex", "align-items": "flex-start", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "100px", "padding-top": "2px" }}>{t("name_template")}</span>
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

          {/* グラフ表示トグル: ヘッダー合計 + 自分キャラ */}
          <div style={pairGrid}>
            <ToggleChip
              label={t("header_sparkline")}
              value={showHeaderSparkline()}
              onChange={setShowHeaderSparkline}
            />
            <ToggleChip
              label={t("graph_for_local")}
              value={graphForLocalPlayer()}
              onChange={setGraphForLocalPlayer}
            />
          </div>
          <div style={{ color: "#555", "font-size": "9px" }}>
            {t("graph_column_hint")}
          </div>

        </div>
      </details>

      {/* ── コピー ── */}
      <details>
        <summary style={sectionHeaderStyle}>{t("settings_copy")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>

          <div style={{ display: "flex", "align-items": "flex-start", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "100px", "padding-top": "2px" }}>{t("copy_template")}</span>
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

          <div style={{ color: "#666", "font-size": "10px", "padding-left": "108px", "font-family": "monospace" }}>
            {"{rank} {name} {class} {spec} {dmg} {dps} {pct} {crit} {critV} {lucky} {luckyV} {hits} {hpm} {score} {seasonLv} {seasonStr}"}
          </div>

          <div style={{ display: "flex", "align-items": "flex-start", gap: "8px" }}>
            <span style={{ color: "#aaa", width: "100px", "padding-top": "2px" }}>{t("copy_template_preview")}</span>
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

      {/* ── 戦闘 ── */}
      <details>
        <summary style={sectionHeaderStyle}>{t("settings_combat")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>

          {/* 幅が広いほど1行に収まる（~300px:2列 / ~560px:4列） */}
          <div style={autoFillGrid}>
            <NumCell
              label={t("combat_exit_sec")}
              value={combatExitSec()}
              min={0}
              max={60}
              onChange={setCombatExitSec}
            />
            <NumCell
              label={t("poll_interval_ms")}
              value={pollIntervalMs()}
              min={50}
              max={2000}
              step={50}
              onChange={setPollIntervalMs}
            />
            <NumCell
              label={t("settings_3min_duration")}
              value={threeMinDurationSec()}
              min={30}
              max={1800}
              step={30}
              onChange={setThreeMinDurationSec}
            />
            <ToggleChip
              label={t("settings_3min_auto_open")}
              value={threeMinAutoOpen()}
              onChange={setThreeMinAutoOpen}
            />
          </div>

        </div>
      </details>

      {/* ── 履歴 ── */}
      <details>
        <summary style={sectionHeaderStyle}>{t("settings_history")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>

          {/* 幅が広いほど1行に収まる（~300px:2列 / ~560px:4列） */}
          <div style={autoFillGrid}>
            <NumCell
              label={t("history_limit")}
              value={historyLimit()}
              min={0}
              max={100}
              onChange={setHistoryLimit}
            />
            <NumCell
              label={t("time_series_samples")}
              value={timeSeriesSamples()}
              min={10}
              max={200}
              step={10}
              onChange={setTimeSeriesSamples}
            />
            <NumCell
              label={t("time_series_interval")}
              value={timeSeriesIntervalMs()}
              min={250}
              max={5000}
              step={250}
              onChange={setTimeSeriesIntervalMs}
            />
            <button
              onClick={() => clearHistory()}
              style={{
                padding: "3px 8px",
                border: "1px solid rgba(255,255,255,0.2)",
                "border-radius": "3px",
                background: "transparent",
                color: "#ccc",
                cursor: "pointer",
                "font-size": "10px",
                "align-self": "end",
              }}
            >
              {t("clear_history")}
            </button>
          </div>

        </div>
      </details>

      {/* ── オーバーレイ ── */}
      <details>
        <summary style={sectionHeaderStyle}>{t("settings_overlay")}</summary>
        <div style={{ ...sectionStyle, "margin-top": "6px" }}>

          {/* イマジンデバフタイマー + 専用モード（対になっているので常に2列） */}
          <div style={pairGrid}>
            <ToggleChip
              label={t("imagine_debuff_timer")}
              value={showBuffOverlay()}
              disabled={imagineOnlyMode()}
              onChange={(v) => {
                if (!v && imagineOnlyMode()) return;
                setShowBuffOverlay(v);
                invoke("set_buffs_window_visible", { visible: v }).catch(() => {});
              }}
            />
            <ToggleChip
              label={t("imagine_only_mode")}
              value={imagineOnlyMode()}
              onChange={(v) => {
                setImagineOnlyMode(v);
                if (v && !showBuffOverlay()) {
                  setShowBuffOverlay(true);
                }
                if (v) {
                  invoke("set_buffs_window_visible", { visible: true }).catch(() => {});
                }
              }}
            />
          </div>
          <div style={{ color: "#555", "font-size": "9px" }}>
            {t("imagine_only_mode_hint")}
          </div>

          {/* クリックスルー */}
          <ToggleChip
            label={t("click_through")}
            value={clickThrough()}
            onChange={setClickThrough}
          />
          <div style={{ color: "#555", "font-size": "9px" }}>
            {t("click_through_hint")}
          </div>

          {/* フォント / タブ / 強調 / マスク: 幅が広いほど1行に収まる */}
          <div style={autoFillGrid}>
            <NumCell
              label={t("font_size")}
              value={fontSize()}
              min={10}
              max={18}
              onChange={setFontSize}
            />
            <SelectCell
              label={t("startup_tab")}
              value={startupTab()}
              onChange={setStartupTab}
            >
              <option value="dps">{t("tab_dps")}</option>
              <option value="heal">{t("tab_heal")}</option>
              <option value="history">{t("tab_history")}</option>
            </SelectCell>
            <ToggleChip
              label={t("highlight_local")}
              value={highlightLocalPlayer()}
              onChange={setHighlightLocalPlayer}
            />
            <ToggleChip
              label={t("privacy_mask")}
              value={privacyMaskNames()}
              onChange={setPrivacyMaskNames}
            />
          </div>

        </div>
      </details>

    </div>
  );
}

// ─── Toggle（列表示グループ用・既存維持） ─────────────────────
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
