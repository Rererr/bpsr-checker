import { createSignal } from "solid-js";
import { Header } from "./components/Header";
import { PlayerTable } from "./components/PlayerTable";
import { SkillTable } from "./components/SkillTable";
import { SettingsPanel } from "./components/SettingsPanel";

export type Tab = "dps" | "heal" | "skills";

export default function App() {
  const [tab, setTab] = createSignal<Tab>("dps");
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
    }}>
      <Header
        tab={tab()}
        onTabChange={setTab}
        onToggleSettings={() => setShowSettings(!showSettings())}
      />
      {showSettings() && <SettingsPanel />}
      {tab() === "skills" && selectedPlayerUid() !== null ? (
        <SkillTable playerUid={selectedPlayerUid()!} onBack={backToList} />
      ) : (
        <PlayerTable tab={tab()} onSelectPlayer={openSkills} />
      )}
    </div>
  );
}
