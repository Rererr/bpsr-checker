//! ウィンドウのネイティブ操作（winit へ降りる）。
//! クリックスルー切替・モニタ列挙・ドラッグ移動を提供する。

use i_slint_backend_winit::WinitWindowAccessor;

/// 物理座標系でのモニタ矩形。
#[derive(Debug, Clone)]
#[allow(dead_code)] // scale/name/primary は S4 のモニタ診断・配置で使用予定
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
    pub name: String,
    pub primary: bool,
}

/// no-frame 窓の縁ドラッグでサイズ変更を開始する（マウス押下中に呼ぶ）。
/// dir: 0=N 1=S 2=E 3=W 4=NE 5=NW 6=SE 7=SW
pub fn start_resize(window: &slint::Window, dir: i32) {
    use i_slint_backend_winit::winit::window::ResizeDirection;
    let direction = match dir {
        0 => ResizeDirection::North,
        1 => ResizeDirection::South,
        2 => ResizeDirection::East,
        3 => ResizeDirection::West,
        4 => ResizeDirection::NorthEast,
        5 => ResizeDirection::NorthWest,
        6 => ResizeDirection::SouthEast,
        7 => ResizeDirection::SouthWest,
        _ => return,
    };
    window.with_winit_window(|w| {
        if let Err(e) = w.drag_resize_window(direction) {
            log::warn!("drag_resize_window failed: {e}");
        }
    });
}

/// クリックスルー切替。`enabled=true` で背後へクリックを素通しさせる。
#[allow(dead_code)] // S4 のオーバーレイで使用予定
pub fn set_click_through(window: &slint::Window, enabled: bool) {
    window.with_winit_window(|w| {
        if let Err(e) = w.set_cursor_hittest(!enabled) {
            log::warn!("set_cursor_hittest failed: {e}");
        }
    });
}

/// メイン/オーバーレイ共通のタスクバー常駐モード適用（Windows）。
/// `show=true` で `WS_EX_APPWINDOW` + `WS_MINIMIZEBOX` を付与し、OS にアプリ窓として
/// 管理させる（最小化→タスクバー→復帰が正しく動く）。`false` でトレイ格納モード。
///
/// winit の `set_skip_taskbar` は `ITaskbarList::AddTab` でタスクバーに出すだけで
/// ウィンドウスタイルを変えないため、frameless(WS_POPUP) 窓を最小化すると Windows が
/// タブを除去して復帰不能（＝終了したように見える）になる。拡張スタイルで補う。
#[cfg(target_os = "windows")]
pub fn apply_taskbar_mode(window: &slint::Window, show: bool) {
    use i_slint_backend_winit::winit::platform::windows::WindowExtWindows;
    use i_slint_backend_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW, IsWindowVisible, SW_HIDE, SW_SHOWNA,
        SetWindowLongPtrW, ShowWindow, WS_EX_APPWINDOW, WS_MINIMIZEBOX, WS_SYSMENU,
    };
    window.with_winit_window(|w| {
        let Ok(handle) = w.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(h) = handle.as_raw() else {
            return;
        };
        let hwnd = HWND(h.hwnd.get());
        unsafe {
            // 最小化を機能させる基本スタイル（frameless でも装飾は描画されない）。
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            let want = style | (WS_MINIMIZEBOX.0 | WS_SYSMENU.0) as isize;
            if want != style {
                SetWindowLongPtrW(hwnd, GWL_STYLE, want);
            }
            // タスクバーへ「アプリ窓」として登録/解除。拡張スタイルの反映には再表示が要る。
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let new_ex = if show {
                ex | WS_EX_APPWINDOW.0 as isize
            } else {
                ex & !(WS_EX_APPWINDOW.0 as isize)
            };
            if new_ex != ex {
                let visible = IsWindowVisible(hwnd).as_bool();
                if visible {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex);
                if visible {
                    let _ = ShowWindow(hwnd, SW_SHOWNA);
                }
            }
        }
        // winit のタブ管理は再表示後に。トレイ格納時は DeleteTab でタスクバーから消す。
        w.set_skip_taskbar(!show);
    });
}

/// ウィンドウを OS 最小化する（タスクバー常駐モードの最小化ボタン用）。
/// `apply_taskbar_mode` で WS_MINIMIZEBOX/WS_EX_APPWINDOW 付与済みの前提。
#[cfg(target_os = "windows")]
pub fn minimize_window(window: &slint::Window) {
    use i_slint_backend_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SW_MINIMIZE, ShowWindow};
    window.with_winit_window(|w| {
        if let Ok(handle) = w.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                unsafe {
                    let _ = ShowWindow(HWND(h.hwnd.get()), SW_MINIMIZE);
                }
            }
        }
    });
}

/// 非 Windows では winit の最小化にフォールバック（実機は Windows のみ）。
#[cfg(not(target_os = "windows"))]
pub fn minimize_window(window: &slint::Window) {
    window.with_winit_window(|w| w.set_minimized(true));
}

/// 非表示/最小化からウィンドウを復帰させ前面化する（トレイ復帰口・タスクバー復帰用）。
pub fn restore_window(window: &slint::Window) {
    window.with_winit_window(|w| {
        w.set_minimized(false);
        w.set_visible(true);
        w.focus_window();
    });
}

/// no-frame 窓のドラッグ移動を開始する（マウス押下イベント中に呼ぶ）。
pub fn start_drag(window: &slint::Window) {
    window.with_winit_window(|w| {
        if let Err(e) = w.drag_window() {
            log::warn!("drag_window failed: {e}");
        }
    });
}

/// 接続中モニタを物理座標で列挙する。show() 後に呼ぶこと。
pub fn monitors(window: &slint::Window) -> Vec<MonitorRect> {
    let mut out = Vec::new();
    window.with_winit_window(|w| {
        let primary = w.primary_monitor();
        for m in w.available_monitors() {
            let p = m.position();
            let s = m.size();
            let is_primary = primary
                .as_ref()
                .map(|pm| pm.position() == p && pm.size() == s)
                .unwrap_or(false);
            out.push(MonitorRect {
                x: p.x,
                y: p.y,
                w: s.width,
                h: s.height,
                scale: m.scale_factor(),
                name: m.name().unwrap_or_else(|| "<unknown>".into()),
                primary: is_primary,
            });
        }
    });
    out
}
