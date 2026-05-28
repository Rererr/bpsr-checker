import { createSignal, createEffect } from "solid-js";

const STORAGE_KEY = "bpsr.watchlist";

function load(): number[] {
  try {
    const s = localStorage.getItem(STORAGE_KEY);
    return s ? (JSON.parse(s) as number[]) : [];
  } catch {
    return [];
  }
}

// uid 追加順を保持するため配列で管理
const [_watched, setWatched] = createSignal<number[]>(load());

createEffect(() => {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(_watched()));
  } catch {}
});

// 別ウィンドウ（BuffOverlay）からの変更を受け取る
window.addEventListener("storage", (e) => {
  if (e.key === STORAGE_KEY && e.newValue !== null) {
    try {
      setWatched(JSON.parse(e.newValue) as number[]);
    } catch {}
  }
});

export const watchedUids = () => _watched();
export const isWatched = (uid: number) => _watched().includes(uid);

export function toggleWatch(uid: number): void {
  setWatched((prev) =>
    prev.includes(uid) ? prev.filter((u) => u !== uid) : [...prev, uid]
  );
}

// local player を先頭にシードする（未追加の場合のみ）
export function seedLocalPlayer(uid: number): void {
  if (uid === 0) return;
  setWatched((prev) => (prev.includes(uid) ? prev : [uid, ...prev]));
}
