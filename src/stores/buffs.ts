import { createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { isWatched, seedLocalPlayer, watchedUids } from "./watchlist";
import type { SelfBuffSnapshot } from "../buffs/types";

export type { SelfBuffSnapshot };

export interface PlayerBuffSnapshot {
  uid: number;
  name: string;
  buffs: SelfBuffSnapshot[];
}

export interface TrackedBuffsData {
  players: PlayerBuffSnapshot[];
  nowMs: number;
  localPlayerUid: number;
}

const EMPTY: TrackedBuffsData = { players: [], nowMs: 0, localPlayerUid: 0 };

export const [trackedBuffs, setTrackedBuffs] = createSignal<TrackedBuffsData>(EMPTY);
// ポーリングデータを受信した performance.now() の時刻（補間計算用）
export const [pollReceivedAt, setPollReceivedAt] = createSignal<number>(performance.now());
// 50ms 補間用 tick（CircularBuff 購読専用）
const [tick, setTick] = createSignal(0);
export { tick };

let timerId: ReturnType<typeof setInterval> | null = null;
let tickTimerId: ReturnType<typeof setInterval> | null = null;

const isTauri = (): boolean => "__TAURI_INTERNALS__" in window;

async function fetchTrackedBuffs(): Promise<void> {
  if (!isTauri()) {
    // dev: mock を返す。seedLocalPlayer は先頭挿入なので逆順にループして順序を保つ
    for (let i = MOCK_DATA.players.length - 1; i >= 0; i--) {
      const p = MOCK_DATA.players[i];
      if (!isWatched(p.uid)) seedLocalPlayer(p.uid);
    }
    setTrackedBuffs(MOCK_DATA);
    setPollReceivedAt(performance.now());
    return;
  }
  const uids = watchedUids();
  const data = await invoke<TrackedBuffsData>("get_tracked_buffs", { uids });
  setPollReceivedAt(performance.now());
  setTrackedBuffs(data);
  if (data.localPlayerUid !== 0) {
    seedLocalPlayer(data.localPlayerUid);
  }
}

export function startBuffPolling(intervalMs: number): void {
  stopBuffPolling();
  fetchTrackedBuffs().catch(() => {});
  timerId = setInterval(() => {
    fetchTrackedBuffs().catch(() => {});
  }, intervalMs);
  tickTimerId = setInterval(() => setTick((n) => n + 1), 50);
}

export function stopBuffPolling(): void {
  if (timerId !== null) {
    clearInterval(timerId);
    timerId = null;
  }
  if (tickTimerId !== null) {
    clearInterval(tickTimerId);
    tickTimerId = null;
  }
}

const now = performance.now();
const MOCK_DATA: TrackedBuffsData = {
  localPlayerUid: 1001,
  nowMs: now,
  players: [
    {
      uid: 1001,
      name: "テストA",
      buffs: [
        { kind: "Tina", baseId: 1, buffUuid: 101, layer: 1, remainingMs: 42000, durationMs: 60000, receivedAtMs: now - 18000 },
        { kind: "Tarta", baseId: 3, buffUuid: 301, layer: 1, remainingMs: 8100, durationMs: 12000, receivedAtMs: now - 3900 },
      ],
    },
    {
      uid: 1002,
      name: "テストB",
      buffs: [
        { kind: "Aluna", baseId: 2, buffUuid: 201, layer: 1, remainingMs: 11000, durationMs: 15000, receivedAtMs: now - 4000 },
        { kind: "Basilisk", baseId: 4, buffUuid: 401, layer: 1, remainingMs: 2300, durationMs: 10000, receivedAtMs: now - 7700 },
      ],
    },
    {
      uid: 1003,
      name: "テストC",
      buffs: [
        { kind: "Tina", baseId: 1, buffUuid: 102, layer: 1, remainingMs: 900, durationMs: 10000, receivedAtMs: now - 9100 },
      ],
    },
  ],
};
