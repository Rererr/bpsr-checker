import { t, locale, setLocale } from "../lib/i18n";
import type { Locale } from "../lib/i18n";
import {
  opacity, setOpacity,
  showCrit, setShowCrit,
  showLucky, setShowLucky,
  showHpm, setShowHpm,
  showScore, setShowScore,
} from "../stores/settings";

export function SettingsPanel() {
  return (
    <div
      style={{
        padding: "8px 12px",
        background: "rgba(0,0,0,0.4)",
        "border-bottom": "1px solid rgba(255,255,255,0.1)",
        display: "flex",
        "flex-direction": "column",
        gap: "6px",
        "font-size": "11px",
      }}
    >
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
      <div style={{ display: "flex", "align-items": "center", gap: "8px", "flex-wrap": "wrap" }}>
        <span style={{ color: "#aaa", width: "60px" }}>{t("columns")}</span>
        <Toggle label={t("crit_rate")} value={showCrit()} onChange={setShowCrit} />
        <Toggle label={t("lucky_rate")} value={showLucky()} onChange={setShowLucky} />
        <Toggle label={t("hpm")} value={showHpm()} onChange={setShowHpm} />
        <Toggle label={t("score")} value={showScore()} onChange={setShowScore} />
      </div>
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
