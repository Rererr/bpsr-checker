import { createSignal } from "solid-js";
import { Header } from "./components/Header";
import { PlayerTable } from "./components/PlayerTable";
import { SkillTable } from "./components/SkillTable";
import { SettingsPanel } from "./components/SettingsPanel";
import { HistoryView } from "./components/HistoryView";
import { wireBackendSettings, fontSize, startupTab } from "./stores/settings";

export type Tab = "dps" | "heal" | "history" | "skills";

const VALID_STARTUP_TABS: Tab[] = ["dps", "heal", "history"];

export default function App() {
  wireBackendSettings();

  const initialTab = VALID_STARTUP_TABS.includes(startupTab() as Tab)
    ? (startupTab() as Tab)
    : "dps";
  const [tab, setTab] = createSignal<Tab>(initialTab);
  const [selectedPlayerUid, setSelectedPlayerUid] = createSignal<number | null>(null);
  const [showSettings, setShowSettings] = createSignal(false);

  const openSkills = (uid: number) => {
    setSelectedPlayerUid(uid);
    setTab("skills");
  };

  const backToList = () => {
    setSelectedPlayerUid(null);
    setTab("dps");
  };

  return (
    <div style={{
      background: "rgba(20, 20, 30, 0.85)",
      "min-height": "100vh",
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
      {tab() === "history" ? (
        <HistoryView />
      ) : tab() === "skills" && selectedPlayerUid() !== null ? (
        <SkillTable playerUid={selectedPlayerUid()!} onBack={backToList} />
      ) : (
        <PlayerTable tab={tab()} onSelectPlayer={openSkills} />
      )}
    </div>
  );
}
