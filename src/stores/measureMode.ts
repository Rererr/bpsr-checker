import { createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type { EncounterSnapshot } from "./encounter";
import { resetPlayerWindows } from "./encounter";

export interface MeasureModeStatus {
  kind: "normal" | "pending" | "active";
  remainingMs?: number;
  durationMs?: number;
  armedAtMs?: number;
}

const [measureModeStatus, setMeasureModeStatus] = createSignal<MeasureModeStatus>({ kind: "normal" });
const [threeMinResult, setThreeMinResult] = createSignal<EncounterSnapshot | null>(null);

export { measureModeStatus, threeMinResult };

export async function fetchMeasureModeStatus(): Promise<void> {
  const status = await invoke<MeasureModeStatus>("get_measure_mode_status").catch(() => ({ kind: "normal" as const }));
  setMeasureModeStatus(status);
  if (status.kind === "active" && status.remainingMs !== undefined && status.remainingMs <= 0) {
    await invoke("finalize_3min_measure_mode").catch(() => {});
  }
}

export async function start3Min(durationSecs: number): Promise<void> {
  try {
    await invoke("start_3min_measure_mode", { durationSecs });
  } catch (e) {
    console.error("start_3min_measure_mode failed:", e);
    return;
  }
  resetPlayerWindows();
  await fetchMeasureModeStatus();
}

export async function cancel3Min(): Promise<void> {
  try {
    await invoke("cancel_3min_measure_mode");
  } catch (e) {
    console.error("cancel_3min_measure_mode failed:", e);
    return;
  }
  setMeasureModeStatus({ kind: "normal" });
  resetPlayerWindows();
}

export function closeThreeMinResult(): void {
  setThreeMinResult(null);
}

export async function wireMeasureMode(): Promise<UnlistenFn> {
  return await listen<EncounterSnapshot>("3min-measure-finalized", (event) => {
    setMeasureModeStatus({ kind: "normal" });
    setThreeMinResult(event.payload);
    resetPlayerWindows();
  });
}
