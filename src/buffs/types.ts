export type CharKind = "Tina" | "Aluna" | "Tarta" | "Basilisk";

export interface SelfBuffSnapshot {
  kind: CharKind;
  baseId: number;
  buffUuid: number;
  layer: number;
  remainingMs: number;
  durationMs: number;
  receivedAtMs: number;
}
