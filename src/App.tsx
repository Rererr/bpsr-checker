import { createSignal, Show } from "solid-js";
import { Header } from "./components/Header";
import { PlayerTable } from "./components/PlayerTable";
import { SkillTable } from "./components/SkillTable";
import { TakenAttackersView } from "./components/TakenAttackersView";
import { TakenSkillsView } from "./components/TakenSkillsView";
import { SettingsPanel } from "./components/SettingsPanel";
import { HistoryView } from "./components/HistoryView";
import { ThreeMinResultModal } from "./components/ThreeMinResultModal";
import { wireBackendSettings, fontSize, startupTab, opacity, imagineOnlyMode } from "./stores/settings";
import { wireMeasureMode, threeMinResult } from "./stores/measureMode";
import { t } from "./lib/i18n";

export type Tab = "dps" | "heal" | "taken" | "history" | "skills";

const VALID_STARTUP_TABS: Tab[] = ["dps", "heal", "taken", "history"];

export default function App() {
  wireBackendSettings();
  wireMeasureMode();

  const initialTab = VALID_STARTUP_TABS.includes(startupTab() as Tab)
    ? (startupTab() as Tab)
    : "dps";
  const [tab, setTab] = createSignal<Tab>(initialTab);
  const [selectedPlayerUid, setSelectedPlayerUid] = createSignal<number | null>(null);
  const [takenTargetUid, setTakenTargetUid] = createSignal<number | null>(null);
  const [takenAttackerUid, setTakenAttackerUid] = createSignal<number | null>(null);
  const [takenAttackerName, setTakenAttackerName] = createSignal<string>("");
  const [showSettings, setShowSettings] = createSignal(false);

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
      background: `rgba(20, 20, 30, ${opacity()})`,
      height: "100vh",
      display: "flex",
      "flex-direction": "column",
      "font-size": `${fontSize()}px`,
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
          <PlayerTable
            tab={tab()}
            onSelectPlayer={tab() === "taken" ? openTakenAttackers : openSkills}
          />
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
