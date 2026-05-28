import { For, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { trackedBuffs, startBuffPolling, stopBuffPolling } from "../stores/buffs";
import { watchedUids } from "../stores/watchlist";
import { PlayerBuffRow } from "./PlayerBuffRow";
import { CHAR_KINDS, KIND_COLORS } from "./CircularBuff";
import type { CharKind } from "./CircularBuff";
import { t } from "../lib/i18n";
import type { TranslationKey } from "../lib/i18n";

const CHAR_LABEL_KEY: Record<CharKind, TranslationKey> = {
  Tina: "char_tina",
  Aluna: "char_aluna",
  Tarta: "char_tarta",
  Basilisk: "char_basilisk",
};

export function BuffOverlay(): JSX.Element {
  onMount(() => {
    startBuffPolling(200);
    onCleanup(() => stopBuffPolling());
  });

  const playerSnap = (uid: number) =>
    trackedBuffs().players.find((p) => p.uid === uid);

  return (
    <div
      data-tauri-drag-region
      style={{
        background: "rgba(10, 10, 18, 0.82)",
        "border-radius": "6px",
        padding: "10px 8px 4px",
        "min-height": "100vh",
        "max-height": "100vh",
        "overflow-y": "auto",
        "font-family": '"Segoe UI", "Meiryo", sans-serif',
        "scrollbar-width": "thin",
        "scrollbar-color": "rgba(255,255,255,0.15) transparent",
      }}
    >
      <Show
        when={watchedUids().length > 0}
        fallback={
          <div
            style={{
              padding: "8px 0",
              "font-size": "10px",
              color: "rgba(255,255,255,0.35)",
              "text-align": "center",
            }}
          >
            {t("buffs_empty_hint")}
          </div>
        }
      >
        {/* カラムヘッダー */}
        <div
          style={{
            display: "flex",
            "align-items": "center",
            gap: "4px",
            padding: "2px 0 3px 0",
            "border-bottom": "1px solid rgba(255,255,255,0.08)",
            "margin-bottom": "2px",
          }}
        >
          {/* 名前列のスペース確保 */}
          <div style={{ width: "54px", "flex-shrink": "0" }} />
          <For each={CHAR_KINDS}>
            {(kind) => (
              <div
                style={{
                  width: "48px",
                  "flex-shrink": "0",
                  "text-align": "center",
                  "font-size": "9px",
                  color: KIND_COLORS[kind],
                  "font-weight": "600",
                  "white-space": "nowrap",
                  "user-select": "none",
                }}
              >
                {t(CHAR_LABEL_KEY[kind])}
              </div>
            )}
          </For>
        </div>

        <For each={watchedUids()}>
          {(uid) => {
            const snap = () => playerSnap(uid);
            return (
              <PlayerBuffRow
                uid={uid}
                name={snap()?.name ?? ""}
                className=""
                buffs={snap()?.buffs ?? []}
              />
            );
          }}
        </For>
      </Show>
    </div>
  );
}
