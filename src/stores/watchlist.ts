import { createSignal, createEffect } from "solid-js";

function makePersisted(key: string) {
  function load(): number[] {
    try {
      const s = localStorage.getItem(key);
      return s ? (JSON.parse(s) as number[]) : [];
    } catch {
      return [];
    }
  }
  const [sig, set] = createSignal<number[]>(load());
  createEffect(() => {
    try {
      localStorage.setItem(key, JSON.stringify(sig()));
    } catch {}
  });
  // storage イベントを購読してウィンドウ間の変更をシグナルに反映する。
  // persisted.ts は同一タブ内のシグナル書き込みのみを担うが、Tauriでは
  // メインウィンドウとバフオーバーレイウィンドウが同じ localStorage を共有するため、
  // 別ウィンドウからウォッチリストが変更された際にも即座に反映する必要がある。
  // このリスナーはモジュール初期化時に一度だけ登録され、アプリのライフサイクル全体で
  // 有効であることが前提のため removeEventListener による cleanup は不要。
  window.addEventListener("storage", (e) => {
    if (e.key === key && e.newValue !== null) {
      try {
        set(JSON.parse(e.newValue) as number[]);
      } catch {}
    }
  });
  return [sig, set] as const;
}

const MAX_WATCHLIST = 20;

// uid 追加順を保持するため配列で管理
const [_watched, setWatched] = makePersisted("bpsr.watchlist");
const [_excluded, setExcluded] = makePersisted("bpsr.watchlist.excluded");

export const watchedUids = () => _watched();
export const isWatched = (uid: number) => _watched().includes(uid);

export function toggleWatch(uid: number): void {
  setWatched((prev) =>
    prev.includes(uid) ? prev.filter((u) => u !== uid) : [...prev, uid]
  );
}

// local player を先頭にシードする（未追加・上限未達の場合のみ）
export function seedLocalPlayer(uid: number): void {
  if (uid === 0) return;
  if (_excluded().includes(uid)) return;
  setWatched((prev) => {
    if (prev.includes(uid)) return prev;
    if (prev.length >= MAX_WATCHLIST) return prev;
    return [uid, ...prev];
  });
}

export function clearWatchlist(): void {
  setWatched([]);
  setExcluded([]);
}

export function removeFromWatchlist(uid: number): void {
  setWatched((prev) => prev.filter((u) => u !== uid));
  setExcluded((prev) => (prev.includes(uid) ? prev : [...prev, uid]));
}

export function bulkAddPlayers(uids: number[]): void {
  setWatched((prev) => {
    if (prev.length >= MAX_WATCHLIST) return prev;
    const excluded = _excluded();
    const merged = [...prev];
    let changed = false;
    for (const uid of uids) {
      if (merged.length >= MAX_WATCHLIST) break;
      if (!merged.includes(uid) && !excluded.includes(uid)) {
        merged.push(uid);
        changed = true;
      }
    }
    return changed ? merged : prev;
  });
}
