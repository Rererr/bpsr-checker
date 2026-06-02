import { createSignal } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { invoke } from "@tauri-apps/api/core";
import { clearWatchlist, bulkAddPlayers } from "./watchlist";

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
  nameResolved: boolean;
  className: string;
  classSpecName: string;
  abilityScore: number;
  seasonLevel: number;
  seasonStrength: number;
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
  element: number;
  damageMode: number;
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

// プレイヤー一覧はポーリング毎に丸ごと差し替えると <For> が全行の DOM を破棄・
// 再生成し、クリックの mousedown→mouseup の間にノードが消えてクリックが行へ
// 届かなくなる。reconcile(uid キー) で既存行オブジェクトの同一性を維持し、
// DOM ノードを使い回す（クリック発火の安定化・再描画コスト削減）。
const emptyWindow = (): PlayersWindow => ({ playerRows: [], localPlayerUid: 0, topValue: 0 });

const [dpsPlayers, setDpsPlayers] = createStore<PlayersWindow>(emptyWindow());
const [healPlayers, setHealPlayers] = createStore<PlayersWindow>(emptyWindow());
const [bossPlayers, setBossPlayers] = createStore<PlayersWindow>(emptyWindow());
const [takenPlayers, setTakenPlayers] = createStore<PlayersWindow>(emptyWindow());

export { header, dpsPlayers, healPlayers, bossPlayers, takenPlayers };

export async function fetchDpsData() {
  try {
    const [h, d] = await Promise.all([
      invoke<HeaderInfo>("get_header_info"),
      invoke<PlayersWindow>("get_dps_players"),
    ]);
    setHeader(h);
    setDpsPlayers(reconcile(d, { key: "uid" }));
    if (d.playerRows.length > 0) {
      bulkAddPlayers(d.playerRows.map((r) => r.uid));
    }
  } catch {
    // silently ignore — backend may not be ready
  }
}

export async function fetchHealData() {
  try {
    const data = await invoke<PlayersWindow>("get_heal_players");
    setHealPlayers(reconcile(data, { key: "uid" }));
  } catch {}
}

export async function fetchBossData() {
  try {
    const data = await invoke<PlayersWindow>("get_dps_boss_players");
    setBossPlayers(reconcile(data, { key: "uid" }));
  } catch {}
}

export async function fetchTakenData() {
  try {
    const data = await invoke<PlayersWindow>("get_dmg_taken_players");
    setTakenPlayers(reconcile(data, { key: "uid" }));
  } catch {}
}

export async function fetchTakenAttackers(playerUid: number): Promise<SkillsWindow | null> {
  try {
    return await invoke<SkillsWindow>("get_dmg_taken_attackers", { playerUid });
  } catch {
    return null;
  }
}

export async function fetchTakenSkills(playerUid: number, attackerUid: number): Promise<SkillsWindow | null> {
  try {
    return await invoke<SkillsWindow>("get_dmg_taken_skills", { playerUid, attackerUid });
  } catch {
    return null;
  }
}

export async function fetchSkills(playerUid: number): Promise<SkillsWindow | null> {
  try {
    return await invoke<SkillsWindow>("get_skills", { playerUid });
  } catch {
    return null;
  }
}

export function resetPlayerWindows(): void {
  setHeader({ totalDps: 0, totalDmg: 0, elapsedMs: 0, timeLastCombatPacketMs: 0 });
  setDpsPlayers(reconcile(emptyWindow(), { key: "uid" }));
  setHealPlayers(reconcile(emptyWindow(), { key: "uid" }));
  setBossPlayers(reconcile(emptyWindow(), { key: "uid" }));
  setTakenPlayers(reconcile(emptyWindow(), { key: "uid" }));
  clearWatchlist();
}

export async function resetEncounter() {
  try {
    await invoke("reset_encounter");
  } catch (e) {
    console.error("reset_encounter failed:", e);
  }
  resetPlayerWindows();
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
  try {
    await invoke("clear_history");
  } catch (e) {
    console.error("clear_history failed:", e);
  }
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

