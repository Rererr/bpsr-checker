import { For, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { selfStatus, startStatusPolling, stopStatusPolling } from "./store";
import { BuffIconCell } from "./BuffIconCell";
import { t } from "../lib/i18n";
import buffDict from "../lib/data/json/BuffName.ja.json";
import type { BuffNameEntry, StatusEntry } from "./types";

const nameDict = buffDict as Record<string, BuffNameEntry>;

const containerStyle: JSX.CSSProperties = {
  background: "rgba(10, 10, 18, 0.82)",
  "border-radius": "6px",
  padding: "6px 8px",
  "min-height": "100vh",
  "max-height": "100vh",
  "overflow-y": "auto",
  "font-family": '"Segoe UI", "Meiryo", sans-serif',
  "scrollbar-width": "thin",
  "scrollbar-color": "rgba(255,255,255,0.15) transparent",
};

const sectionLabelStyle: JSX.CSSProperties = {
  "font-size": "9px",
  color: "rgba(255,255,255,0.4)",
  "text-transform": "uppercase",
  "letter-spacing": "0.08em",
  "margin-bottom": "3px",
  "user-select": "none",
};

const rowStyle: JSX.CSSProperties = {
  display: "flex",
  "flex-wrap": "wrap",
  gap: "4px",
  "margin-bottom": "6px",
};

function Section(props: { label: string; entries: StatusEntry[]; nameDict: Record<string, BuffNameEntry> }): JSX.Element {
  return (
    <Show when={props.entries.length > 0}>
      <div>
        <div style={sectionLabelStyle}>{props.label}</div>
        <div style={rowStyle}>
          <For each={props.entries}>
            {(entry) => <BuffIconCell entry={entry} nameDict={props.nameDict} />}
          </For>
        </div>
      </div>
    </Show>
  );
}

export function SelfStatusOverlay(): JSX.Element {
  onMount(() => {
    startStatusPolling(200);
    onCleanup(() => stopStatusPolling());
  });

  const status = () => selfStatus();
  const isEmpty = () => status().buffs.length === 0 && status().debuffs.length === 0;
  const isWaiting = () => status().localPlayerUid === 0;

  return (
    <div data-tauri-drag-region style={containerStyle}>
      <Show
        when={!isWaiting()}
        fallback={
          <div style={{ "font-size": "10px", color: "rgba(255,255,255,0.3)", "text-align": "center", padding: "8px 0" }}>
            {t("self_status_waiting")}
          </div>
        }
      >
        <Show
          when={!isEmpty()}
          fallback={
            <div style={{ "font-size": "10px", color: "rgba(255,255,255,0.3)", "text-align": "center", padding: "8px 0" }}>
              {t("self_status_empty_hint")}
            </div>
          }
        >
          <Section label={t("self_buffs_section")} entries={status().buffs} nameDict={nameDict} />
          <Section label={t("self_debuffs_section")} entries={status().debuffs} nameDict={nameDict} />
        </Show>
      </Show>
    </div>
  );
}
