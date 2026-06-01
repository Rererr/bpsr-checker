import { createSignal, createEffect } from "solid-js";

function isSameKind<T>(parsed: unknown, defaultValue: T): parsed is T {
  if (defaultValue === null) return parsed === null;
  if (Array.isArray(defaultValue)) return Array.isArray(parsed);
  if (typeof defaultValue === "object") return typeof parsed === "object" && parsed !== null && !Array.isArray(parsed);
  return typeof parsed === typeof defaultValue;
}

export function persisted<T>(key: string, defaultValue: T): [() => T, (v: T) => void] {
  const storageKey = `bpsr.settings.${key}`;
  let initial = defaultValue;
  try {
    const s = localStorage.getItem(storageKey);
    if (s !== null) {
      const parsed: unknown = JSON.parse(s);
      initial = isSameKind(parsed, defaultValue) ? parsed : defaultValue;
    }
  } catch {}
  const [get, set] = createSignal<T>(initial);
  createEffect(() => { try { localStorage.setItem(storageKey, JSON.stringify(get())); } catch {} });
  return [get, (v: T) => set(() => v)];
}
