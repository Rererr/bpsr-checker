//! グローバルショートカット（Windows専用・issue #3）。
//! テーブル駆動で「アクション定義・キー表現・キャプチャ規則・GlobalHotKeyManager 配線」を
//! 一箇所にまとめる。tray.rs と同じくモジュール全体を Windows 前提とし、main.rs 側で
//! `#[cfg(windows)]` ガードして使う。
//!
//! 保存形式＝表示形式は "Ctrl+Shift+R" のような文字列（空文字＝未割当）。
//! Slint KeyEvent(char) → Code、Code → 表示/保存文字列、保存文字列 → Code の3方向は
//! `SPECIAL_KEYS` 1つのテーブルから導出する（`HotKey::from_str` は使わない。Slint 側の
//! キー判定と文字列⇔Code変換が二重管理になるため）。

use crate::settings::Settings;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use slint::platform::Key;

/// 割当可能なアクション（テーブル駆動）。追加時は下の `ACTIONS` 配列にも追記すること。
/// 各 match は網羅チェックでコンパイルエラーになる（`_ =>` は書かない）が、`ACTIONS` への
/// 追記漏れ自体はコンパイラでは検出できないため、追加時は忘れず両方更新する。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShortcutAction {
    ResetEncounter,     // 計測を初期化
    TogglePause,        // 一時停止/再開
    ToggleMeasure,      // 3分計測
    CopyList,           // 一覧をクリップボードへコピー
    ToggleAlwaysOnTop,  // 常に最前面
}

/// UI の行順＝この配列順（app.slint の `shortcut-labels` と同じ並びにすること）。
pub const ACTIONS: [ShortcutAction; 5] = [
    ShortcutAction::ResetEncounter,
    ShortcutAction::TogglePause,
    ShortcutAction::ToggleMeasure,
    ShortcutAction::CopyList,
    ShortcutAction::ToggleAlwaysOnTop,
];

impl ShortcutAction {
    /// UI 行 index（0..ACTIONS.len()）からアクションを引く。
    pub fn from_index(i: usize) -> Option<Self> {
        ACTIONS.get(i).copied()
    }

    /// ACTIONS 内での index（UI 行との対応。shortcuts モデル・shortcut-labels と同じ添字）。
    /// ACTIONS への追記漏れは起動時の不変条件違反なので、0 へ握り潰さず即座に落とす
    /// （unwrap_or(0) だと追記漏れが「Reset行に他アクションのエラーが出る」という
    /// 追跡困難な症状に化けるため）。
    pub fn index(self) -> usize {
        ACTIONS.iter().position(|a| *a == self).expect("ShortcutAction must be listed in ACTIONS")
    }

    /// 保存/表示兼用文字列を Settings から読む（空 = 未割当）。
    pub fn key_text(self, s: &Settings) -> &str {
        match self {
            ShortcutAction::ResetEncounter => &s.hotkey_reset,
            ShortcutAction::TogglePause => &s.hotkey_pause,
            ShortcutAction::ToggleMeasure => &s.hotkey_measure,
            ShortcutAction::CopyList => &s.hotkey_copy,
            ShortcutAction::ToggleAlwaysOnTop => &s.hotkey_aot,
        }
    }

    /// 保存/表示兼用文字列を Settings へ書く。
    pub fn set_key_text(self, s: &mut Settings, v: String) {
        match self {
            ShortcutAction::ResetEncounter => s.hotkey_reset = v,
            ShortcutAction::TogglePause => s.hotkey_pause = v,
            ShortcutAction::ToggleMeasure => s.hotkey_measure = v,
            ShortcutAction::CopyList => s.hotkey_copy = v,
            ShortcutAction::ToggleAlwaysOnTop => s.hotkey_aot = v,
        }
    }
}

// ─── キー表現（対応表は1つだけ） ──────────────────────────────────────

/// 特殊キー1件の対応。Slint キャプチャ用 char・global-hotkey の Code・表示/保存文字列の
/// 3方向すべてをこの1つのテーブルから導出する（private use area の値はハードコードせず、
/// `char::from(Key::F9)` のように `slint::platform::Key` から都度導出する）。
struct SpecialKey {
    slint_key: Key,
    code: Code,
    display: &'static str,
    /// 修飾キー無しでも単独割当を許可するか（F1〜F12 のみ true。誤爆防止のため他は false）。
    solo_ok: bool,
}

const SPECIAL_KEYS: &[SpecialKey] = &[
    SpecialKey { slint_key: Key::F1, code: Code::F1, display: "F1", solo_ok: true },
    SpecialKey { slint_key: Key::F2, code: Code::F2, display: "F2", solo_ok: true },
    SpecialKey { slint_key: Key::F3, code: Code::F3, display: "F3", solo_ok: true },
    SpecialKey { slint_key: Key::F4, code: Code::F4, display: "F4", solo_ok: true },
    SpecialKey { slint_key: Key::F5, code: Code::F5, display: "F5", solo_ok: true },
    SpecialKey { slint_key: Key::F6, code: Code::F6, display: "F6", solo_ok: true },
    SpecialKey { slint_key: Key::F7, code: Code::F7, display: "F7", solo_ok: true },
    SpecialKey { slint_key: Key::F8, code: Code::F8, display: "F8", solo_ok: true },
    SpecialKey { slint_key: Key::F9, code: Code::F9, display: "F9", solo_ok: true },
    SpecialKey { slint_key: Key::F10, code: Code::F10, display: "F10", solo_ok: true },
    SpecialKey { slint_key: Key::F11, code: Code::F11, display: "F11", solo_ok: true },
    SpecialKey { slint_key: Key::F12, code: Code::F12, display: "F12", solo_ok: true },
    SpecialKey { slint_key: Key::Space, code: Code::Space, display: "Space", solo_ok: false },
    SpecialKey { slint_key: Key::Return, code: Code::Enter, display: "Enter", solo_ok: false },
    SpecialKey { slint_key: Key::Tab, code: Code::Tab, display: "Tab", solo_ok: false },
    SpecialKey { slint_key: Key::Backspace, code: Code::Backspace, display: "Backspace", solo_ok: false },
    SpecialKey { slint_key: Key::Delete, code: Code::Delete, display: "Delete", solo_ok: false },
    SpecialKey { slint_key: Key::Insert, code: Code::Insert, display: "Insert", solo_ok: false },
    SpecialKey { slint_key: Key::Home, code: Code::Home, display: "Home", solo_ok: false },
    SpecialKey { slint_key: Key::End, code: Code::End, display: "End", solo_ok: false },
    SpecialKey { slint_key: Key::PageUp, code: Code::PageUp, display: "PageUp", solo_ok: false },
    SpecialKey { slint_key: Key::PageDown, code: Code::PageDown, display: "PageDown", solo_ok: false },
    SpecialKey { slint_key: Key::UpArrow, code: Code::ArrowUp, display: "Up", solo_ok: false },
    SpecialKey { slint_key: Key::DownArrow, code: Code::ArrowDown, display: "Down", solo_ok: false },
    SpecialKey { slint_key: Key::LeftArrow, code: Code::ArrowLeft, display: "Left", solo_ok: false },
    SpecialKey { slint_key: Key::RightArrow, code: Code::ArrowRight, display: "Right", solo_ok: false },
    SpecialKey { slint_key: Key::Pause, code: Code::Pause, display: "Pause", solo_ok: false },
    SpecialKey { slint_key: Key::ScrollLock, code: Code::ScrollLock, display: "ScrollLock", solo_ok: false },
];

const LETTER_CODES: [Code; 26] = [
    Code::KeyA, Code::KeyB, Code::KeyC, Code::KeyD, Code::KeyE, Code::KeyF, Code::KeyG,
    Code::KeyH, Code::KeyI, Code::KeyJ, Code::KeyK, Code::KeyL, Code::KeyM, Code::KeyN,
    Code::KeyO, Code::KeyP, Code::KeyQ, Code::KeyR, Code::KeyS, Code::KeyT, Code::KeyU,
    Code::KeyV, Code::KeyW, Code::KeyX, Code::KeyY, Code::KeyZ,
];

const DIGIT_CODES: [Code; 10] = [
    Code::Digit0, Code::Digit1, Code::Digit2, Code::Digit3, Code::Digit4,
    Code::Digit5, Code::Digit6, Code::Digit7, Code::Digit8, Code::Digit9,
];

/// 修飾キー単体の text か（Slint は修飾キー単体でも key-pressed を飛ばすため無視して待機継続する）。
fn is_modifier_only(ch: char) -> bool {
    const MODS: [Key; 9] = [
        Key::Control, Key::ControlR, Key::Shift, Key::ShiftR,
        Key::Alt, Key::AltGr, Key::Meta, Key::MetaR, Key::CapsLock,
    ];
    MODS.iter().any(|k| char::from(*k) == ch)
}

/// 表示/保存文字列の最後のトークン（キー本体）→ (Code, solo_ok)。SPECIAL_KEYS と文字/数字1文字
/// から解決する。solo_ok は修飾キー無し単独割当を許可するか（F1〜F12 のみ true）。
/// capture_key（キャプチャ時点の判定）と parse_saved（保存値の再検証）の両方が、ここで返す
/// solo_ok を has_required_modifier() へ渡す（必須修飾の判定式を二重管理しない。W-A）。
fn key_from_display(s: &str) -> Option<(Code, bool)> {
    if let Some(sk) = SPECIAL_KEYS.iter().find(|k| k.display == s) {
        return Some((sk.code, sk.solo_ok));
    }
    let mut chars = s.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None; // 複数文字は特殊キー名のはずなので上でヒットしていない = 不明
    }
    if c.is_ascii_uppercase() {
        return Some((LETTER_CODES[(c as u8 - b'A') as usize], false));
    }
    if c.is_ascii_digit() {
        return Some((DIGIT_CODES[(c as u8 - b'0') as usize], false));
    }
    None
}

/// 必須修飾（Ctrl/Alt/Win のいずれか）を満たしているか、または単独割当を許可されたキー
/// （solo_ok。F1〜F12）か。Shift は RegisterHotKey がシステム全体の通常入力（例 Shift+A の
/// 大文字）を奪うため、単独の必須修飾としては認めない（Ctrl 等と併用する追加修飾は可）。
/// capture_key（キャプチャ時点）と parse_saved（保存値の再検証。手編集や旧バージョンで
/// 保存された "Shift+A" 等を弾く）の両方がここから判定する（W-A）。
fn has_required_modifier(ctrl: bool, alt: bool, meta: bool, solo_ok: bool) -> bool {
    ctrl || alt || meta || solo_ok
}

/// parse_saved の失敗理由。呼び出し側（Hotkeys）でエラー文言を使い分けるために区別する（W-A）。
#[derive(Debug, PartialEq, Eq)]
enum SavedKeyError {
    /// キー名やトークン列を解決できない（未知のキー名・非正準順/重複した修飾など）。
    Unparseable,
    /// キー自体は解決できたが、必須修飾（Ctrl/Alt/Win のいずれか、または F1〜F12 単独）を欠く。
    MissingModifier,
}

/// 保存文字列 → HotKey（起動時のパース、および設定パネル上での保存値の再検証）。表示文字列と
/// 同一フォーマット（修飾子は Ctrl→Shift→Alt→Win の正準順、各1回まで）のみ受理する
/// （各プレフィックスを正準順に1回ずつしか剥がさないため、非正準順・重複した修飾は最後まで
/// 消費しきれず rest に残り、key_from_display で解決できず弾かれる）。順不同を許すと、
/// 文字列としては異なるのに OS 上は同一キーになる組（例 "Alt+Ctrl+A" と "Ctrl+Alt+A"）が
/// 生まれ、is_duplicate（文字列比較）をすり抜けて AlreadyRegistered が「他のアプリが
/// 使用中」という誤った原因表示になる（W-B）。生成側（mods_prefix）は常にこの順で作るため
/// キャプチャ経由の値との往復は保証される。
/// 必須修飾も capture_key と同じ has_required_modifier() から判定する（W-A。手編集や
/// 旧バージョンの保存値 "Shift+A" 等、キャプチャ経由では作れない値を弾く）。
fn parse_saved(s: &str) -> Result<HotKey, SavedKeyError> {
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut meta = false;
    let mut rest = s;
    if let Some(r) = rest.strip_prefix("Ctrl+") {
        ctrl = true;
        rest = r;
    }
    if let Some(r) = rest.strip_prefix("Shift+") {
        shift = true;
        rest = r;
    }
    if let Some(r) = rest.strip_prefix("Alt+") {
        alt = true;
        rest = r;
    }
    if let Some(r) = rest.strip_prefix("Win+") {
        meta = true;
        rest = r;
    }
    let (code, solo_ok) = key_from_display(rest).ok_or(SavedKeyError::Unparseable)?;
    if !has_required_modifier(ctrl, alt, meta, solo_ok) {
        return Err(SavedKeyError::MissingModifier);
    }
    let mut mods = Modifiers::empty();
    if ctrl {
        mods |= Modifiers::CONTROL;
    }
    if shift {
        mods |= Modifiers::SHIFT;
    }
    if alt {
        mods |= Modifiers::ALT;
    }
    if meta {
        mods |= Modifiers::SUPER;
    }
    Ok(HotKey::new(Some(mods), code))
}

/// 修飾子表記（Ctrl/Shift/Alt/Win の順で連結。例 "Ctrl+Shift+"）。
fn mods_prefix(ctrl: bool, shift: bool, alt: bool, meta: bool) -> String {
    let mut s = String::new();
    if ctrl {
        s.push_str("Ctrl+");
    }
    if shift {
        s.push_str("Shift+");
    }
    if alt {
        s.push_str("Alt+");
    }
    if meta {
        s.push_str("Win+");
    }
    s
}

// ─── キャプチャ判定（Rust側で判定する） ──────────────────────────────

/// `capture_key` の判定結果。
pub enum CaptureOutcome {
    /// 未確定のまま待機継続。`Some` の場合はその理由を一時的にエラー表示する
    /// （修飾キー無しの非Fキー等）。`None` は無視して黙って継続する
    /// （repeat・修飾キー単体・未対応char）。
    Continue(Option<&'static str>),
    /// Escape によるキャンセル（呼び出し側は元の割当を維持する）。
    Cancelled,
    /// 確定した保存/表示兼用文字列。呼び出し側でアプリ内重複（`is_duplicate`）を
    /// 確認してから settings へ採用すること。
    Captured(String),
}

/// FocusScope の `key-pressed` から渡された1イベントを判定する。
pub fn capture_key(
    text: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
    repeat: bool,
) -> CaptureOutcome {
    if repeat {
        return CaptureOutcome::Continue(None);
    }
    let Some(ch) = text.chars().next() else {
        return CaptureOutcome::Continue(None);
    };
    if ch == char::from(Key::Escape) {
        return CaptureOutcome::Cancelled;
    }
    if is_modifier_only(ch) {
        return CaptureOutcome::Continue(None);
    }

    // 特殊キー → 文字/数字キー の順で解決。どちらにも当たらなければ未対応として無視する。
    let resolved = if let Some(sk) = SPECIAL_KEYS.iter().find(|k| char::from(k.slint_key) == ch) {
        Some((sk.display.to_string(), sk.solo_ok))
    } else {
        let lower = ch.to_ascii_lowercase();
        if ('a'..='z').contains(&lower) {
            Some((lower.to_ascii_uppercase().to_string(), false))
        } else if ('0'..='9').contains(&ch) {
            Some((ch.to_string(), false))
        } else {
            None
        }
    };
    let Some((key_disp, solo_ok)) = resolved else {
        // 未対応キーも黙って無視せず、故障に見えないよう理由を出して待機継続する。
        return CaptureOutcome::Continue(Some(msg_parse_failed()));
    };

    // 必須修飾は Ctrl/Alt/Win のいずれか（F1〜F12 は solo_ok で単独許可）。判定は
    // parse_saved の保存値再検証と共通の has_required_modifier() から（同じ判定式を
    // 2箇所に書かない）。
    if !has_required_modifier(ctrl, alt, meta, solo_ok) {
        return CaptureOutcome::Continue(Some(msg_need_modifier()));
    }

    CaptureOutcome::Captured(format!("{}{}", mods_prefix(ctrl, shift, alt, meta), key_disp))
}

/// key_text が action 以外の既存割当と重複していないか（文字列比較。保存文字列は常に
/// 同一フォーマッタ経由で生成されるため文字列一致で十分）。
pub fn is_duplicate(settings: &Settings, action: ShortcutAction, key_text: &str) -> bool {
    ACTIONS.iter().any(|&a| a != action && a.key_text(settings) == key_text)
}

// ─── エラー文言（i18n） ──────────────────────────────────────────────
// 表示言語の判定は main.rs の is_ja() に一本化（tray.rs も同様）。private だが hotkey は
// main.rs の子モジュールなので crate::is_ja() で参照できる（同じ判定式を複数箇所に持たない）。

fn msg_already_registered() -> &'static str {
    if crate::is_ja() { "他のアプリが使用中です" } else { "In use by another app" }
}
fn msg_register_failed() -> &'static str {
    if crate::is_ja() { "登録できませんでした" } else { "Failed to register" }
}
fn msg_parse_failed() -> &'static str {
    if crate::is_ja() { "このキーは使えません" } else { "Unsupported key" }
}
/// アプリ内重複のエラー文言。main.rs 側のキャプチャ確定時（保存前）の即時表示にも使うため公開する。
pub fn msg_duplicate() -> &'static str {
    if crate::is_ja() { "他の項目と重複しています" } else { "Already used by another action" }
}
fn msg_manager_init_failed() -> &'static str {
    if crate::is_ja() {
        "ショートカット機能を初期化できませんでした"
    } else {
        "Could not initialize shortcuts"
    }
}
/// Ctrl/Alt/Win のいずれも押していない（Shift のみ、または修飾キー無し）非Fキーを
/// 捕まえたときの案内。Shift 単独を必須修飾として認めると RegisterHotKey がシステム全体の
/// 通常入力（例: Shift+A の大文字）を奪うため、必須修飾からは意図的に外している
/// （Shift はここに挙げた3つと併用する追加修飾としてのみ有効）。
fn msg_need_modifier() -> &'static str {
    if crate::is_ja() { "Ctrl / Alt / Win のいずれかと組み合わせてください" } else { "Combine with Ctrl, Alt, or Win" }
}

// ─── マネージャ ──────────────────────────────────────────────────────

/// グローバルショートカットの登録状態。`GlobalHotKeyManager` 生成に失敗した場合は
/// manager が None になり、以後 `apply()` は全アクションへ初期化失敗のエラーを積む
/// （機能全体が使えないことを UI から分かるようにするため）。
pub struct Hotkeys {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<(ShortcutAction, HotKey)>,
    errors: Vec<String>, // index = ShortcutAction::index()。空文字 = 正常
}

impl Hotkeys {
    /// winit イベントループ稼働後・同一スレッドで呼ぶこと
    /// （Windows の RegisterHotKey はメッセージループのあるスレッドでのみ有効）。
    pub fn new() -> Self {
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => Some(m),
            Err(e) => {
                log::warn!("global hotkey manager init failed: {e}");
                None
            }
        };
        Self { manager, registered: Vec::new(), errors: vec![String::new(); ACTIONS.len()] }
    }

    /// 全ホットキーを unregister し `registered` を空にする（呼び出し側が `apply()` するまで
    /// 何も登録されていない状態にする）。ダイアログでキーをキャプチャする間、割当済みキーが
    /// RegisterHotKey にシステム側で吸収されて FocusScope の key-pressed へ届かず、代わりに
    /// 本番アクションが誤発火する問題を避けるために呼ぶ（呼び出し側は必ずダイアログを
    /// 閉じる際に `apply()` して復帰させること）。
    pub fn suspend(&mut self) {
        let Some(mgr) = &self.manager else {
            self.registered.clear();
            return;
        };
        // unregister 失敗分は queue に残し、OS側に登録が残ったままアプリから追跡不能になる
        // （次回 apply() まで解放不能）事態を避け、次の apply() で再度 unregister を試みる。
        // 残留分は OS 上まだ有効（押せば実際に発火する）ので、poll() の逆引き対象
        // （registered）にもそのまま残す。UnregisterHotKey は同一スレッド・自前 hwnd の
        // 組合せで呼ぶため実質失敗せず、この分岐の到達性は極めて低い。
        let mut still_registered = Vec::new();
        for (action, hk) in self.registered.drain(..) {
            if let Err(e) = mgr.unregister(hk) {
                log::warn!("hotkey unregister failed for {action:?}: {e}");
                still_registered.push((action, hk));
            }
        }
        self.registered = still_registered;
    }

    /// register を試みない範囲（マネージャ未生成・アプリ内重複・パース可否・必須修飾）で
    /// エラー文言を計算する。apply()（登録前のフィルタ・登録失敗の上書き元）と revalidate()
    /// （登録なしの再計算。suspend 状態のまま呼べる）の両方から呼び、エラー計算ロジックを
    /// 二重管理しない（C1 / W-A）。
    fn compute_static_errors(&self, settings: &Settings) -> Vec<String> {
        let mut errors = vec![String::new(); ACTIONS.len()];
        if self.manager.is_none() {
            let msg = msg_manager_init_failed().to_string();
            for e in errors.iter_mut() {
                *e = msg.clone();
            }
            return errors;
        }
        for action in ACTIONS {
            let text = action.key_text(settings);
            if text.is_empty() {
                continue;
            }
            if is_duplicate(settings, action, text) {
                errors[action.index()] = msg_duplicate().to_string();
                continue;
            }
            if let Err(e) = parse_saved(text) {
                errors[action.index()] = match e {
                    SavedKeyError::MissingModifier => msg_need_modifier().to_string(),
                    SavedKeyError::Unparseable => msg_parse_failed().to_string(),
                };
            }
        }
        errors
    }

    /// 登録を伴わない静的チェックだけでエラー文言を再計算する（suspend 状態は変えない・
    /// register を一切試みない）。ダイアログ表示中に1行だけ確定/解除した直後はこちらを呼ぶ
    /// こと。apply() は内部で suspend() → 全行 register をやり直すため、ダイアログを
    /// 開いた瞬間に行った「開いている間は全ホットキー未登録にする」前提を崩してしまう
    /// （レビュー実測: ダイアログを開いたまま1行確定しただけで、別プロセスからの
    /// RegisterHotKey プローブが AlreadyRegistered になった＝suspend 中のはずが実際には
    /// グローバル登録されていた。C1）。実際に OS へ登録するまで確定しない
    /// AlreadyRegistered/RegisterFailed は suspend 中は判定できないため、該当行は次の
    /// apply()（ダイアログを閉じた時）まで更新されない。
    pub fn revalidate(&mut self, settings: &Settings) {
        self.errors = self.compute_static_errors(settings);
    }

    /// 全ホットキーを unregister（`suspend()`）→ settings の内容で register し直す（多重登録を防ぐ）。
    /// 実際に OS へ登録するのはここだけ（C1）。ダイアログ表示中の1行確定/解除では
    /// revalidate() を使い、ダイアログを閉じるまでここは呼ばないこと。
    pub fn apply(&mut self, settings: &Settings) {
        self.suspend();

        let mut errors = self.compute_static_errors(settings);
        let Some(mgr) = self.manager.as_ref() else {
            self.errors = errors;
            return;
        };

        for action in ACTIONS {
            let text = action.key_text(settings);
            if text.is_empty() || !errors[action.index()].is_empty() {
                continue; // 未割当、またはアプリ内重複/パース不可で既にエラー確定済みの行は登録を試みない
            }
            let Ok(hk) = parse_saved(text) else {
                // compute_static_errors で弾かれなかった値がここで失敗するのはロジックの
                // ずれを意味する。握り潰さず警告だけ残す（本来到達しないはず）。
                log::warn!("hotkey parse_saved unexpectedly failed for {action:?}: {text:?}");
                continue;
            };
            match mgr.register(hk) {
                Ok(()) => self.registered.push((action, hk)),
                Err(global_hotkey::Error::AlreadyRegistered(_)) => {
                    errors[action.index()] = msg_already_registered().to_string();
                }
                Err(e) => {
                    log::warn!("hotkey register failed for {action:?}: {e}");
                    errors[action.index()] = msg_register_failed().to_string();
                }
            }
        }
        self.errors = errors;
    }

    /// アクション毎のエラー文言（空文字 = 正常）。
    pub fn error_for(&self, action: ShortcutAction) -> &str {
        self.errors.get(action.index()).map(String::as_str).unwrap_or("")
    }

    /// 発火した（Pressedのみ）アクションを汲む。呼び出し側で対応する既存コールバックを invoke する。
    pub fn poll(&self) -> Vec<ShortcutAction> {
        let mut fired = Vec::new();
        while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
            if ev.state() != HotKeyState::Pressed {
                continue;
            }
            if let Some((action, _)) = self.registered.iter().find(|(_, hk)| hk.id() == ev.id()) {
                fired.push(*action);
            }
        }
        fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ACTIONS への追記漏れ・重複は「行が出ない」「別行にエラーが出る」という追跡困難な
    // 症状に化けるため、全 variant を ACTIONS とは独立に手で列挙して突き合わせる。
    // 新しい ShortcutAction を追加したらこの配列にも追記すること。
    const ALL_VARIANTS: [ShortcutAction; 5] = [
        ShortcutAction::ResetEncounter,
        ShortcutAction::TogglePause,
        ShortcutAction::ToggleMeasure,
        ShortcutAction::CopyList,
        ShortcutAction::ToggleAlwaysOnTop,
    ];

    #[test]
    fn all_actions_are_listed_in_actions_exactly_once() {
        // 件数不一致 = ACTIONS 側の重複または欠落。
        assert_eq!(
            ACTIONS.len(),
            ALL_VARIANTS.len(),
            "ACTIONS の件数が ShortcutAction の variant 数と一致していません（重複または欠落）"
        );
        // 各 variant が自分自身の index で ACTIONS から引けること（欠落していれば
        // index() の expect がここより先にパニックする）。
        for a in ALL_VARIANTS {
            assert_eq!(ACTIONS[a.index()], a, "{a:?} が ACTIONS 内の自身の index に対応していません");
        }
    }

    // W-A: 手編集・旧バージョンの保存値に含まれうる「必須修飾なし」を拒否する。
    #[test]
    fn parse_saved_rejects_shift_only_modifier() {
        assert_eq!(parse_saved("Shift+A"), Err(SavedKeyError::MissingModifier));
    }

    #[test]
    fn parse_saved_rejects_no_modifier_non_function_key() {
        assert_eq!(parse_saved("A"), Err(SavedKeyError::MissingModifier));
    }

    #[test]
    fn parse_saved_accepts_required_modifier() {
        assert!(parse_saved("Ctrl+A").is_ok());
        assert!(parse_saved("Alt+A").is_ok());
        assert!(parse_saved("Win+A").is_ok());
    }

    #[test]
    fn parse_saved_accepts_function_key_without_modifier() {
        assert!(parse_saved("F1").is_ok());
    }

    // W-B: 正準順（Ctrl→Shift→Alt→Win）以外・修飾の重複は拒否する
    // （許すと文字列は別でも OS 上は同一キーになり is_duplicate をすり抜けてしまうため）。
    #[test]
    fn parse_saved_rejects_non_canonical_modifier_order() {
        assert_eq!(parse_saved("Alt+Ctrl+A"), Err(SavedKeyError::Unparseable));
    }

    #[test]
    fn parse_saved_rejects_duplicated_modifier() {
        assert_eq!(parse_saved("Ctrl+Ctrl+A"), Err(SavedKeyError::Unparseable));
    }

    #[test]
    fn parse_saved_accepts_canonical_multi_modifier_order() {
        assert!(parse_saved("Ctrl+Shift+Alt+Win+A").is_ok());
    }
}
