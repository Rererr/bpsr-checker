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
/// 最小サイズを満たすために縮めることはしない（モニタに収まらない分だけはモニタ内へ縮める）。
///
/// 最小化中・最大化中は何もしない。最小化中は位置が (-32000,-32000) を返し、モニタ内へ
/// 補正した SetWindowPos を最小化中の窓へ出してしまう。最大化中は位置が枠の分だけ負になりうり
/// （未実測）、そのまま set_position/request_inner_size を出すと最大化が解除されてしまう。
pub fn grow_to_min(window: &slint::Window, min_w: u32, min_h: u32) {
    let cur = window.size();
    if cur.width >= min_w && cur.height >= min_h {
        return;
    }
    // アプリは ShowWindow(SW_MINIMIZE) で直接最小化するため、Slint の状態でなく winit に問う。
    let min_or_max = window.with_winit_window(|w| w.is_minimized() == Some(true) || w.is_maximized());
    if min_or_max == Some(true) {
        return;
    }
    let pos = window.position();
    let current = WinRect { x: pos.x, y: pos.y, w: cur.width, h: cur.height };
    let monitors = crate::overlay::monitors(window);
    let Some(target) = grow_target(&current, min_w, min_h, &monitors) else {
        return;
    };
    if (target.x, target.y) != (current.x, current.y) {
        window.set_position(PhysicalPosition::new(target.x, target.y));
    }
    enforce_size(window, &target);
}

/// `current` を最小サイズまで広げ、最も重なるモニタ内へ収めた矩形を返す。
/// 変更が不要なら None。最小幅がモニタより広いとモニタ幅で頭打ちになり
/// `current` のままになるため、その場合も None を返して毎tick の SetWindowPos を避ける。
fn grow_target(current: &WinRect, min_w: u32, min_h: u32, monitors: &[MonitorRect]) -> Option<WinRect> {
    let mut target = WinRect {
        w: current.w.max(min_w),
        h: current.h.max(min_h),
        ..current.clone()
    };
    if let Some(m) = best_monitor(&target, monitors) {
        target = clamp_to_monitor(&target, m);
    }
    (target != *current).then_some(target)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(x: i32, w: u32) -> MonitorRect {
        MonitorRect { x, y: 0, w, h: 1080, scale: 1.0, name: String::new(), primary: x == 0 }
    }

    fn rect(x: i32, y: i32, w: u32, h: u32) -> WinRect {
        WinRect { x, y, w, h }
    }

    #[test]
    fn grow_target_widens_to_min() {
        let got = grow_target(&rect(100, 100, 432, 420), 474, 300, &[mon(0, 1920)]);
        assert_eq!(got, Some(rect(100, 100, 474, 420)));
    }

    #[test]
    fn grow_target_none_when_already_wide_enough() {
        assert_eq!(grow_target(&rect(100, 100, 800, 420), 474, 300, &[mon(0, 1920)]), None);
    }

    #[test]
    fn grow_target_shifts_left_at_monitor_edge() {
        let got = grow_target(&rect(1500, 100, 400, 420), 474, 300, &[mon(0, 1920)]);
        assert_eq!(got, Some(rect(1446, 100, 474, 420)));
    }

    #[test]
    fn grow_target_none_when_min_exceeds_monitor_and_already_capped() {
        // 最小幅がモニタより広いとモニタ幅で頭打ちになる。頭打ち済みなら毎tick 動かさない。
        assert_eq!(grow_target(&rect(0, 0, 1280, 420), 1500, 300, &[mon(0, 1280)]), None);
    }
}
