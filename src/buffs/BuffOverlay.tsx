import { For, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { trackedBuffs, startBuffPolling, stopBuffPolling } from "../stores/buffs";
import { watchedUids } from "../stores/watchlist";
import { dpsPlayers } from "../stores/encounter";
import { PlayerBuffRow } from "./PlayerBuffRow";
import { t } from "../lib/i18n";

export function BuffOverlay(): JSX.Element {
  onMount(() => {
    startBuffPolling(200);
    onCleanup(() => stopBuffPolling());
  });

  const getPlayerInfo = (uid: number) => {
    const rows = dpsPlayers().playerRows;
    const row = rows.find((r) => r.uid === uid);
    return {
      name: row?.name ?? "",
      className: row?.className ?? "",
    };
  };

  const buffsByUid = (uid: number) =>
    trackedBuffs().players.find((p) => p.uid === uid)?.buffs ?? [];

  return (
    <div
      data-tauri-drag-region
      style={{
        background: "rgba(10, 10, 18, 0.82)",
        "border-radius": "6px",
        padding: "4px 8px",
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
        <For each={watchedUids()}>
          {(uid) => {
            const info = () => getPlayerInfo(uid);
            return (
              <PlayerBuffRow
                uid={uid}
                name={info().name}
                className={info().className}
                buffs={buffsByUid(uid)}
              />
            );
          }}
        </For>
      </Show>
    </div>
  );
}
