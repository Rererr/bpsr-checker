import { createSignal } from "solid-js";

export interface SelfBuffSnapshot {
  kind: string;
  baseId: number;
  buffUuid: number;
  layer: number;
  remainingMs: number;
  durationMs: number;
  receivedAtMs: number;
}

export interface SelfBuffsData {
  buffs: SelfBuffSnapshot[];
  nowMs: number;
}

const EMPTY: SelfBuffsData = { buffs: [], nowMs: 0 };

export const [selfBuffs, setSelfBuffs] = createSignal<SelfBuffsData>(EMPTY);
// ポーリングデータを受信した performance.now() の時刻（補間計算用）
export const [pollReceivedAt, setPollReceivedAt] = createSignal<number>(performance.now());

let timerId: ReturnType<typeof setInterval> | null = null;

const isTauri = (): boolean => "__TAURI_INTERNALS__" in window;

async function fetchBuffs(): Promise<void> {
  if (!isTauri()) {
    setSelfBuffs(MOCK_DATA);
    setPollReceivedAt(performance.now());
    return;
  }
  const { invoke } = await import("@tauri-apps/api/core");
  const data = await invoke<SelfBuffsData>("get_self_buffs");
  setPollReceivedAt(performance.now());
  setSelfBuffs(data);
}

export function startBuffPolling(intervalMs: number): void {
  stopBuffPolling();
  fetchBuffs().catch(() => {});
  timerId = setInterval(() => {
    fetchBuffs().catch(() => {});
  }, intervalMs);
}

export function stopBuffPolling(): void {
  if (timerId !== null) {
    clearInterval(timerId);
    timerId = null;
  }
}

const MOCK_DATA: SelfBuffsData = {
  buffs: [
    {
      kind: "Tina",
      baseId: 1,
      buffUuid: 101,
      layer: 1,
      remainingMs: 4200,
      durationMs: 10000,
      receivedAtMs: performance.now() - 5800,
    },
    {
      kind: "Tarta",
      baseId: 3,
      buffUuid: 301,
      layer: 1,
      remainingMs: 8100,
      durationMs: 12000,
      receivedAtMs: performance.now() - 3900,
    },
    {
      kind: "Basilisk",
      baseId: 4,
      buffUuid: 401,
      layer: 1,
      remainingMs: 12000,
      durationMs: 15000,
      receivedAtMs: performance.now() - 3000,
    },
  ],
  nowMs: performance.now(),
};
