import { For } from "solid-js";
import type { JSX } from "solid-js";
import type { SelfBuffSnapshot } from "../stores/buffs";
import { getClassColor } from "../utils";
import { CircularBuff, CHAR_KINDS } from "./CircularBuff";
import type { CharKind } from "./CircularBuff";

interface PlayerBuffRowProps {
  uid: number;
  name: string;
  className: string;
  buffs: SelfBuffSnapshot[];
}

export function PlayerBuffRow(props: PlayerBuffRowProps): JSX.Element {
  const classColor = () => getClassColor(props.className);
  const getSnap = (kind: CharKind) =>
    props.buffs.find((b) => b.kind === kind) ?? null;

  return (
    <div
      style={{
        display: "flex",
        "align-items": "center",
        gap: "4px",
        padding: "2px 0",
      }}
    >
      <div
        title={props.name || `UID:${props.uid}`}
        style={{
          width: "54px",
          "flex-shrink": "0",
          "font-size": "10px",
          color: classColor(),
          overflow: "hidden",
          "text-overflow": "ellipsis",
          "white-space": "nowrap",
          "line-height": "28px",
        }}
      >
        {props.name || `${props.uid}`}
      </div>
      <For each={CHAR_KINDS}>
        {(kind) => (
          <div style={{ width: "48px", "flex-shrink": "0", display: "flex", "justify-content": "center" }}>
            <CircularBuff kind={kind} snap={getSnap(kind)} />
          </div>
        )}
      </For>
    </div>
  );
}
