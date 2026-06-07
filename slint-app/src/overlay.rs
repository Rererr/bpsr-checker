//! ウィンドウのネイティブ操作（winit へ降りる）。
//! クリックスルー切替・モニタ列挙・ドラッグ移動を提供する。

use i_slint_backend_winit::WinitWindowAccessor;

/// 物理座標系でのモニタ矩形。
#[derive(Debug, Clone)]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
    pub name: String,
    pub primary: bool,
}

/// クリックスルー切替。`enabled=true` で背後へクリックを素通しさせる。
pub fn set_click_through(window: &slint::Window, enabled: bool) {
    window.with_winit_window(|w| {
        if let Err(e) = w.set_cursor_hittest(!enabled) {
            log::warn!("set_cursor_hittest failed: {e}");
        }
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
