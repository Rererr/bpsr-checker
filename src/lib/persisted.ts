import { createSignal, createEffect } from "solid-js";

export function persisted<T>(key: string, defaultValue: T): [() => T, (v: T) => void] {
  const storageKey = `bpsr.settings.${key}`;
  let initial = defaultValue;
  try { const s = localStorage.getItem(storageKey); if (s !== null) initial = JSON.parse(s) as T; } catch {}
  const [get, set] = createSignal<T>(initial);
  createEffect(() => { try { localStorage.setItem(storageKey, JSON.stringify(get())); } catch {} });
  return [get, (v: T) => set(() => v)];
}
