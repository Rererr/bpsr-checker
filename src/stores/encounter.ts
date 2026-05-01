import { createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

// Types matching Rust bridge/models.rs
export interface HeaderInfo {
  totalDps: number;
  totalDmg: number;
  elapsedMs: number;
  timeLastCombatPacketMs: number;
}

export interface PlayerRow {
  uid: number;
  name: string;
  className: string;
  classSpecName: string;
  abilityScore: number;
  totalValue: number;
  valuePerSec: number;
  valuePct: number;
  critRate: number;
  critValueRate: number;
  luckyRate: number;
  luckyValueRate: number;
  hits: number;
  hitsPerMinute: number;
  timeSeries: TimeSeriesPoint[];
}

export interface PlayersWindow {
  playerRows: PlayerRow[];
  localPlayerUid: number;
  topValue: number;
}

export interface SkillRow {
  uid: number;
  name: string;
  totalValue: number;
  valuePerSec: number;
  valuePct: number;
  critRate: number;
  critValueRate: number;
  luckyRate: number;
  luckyValueRate: number;
  hits: number;
  hitsPerMinute: number;
}

export interface SkillsWindow {
  inspectedPlayer: PlayerRow;
  skillRows: SkillRow[];
  localPlayerUid: number;
  topValue: number;
}

// Signals
const [header, setHeader] = createSignal<HeaderInfo>({
  totalDps: 0,
  totalDmg: 0,
  elapsedMs: 0,
  timeLastCombatPacketMs: 0,
});

const [dpsPlayers, setDpsPlayers] = createSignal<PlayersWindow>({
  playerRows: [],
  localPlayerUid: 0,
  topValue: 0,
});

const [healPlayers, setHealPlayers] = createSignal<PlayersWindow>({
  playerRows: [],
  localPlayerUid: 0,
  topValue: 0,
});

const [bossPlayers, setBossPlayers] = createSignal<PlayersWindow>({
  playerRows: [],
  localPlayerUid: 0,
  topValue: 0,
});

export { header, dpsPlayers, healPlayers, bossPlayers };

export async function fetchDpsData() {
  try {
    const [h, d] = await Promise.all([
      invoke<HeaderInfo>("get_header_info"),
      invoke<PlayersWindow>("get_dps_players"),
    ]);
    setHeader(h);
    setDpsPlayers(d);
  } catch {
    // silently ignore — backend may not be ready
  }
}

export async function fetchHealData() {
  try {
    const data = await invoke<PlayersWindow>("get_heal_players");
    setHealPlayers(data);
  } catch {}
}

export async function fetchBossData() {
  try {
    const data = await invoke<PlayersWindow>("get_dps_boss_players");
    setBossPlayers(data);
  } catch {}
}

export async function fetchSkills(playerUid: number): Promise<SkillsWindow | null> {
  try {
    return await invoke<SkillsWindow>("get_skills", { playerUid });
  } catch {
    return null;
  }
}

export async function resetEncounter() {
  await invoke("reset_encounter");
}

export async function togglePause() {
  await invoke("toggle_pause");
}

export interface TimeSeriesPoint {
  tMs: number;
  totalDmg: number;
  totalDps: number;
}

export interface EncounterSnapshot {
  id: number;
  startMs: number;
  endMs: number;
  durationMs: number;
  totalDmg: number;
  totalDps: number;
  playerRows: PlayerRow[];
  timeSeries: TimeSeriesPoint[];
}

const [history, setHistory] = createSignal<EncounterSnapshot[]>([]);
export { history };

export async function fetchHistory() {
  try {
    const data = await invoke<EncounterSnapshot[]>("get_history");
    setHistory(data);
  } catch {}
}

export async function clearHistory() {
  await invoke("clear_history");
  await fetchHistory();
}

const [timeSeries, setTimeSeries] = createSignal<TimeSeriesPoint[]>([]);
export { timeSeries };

export async function fetchTimeSeries() {
  try {
    const data = await invoke<TimeSeriesPoint[]>("get_time_series");
    setTimeSeries(data);
  } catch {}
}
