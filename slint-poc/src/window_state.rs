//! ウィンドウ位置・サイズの保存/復元（物理座標）。
//! 復元時は現在のモニタ範囲と交差するか検査し、画面外なら既定モニタへ戻す。

use crate::overlay::MonitorRect;
use serde::{Deserialize, Serialize};
use slint::{PhysicalPosition, PhysicalSize};
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
    pub dps: Option<WinRect>,
    pub buffs: Option<WinRect>,
}

fn config_path() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base)
        .join("bpsr-checker-slint-poc")
        .join("window_layout.json")
}

pub fn load() -> Layout {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            eprintln!("[window_state] parse failed ({e}); using default");
            Layout::default()
        }),
        Err(_) => Layout::default(),
    }
}

pub fn save(layout: &Layout) {
    let path = config_path();
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("[window_state] create_dir_all failed: {e}");
            return;
        }
    }
    match serde_json::to_string_pretty(layout) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&path, s) {
                eprintln!("[window_state] write failed: {e}");
            }
        }
        Err(e) => eprintln!("[window_state] serialize failed: {e}"),
    }
}

/// 現在のウィンドウ位置・サイズを物理座標で取得。
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

fn overlaps(a: &WinRect, m: &MonitorRect) -> bool {
    let ax2 = a.x + a.w as i32;
    let ay2 = a.y + a.h as i32;
    let mx2 = m.x + m.w as i32;
    let my2 = m.y + m.h as i32;
    a.x < mx2 && ax2 > m.x && a.y < my2 && ay2 > m.y
}

fn intersects_any(a: &WinRect, monitors: &[MonitorRect]) -> bool {
    monitors.iter().any(|m| overlaps(a, m))
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

fn overlap_area(a: &WinRect, m: &MonitorRect) -> i64 {
    let ix = (a.x + a.w as i32).min(m.x + m.w as i32) - a.x.max(m.x);
    let iy = (a.y + a.h as i32).min(m.y + m.h as i32) - a.y.max(m.y);
    if ix <= 0 || iy <= 0 {
        0
    } else {
        ix as i64 * iy as i64
    }
}

fn best_monitor<'a>(rect: &WinRect, monitors: &'a [MonitorRect]) -> Option<&'a MonitorRect> {
    monitors.iter().max_by_key(|m| overlap_area(rect, m))
}

/// サイズを対象モニタ内に収め、位置もモニタ範囲内へ寄せる（異常値からの自己回復）。
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

/// 保存値があり、かついずれかのモニタと交差していればそれを採用。
/// 無効/画面外なら `default_monitor` を基準にした既定位置へフォールバック。
/// いずれの場合も対象モニタ内へクランプしてから適用する。
pub fn restore(
    window: &slint::Window,
    saved: Option<&WinRect>,
    monitors: &[MonitorRect],
    default_monitor: usize,
    default_size: (u32, u32),
) {
    let rect = match saved {
        Some(r) if intersects_any(r, monitors) => r.clone(),
        _ => default_rect(monitors, default_monitor, default_size),
    };
    let rect = match best_monitor(&rect, monitors) {
        Some(m) => clamp_to_monitor(&rect, m),
        None => rect,
    };
    window.set_size(PhysicalSize::new(rect.w, rect.h));
    window.set_position(PhysicalPosition::new(rect.x, rect.y));
}
