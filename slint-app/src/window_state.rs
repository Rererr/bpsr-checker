//! ウィンドウ位置・サイズの保存/復元（物理座標）。
//! 復元時は現在のモニタ範囲と交差するか検査し、画面外なら既定モニタへ収め直す。

use crate::overlay::MonitorRect;
use i_slint_backend_winit::WinitWindowAccessor;
use serde::{Deserialize, Serialize};
use slint::PhysicalPosition;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WinRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub main: Option<WinRect>,
    pub buffs: Option<WinRect>,
    pub self_status: Option<WinRect>,
    pub stats: Option<WinRect>,
}

fn config_path() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base)
        .join("bpsr-checker")
        .join("window_layout.json")
}

pub fn load() -> Layout {
    match std::fs::read_to_string(config_path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Layout::default(),
    }
}

pub fn save(layout: &Layout) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(layout) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&path, s) {
                log::warn!("window_state write failed: {e}");
            }
        }
        Err(e) => log::warn!("window_state serialize failed: {e}"),
    }
}

pub fn capture(window: &slint::Window) -> WinRect {
    let p = window.position();
    let s = window.size();
    WinRect {
        x: p.x,
        y: p.y,
        w: s.width,
        h: s.height,
    }
}

fn overlap_area(a: &WinRect, m: &MonitorRect) -> i64 {
    let ix = (a.x + a.w as i32).min(m.x + m.w as i32) - a.x.max(m.x);
    let iy = (a.y + a.h as i32).min(m.y + m.h as i32) - a.y.max(m.y);
    if ix <= 0 || iy <= 0 {
        0
    } else {
        ix as i64 * iy as i64
    }
}

fn intersects_any(a: &WinRect, monitors: &[MonitorRect]) -> bool {
    monitors.iter().any(|m| overlap_area(a, m) > 0)
}

fn best_monitor<'a>(rect: &WinRect, monitors: &'a [MonitorRect]) -> Option<&'a MonitorRect> {
    monitors.iter().max_by_key(|m| overlap_area(rect, m))
}

fn clamp_to_monitor(r: &WinRect, m: &MonitorRect) -> WinRect {
    let w = r.w.min(m.w).max(120);
    let h = r.h.min(m.h).max(80);
    let max_x = (m.x + m.w as i32 - w as i32).max(m.x);
    let max_y = (m.y + m.h as i32 - h as i32).max(m.y);
    WinRect {
        x: r.x.clamp(m.x, max_x),
        y: r.y.clamp(m.y, max_y),
        w,
        h,
    }
}

fn default_rect(monitors: &[MonitorRect], idx: usize, size: (u32, u32)) -> WinRect {
    match monitors.get(idx).or_else(|| monitors.first()) {
        Some(m) => WinRect {
            x: m.x + 40,
            y: m.y + 40,
            w: size.0,
            h: size.1,
        },
        None => WinRect {
            x: 100,
            y: 100,
            w: size.0,
            h: size.1,
        },
    }
}

/// 保存値が有効（いずれかのモニタと交差）ならクランプして適用、
/// 無効/画面外なら `default_monitor` を基準にした既定位置へフォールバック。
pub fn restore(
    window: &slint::Window,
    saved: Option<&WinRect>,
    monitors: &[MonitorRect],
    default_monitor: usize,
    default_size: (u32, u32),
) -> WinRect {
    let rect = match saved {
        Some(r) if intersects_any(r, monitors) => r.clone(),
        _ => default_rect(monitors, default_monitor, default_size),
    };
    let rect = match best_monitor(&rect, monitors) {
        Some(m) => clamp_to_monitor(&rect, m),
        None => rect,
    };
    // 位置は Slint API で（混在DPIでも実績あり）。サイズは Slint の set_size だと
    // Window の preferred-width/height に上書きされて効かないため、winit の
    // request_inner_size で直接適用する（ドラッグリサイズと同じ経路＝確実に効く）。
    window.set_position(PhysicalPosition::new(rect.x, rect.y));
    enforce_size(window, &rect);
    log::info!("restore: saved={saved:?} applied rect={rect:?} (size は winit 適用)");
    rect
}

/// 現在の窓サイズが `min_w`/`min_h`（物理px、呼び出し側で MainWindow の
/// `layout-min-width`/`layout-min-height` に scale_factor を掛けて算出）を下回っていれば
/// 底上げする。settle 期間（`poll_window_settle`）は毎tick `enforce_size` で復元サイズを
/// 強制再適用しており、その経路（winit `request_inner_size`）は Slint のレイアウト制約を
/// 経由しないため、settle 完了までは狭すぎるサイズが残りうる。settle 完了後に呼び、
/// 最終的に正しい幅へ底上げする（呼び出し側で settle 完了を判定する）。
/// ユーザーが広げたサイズを縮める方向には関与しない。
pub fn grow_to_min(window: &slint::Window, min_w: u32, min_h: u32) {
    let cur = window.size();
    if cur.width >= min_w && cur.height >= min_h {
        return;
    }
    let pos = window.position();
    let mut target = WinRect {
        x: pos.x,
        y: pos.y,
        w: cur.width.max(min_w),
        h: cur.height.max(min_h),
    };
    let monitors = crate::overlay::monitors(window);
    if let Some(m) = best_monitor(&target, &monitors) {
        target = clamp_to_monitor(&target, m);
    }
    window.set_position(PhysicalPosition::new(target.x, target.y));
    enforce_size(window, &target);
}

/// 保存サイズを winit 経由で再適用（preferred 再アサートによる上書き対策）。
/// 現在サイズが一致していれば何もしない（チラつき・不要な OS 呼び出しを避ける）。
///
/// 注意: ここで使う winit の `request_inner_size`（`SetWindowPos` 相当）による
/// リサイズには Slint の最小サイズ制約が効かない。winit 0.30.13 の
/// `WM_WINDOWPOSCHANGING` ハンドラ（`platform_impl/windows/event_loop.rs`）は
/// 常に `DefWindowProcW` を呼ばず 0 を返すため、`DefWindowProcW` が本来行う
/// `WM_GETMINMAXINFO` 由来のサイズクランプが一切走らない。そのため呼び出し側
/// （`grow_to_min` 等）が明示的に最小サイズを確保する必要がある。
pub fn enforce_size(window: &slint::Window, target: &WinRect) {
    let cur = window.size();
    if cur.width == target.w && cur.height == target.h {
        return;
    }
    window.with_winit_window(|w| {
        let _ = w.request_inner_size(i_slint_backend_winit::winit::dpi::PhysicalSize::new(
            target.w, target.h,
        ));
    });
}
