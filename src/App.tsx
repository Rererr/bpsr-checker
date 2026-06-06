import { createEffect, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { Header } from "./components/Header";
import { PlayerTable } from "./components/PlayerTable";
import { SkillTable } from "./components/SkillTable";
import { TakenAttackersView } from "./components/TakenAttackersView";
import { TakenSkillsView } from "./components/TakenSkillsView";
import { SettingsPanel } from "./components/SettingsPanel";
import { HistoryView } from "./components/HistoryView";
import { ThreeMinResultModal } from "./components/ThreeMinResultModal";
import { wireBackendSettings, fontSize, startupTab, imagineOnlyMode } from "./stores/settings";
import { wireMeasureMode, threeMinResult } from "./stores/measureMode";
import { t } from "./lib/i18n";

export type Tab = "dps" | "heal" | "taken" | "history" | "skills";

const VALID_STARTUP_TABS: Tab[] = ["dps", "heal", "taken", "history"];

// オーバーレイ白紙化対策のリロード時だけ、タブ/設定パネルの表示を復元する。
// sessionStorage はリロードで保持・アプリ再起動で消えるため、通常起動では
// startupTab を尊重する。マーカーで「トグル起因のリロード」のみを判定する。
const VIEW_PENDING_RELOAD = "bpsr.view.pendingReload";
const VIEW_TAB_KEY = "bpsr.view.tab";
const VIEW_SETTINGS_KEY = "bpsr.view.showSettings";

export default function App() {
  // リロード直後か判定し、マーカーは一度きりで消費する
  const restoringReload = (() => {
    try {
      if (sessionStorage.getItem(VIEW_PENDING_RELOAD) === "1") {
        sessionStorage.removeItem(VIEW_PENDING_RELOAD);
        return true;
      }
    } catch {}
    return false;
  })();

  onMount(async () => {
    const unlisteners = await Promise.all([
      wireBackendSettings(),
      wireMeasureMode(),
    ]);
    onCleanup(() => unlisteners.forEach((u) => u()));
  });
  const restoredTab = (): Tab | null => {
    if (!restoringReload) return null;
    try {
      const v = sessionStorage.getItem(VIEW_TAB_KEY);
      if (v && VALID_STARTUP_TABS.includes(v as Tab)) return v as Tab;
    } catch {}
    return null;
  };
  const restoredShowSettings = (): boolean => {
    if (!restoringReload) return false;
    try { return sessionStorage.getItem(VIEW_SETTINGS_KEY) === "1"; } catch { return false; }
  };

  const initialTab = restoredTab()
    ?? (VALID_STARTUP_TABS.includes(startupTab() as Tab) ? (startupTab() as Tab) : "dps");
  const [tab, setTab] = createSignal<Tab>(initialTab);
  const [selectedPlayerUid, setSelectedPlayerUid] = createSignal<number | null>(null);
  const [takenTargetUid, setTakenTargetUid] = createSignal<number | null>(null);
  const [takenAttackerUid, setTakenAttackerUid] = createSignal<number | null>(null);
  const [takenAttackerName, setTakenAttackerName] = createSignal<string>("");
  const [showSettings, setShowSettings] = createSignal(restoredShowSettings());

  // 現在のタブ/設定パネル表示を sessionStorage に同期（リロード復元用）
  createEffect(() => { try { sessionStorage.setItem(VIEW_TAB_KEY, tab()); } catch {} });
  createEffect(() => { try { sessionStorage.setItem(VIEW_SETTINGS_KEY, showSettings() ? "1" : "0"); } catch {} });

  const openSkills = (uid: number) => {
    setSelectedPlayerUid(uid);
    setTab("skills");
  };

  const backToList = () => {
    setSelectedPlayerUid(null);
    setTab("dps");
  };

  const openTakenAttackers = (uid: number) => {
    setTakenTargetUid(uid);
    setTakenAttackerUid(null);
  };

  const openTakenSkills = (attackerUid: number, attackerName: string) => {
    setTakenAttackerUid(attackerUid);
    setTakenAttackerName(attackerName);
  };

  const backFromTakenSkills = () => {
    setTakenAttackerUid(null);
  };

  const backFromTakenAttackers = () => {
    setTakenTargetUid(null);
  };

  return (
    <div style={{
      // 透過は OS のレイヤードウィンドウ・アルファ(set_main_opacity)で実現するため不透明
      background: "rgb(20, 20, 30)",
      height: "100vh",
      display: "flex",
      "flex-direction": "column",
      "font-size": `${fontSize()}px`,
      // flex 子が 100vh を超えて伸びてもヘッダーを画面外へ押し出さない
      overflow: "hidden",
    }}>
      <Header
        tab={tab()}
        onTabChange={setTab}
        onToggleSettings={() => setShowSettings(!showSettings())}
      />
      {showSettings() && <SettingsPanel />}
      <Show
        when={!imagineOnlyMode()}
        fallback={<ImagineOnlyBanner onOpenSettings={() => setShowSettings(true)} />}
      >
        {tab() === "history" ? (
          <HistoryView />
        ) : tab() === "skills" && selectedPlayerUid() !== null ? (
          <SkillTable playerUid={selectedPlayerUid()!} onBack={backToList} />
        ) : tab() === "taken" && takenTargetUid() !== null && takenAttackerUid() !== null ? (
          <TakenSkillsView
            playerUid={takenTargetUid()!}
            attackerUid={takenAttackerUid()!}
            attackerName={takenAttackerName()}
            onBack={backFromTakenSkills}
          />
        ) : tab() === "taken" && takenTargetUid() !== null ? (
          <TakenAttackersView
            playerUid={takenTargetUid()!}
            onSelectAttacker={openTakenSkills}
            onBack={backFromTakenAttackers}
          />
        ) : (
          <div style={{ flex: "1", display: "flex", "flex-direction": "column", "min-height": "0" }}>
            <Show when={tab() === "taken"}>
              <div style={{ padding: "4px 8px", "font-size": "10px", color: "#888", "border-bottom": "1px solid rgba(255,255,255,0.05)" }}>
                {t("taken_select_target")}
              </div>
            </Show>
            <PlayerTable
              tab={tab()}
              onSelectPlayer={tab() === "taken" ? openTakenAttackers : openSkills}
            />
          </div>
        )}
      </Show>
      <Show when={threeMinResult() !== null && !imagineOnlyMode()}>
        <ThreeMinResultModal />
      </Show>
    </div>
  );
}

function ImagineOnlyBanner(props: { onOpenSettings: () => void }) {
  return (
    <div
      style={{
        flex: "1",
        display: "flex",
        "flex-direction": "column",
        "align-items": "center",
        "justify-content": "center",
        gap: "10px",
        padding: "16px",
        color: "#bbb",
        "text-align": "center",
      }}
    >
      <div style={{ "font-size": "13px", color: "#ddd", "font-weight": "bold" }}>
        {t("imagine_only_mode_active_title")}
      </div>
      <div style={{ "font-size": "11px", color: "#999", "line-height": "1.5", "max-width": "360px" }}>
        {t("imagine_only_mode_active_body")}
      </div>
      <div style={{ "font-size": "10px", color: "#666", "max-width": "360px" }}>
        {t("imagine_only_mode_active_hint")}
      </div>
      <button
        onClick={props.onOpenSettings}
        style={{
          padding: "4px 12px",
          border: "1px solid rgba(255,255,255,0.2)",
          "border-radius": "3px",
          background: "transparent",
          color: "#ccc",
          cursor: "pointer",
          "font-size": "11px",
        }}
      >
        {t("settings")}
      </button>
    </div>
  );
}
