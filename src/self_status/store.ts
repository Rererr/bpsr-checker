import { createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { SelfStatusData } from "./types";

const EMPTY: SelfStatusData = { buffs: [], debuffs: [], nowMs: 0, localPlayerUid: 0 };

export const [selfStatus, setSelfStatus] = createSignal<SelfStatusData>(EMPTY);
export const [pollReceivedAt, setPollReceivedAt] = createSignal<number>(performance.now());
const [tick, setTick] = createSignal(0);
export { tick };

let pollTimer: ReturnType<typeof setInterval> | null = null;
let tickTimer: ReturnType<typeof setInterval> | null = null;

const isTauri = (): boolean => "__TAURI_INTERNALS__" in window;

async function fetchStatus(): Promise<void> {
  if (!isTauri()) {
    setSelfStatus(MOCK_DATA);
    setPollReceivedAt(performance.now());
    return;
  }
  const data = await invoke<SelfStatusData>("get_self_buff_status");
  setPollReceivedAt(performance.now());
  setSelfStatus(data);
}

export function startStatusPolling(intervalMs: number): void {
  stopStatusPolling();
  fetchStatus().catch(() => {});
  pollTimer = setInterval(() => fetchStatus().catch(() => {}), intervalMs);
  tickTimer = setInterval(() => setTick((n) => n + 1), 50);
}

export function stopStatusPolling(): void {
  if (pollTimer !== null) { clearInterval(pollTimer); pollTimer = null; }
  if (tickTimer !== null) { clearInterval(tickTimer); tickTimer = null; }
}

const now = performance.now();
const MOCK_DATA: SelfStatusData = {
  localPlayerUid: 1001,
  nowMs: now,
  buffs: [
    { instanceId: 1, baseId: 2110095, category: "buff", priority: "high", remainingMs: 18000, durationMs: 30000, layer: 1, sourceConfigId: 0 },
    { instanceId: 2, baseId: 2100151, category: "buff", priority: "high", remainingMs: 8500, durationMs: 12000, layer: 1, sourceConfigId: 0 },
    { instanceId: 3, baseId: 55330, category: "buff", priority: "alert", remainingMs: 4200, durationMs: 6000, layer: 1, sourceConfigId: 0 },
    { instanceId: 4, baseId: 21422, category: "buff", priority: "high", remainingMs: 0, durationMs: 0, layer: 1, sourceConfigId: 0 },
  ],
  debuffs: [
    { instanceId: 5, baseId: 2110056, category: "debuff", priority: "normal", remainingMs: 12000, durationMs: 15000, layer: 1, sourceConfigId: 0 },
    { instanceId: 6, baseId: 4501, category: "debuff", priority: "high", remainingMs: 3100, durationMs: 8000, layer: 1, sourceConfigId: 0 },
  ],
};
