export type BuffCategory = "buff" | "debuff" | "recovery" | "item" | "unknown";
export type DisplayPriority = "hidden" | "low" | "normal" | "high" | "alert";

export interface StatusEntry {
  instanceId: number;
  baseId: number;
  category: BuffCategory;
  priority: DisplayPriority;
  remainingMs: number;
  durationMs: number;
  layer: number;
  sourceConfigId: number;
}

export interface SelfStatusData {
  buffs: StatusEntry[];
  debuffs: StatusEntry[];
  nowMs: number;
  localPlayerUid: number;
}

export interface BuffNameEntry {
  name: string;
  desc?: string;
}
