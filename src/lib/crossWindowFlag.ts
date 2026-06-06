import { createSignal, onCleanup } from "solid-js";

/**
 * 別ウィンドウ(main)が `persisted()` で書き込んだ bool 設定を購読する。
 *
 * オーバーレイ(buffs/self_status)ウィンドウは常時表示のまま内容だけを出し分ける
 * 設計のため、main 側の ON/OFF を localStorage 経由で受け取る必要がある。
 * 同一オリジンの別ドキュメントでの変更は `storage` イベントで届く。
 *
 * （ウィンドウを hide/show すると混在DPIで main の透明合成が壊れるため、
 *   表示制御はこの仕組みで行う）
 */
export function crossWindowFlag(key: string, defaultValue = false): () => boolean {
  const storageKey = `bpsr.settings.${key}`;
  const read = (): boolean => {
    try {
      const s = localStorage.getItem(storageKey);
      if (s === null) return defaultValue;
      const v: unknown = JSON.parse(s);
      return typeof v === "boolean" ? v : defaultValue;
    } catch {
      return defaultValue;
    }
  };
  const [get, set] = createSignal(read());
  const onStorage = (e: StorageEvent) => {
    if (e.key === storageKey || e.key === null) set(read());
  };
  window.addEventListener("storage", onStorage);
  onCleanup(() => window.removeEventListener("storage", onStorage));
  return get;
}
