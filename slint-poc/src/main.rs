// Slint オーバーレイ検証 PoC（使い捨て）
// 検証対象: 透明 / 最前面 / クリックスルー / 複数モニタ配置 / 位置保存・復元
// データは mock。engine/capture には依存しない。

slint::include_modules!();

mod mock;
mod overlay;
mod window_state;

use slint::{ComponentHandle, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

// 既定配置に使うモニタ index
const PRIMARY: usize = 0;
const SECONDARY: usize = 1;

/// winit/glutin 等の `log` 経由メッセージを WARN 以上だけ簡潔に表示する。
/// 念のため "segmentation model" を含む行は握り潰す（ただし Slint の CJK 警告は
/// `log` ではなく直接 stderr に出るため、これでは止まらない。PoC では mock の
/// ラベルを ASCII にして発生自体を避けている。mock.rs 参照）。
struct QuietLog;
impl log::Log for QuietLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        let msg = record.args().to_string();
        if msg.contains("segmentation model") {
            return;
        }
        eprintln!("[{}] {}", record.level(), msg);
    }
    fn flush(&self) {}
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = log::set_logger(&QuietLog);
    log::set_max_level(log::LevelFilter::Warn);

    // ── winit backend を生成時属性フック付きで初期化 ──
    // 全ウィンドウを「透明合成可能 + タスクバー非表示」にする土台。
    // 実際に透けるかは各 .slint の background で決まる。
    let backend = i_slint_backend_winit::Backend::builder()
        .with_window_attributes_hook(|attrs| {
            let attrs = attrs.with_transparent(true);
            #[cfg(target_os = "windows")]
            let attrs = {
                use i_slint_backend_winit::winit::platform::windows::WindowAttributesExtWindows;
                attrs.with_skip_taskbar(true)
            };
            attrs
        })
        .build()?;
    slint::platform::set_platform(Box::new(backend))
        .map_err(|e| format!("set_platform failed: {e:?}"))?;

    // ── ウィンドウ生成 ──
    let dps = DpsList::new()?;
    let buffs = BuffOverlay::new()?;

    // ── モデル接続 ──
    let rows = Rc::new(VecModel::<PlayerRow>::default());
    dps.set_rows(rows.clone().into());
    let monitors_model = Rc::new(VecModel::<MonitorInfo>::default());
    dps.set_monitors(monitors_model.clone().into());

    let bars = Rc::new(VecModel::<BuffTimer>::default());
    buffs.set_bars(bars.clone().into());
    let circles = Rc::new(VecModel::<BuffTimer>::default());
    buffs.set_circles(circles.clone().into());

    // 終了
    dps.on_quit(|| {
        let _ = slint::quit_event_loop();
    });

    // オーバーレイ表示/非表示トグル。
    // あえてネイティブの hide()/show()（ShowWindow 相当）を使い、兄弟窓の
    // 表示切替で DPS 窓が白紙化しないか（Tauri/WebView2 で起きる現象）を検証する。
    {
        let buffs_w = buffs.as_weak();
        let dps_w = dps.as_weak();
        dps.on_toggle_buffs(move || {
            let (Some(bf), Some(d)) = (buffs_w.upgrade(), dps_w.upgrade()) else {
                return;
            };
            if d.get_buffs_visible() {
                let _ = bf.window().hide();
                d.set_buffs_visible(false);
                eprintln!("[poc] buffs overlay: hide()");
            } else {
                let _ = bf.window().show();
                overlay::set_click_through(bf.window(), d.get_buffs_locked()); // 現在のロック状態で再適用
                d.set_buffs_visible(true);
                eprintln!("[poc] buffs overlay: show()");
            }
        });
    }

    // ロック切替: locked=クリックスルー(ゲーム操作優先) / unlocked=当たり判定ありで移動可
    {
        let buffs_w = buffs.as_weak();
        let dps_w = dps.as_weak();
        dps.on_toggle_lock(move || {
            let (Some(bf), Some(d)) = (buffs_w.upgrade(), dps_w.upgrade()) else {
                return;
            };
            let new_locked = !d.get_buffs_locked();
            d.set_buffs_locked(new_locked);
            bf.set_locked(new_locked);
            overlay::set_click_through(bf.window(), new_locked);
            eprintln!(
                "[poc] buffs overlay: {}",
                if new_locked {
                    "locked (click-through)"
                } else {
                    "unlocked (draggable)"
                }
            );
        });
    }

    // no-frame 窓のドラッグ移動（winit drag_window をマウス押下時に発火）
    {
        let dps_w = dps.as_weak();
        dps.on_start_drag(move || {
            if let Some(d) = dps_w.upgrade() {
                overlay::start_drag(d.window());
            }
        });
    }
    {
        let buffs_w = buffs.as_weak();
        buffs.on_start_drag(move || {
            if let Some(bf) = buffs_w.upgrade() {
                overlay::start_drag(bf.window());
            }
        });
    }

    // ── 表示 ──
    // 注意: show() 時点では winit ウィンドウはまだ実体化していないことがある。
    // モニタ列挙・クリックスルー・位置復元など「ネイティブ窓が必要な処理」は
    // イベントループ開始後の初回 Timer tick（下の setup）で行う。
    dps.show()?;
    buffs.show()?;

    // ── 周期更新（mock）＋初回セットアップ＋レイアウト自動保存 ──
    let dps_w = dps.as_weak();
    let buffs_w = buffs.as_weak();
    let saved = window_state::load();
    let last_saved = Rc::new(RefCell::new(saved.clone()));
    let mut tick: u64 = 0;
    let mut setup_done = false;
    let mut setup_tick: u64 = 0;
    // 復元(set_position/set_size)は次の周回で反映されるため、確定するまで保存を待つ
    const SETTLE_TICKS: u64 = 5;

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(200), move || {
        tick += 1;

        rows.set_vec(mock::players(tick));
        let (b, c) = mock::buffs(tick);
        bars.set_vec(b);
        circles.set_vec(c);

        let (Some(d), Some(bf)) = (dps_w.upgrade(), buffs_w.upgrade()) else {
            return;
        };

        // 初回: winit ウィンドウが実体化していればネイティブ設定を適用
        if !setup_done {
            let mons = overlay::monitors(d.window());
            if !mons.is_empty() {
                eprintln!("[poc] detected {} monitor(s):", mons.len());
                for m in &mons {
                    eprintln!(
                        "  - {:<14} pos=({:>5},{:>5}) size={:>5}x{:<5} scale={:.2} {}",
                        m.name,
                        m.x,
                        m.y,
                        m.w,
                        m.h,
                        m.scale,
                        if m.primary { "[primary]" } else { "" }
                    );
                }
                monitors_model.set_vec(
                    mons.iter()
                        .map(|m| MonitorInfo {
                            name: m.name.clone().into(),
                            geometry: format!("{}x{} @({},{})", m.w, m.h, m.x, m.y).into(),
                            scale: m.scale as f32,
                            primary: m.primary,
                        })
                        .collect::<Vec<_>>(),
                );

                // バフオーバーレイはクリックスルー（ゲーム操作を妨げない）
                overlay::set_click_through(bf.window(), true);

                // 位置復元（無ければ DPS=プライマリ / バフ=セカンダリ）
                window_state::restore(d.window(), saved.dps.as_ref(), &mons, PRIMARY, (540, 660));
                window_state::restore(
                    bf.window(),
                    saved.buffs.as_ref(),
                    &mons,
                    SECONDARY,
                    (320, 150),
                );
                setup_done = true;
                setup_tick = tick;
            }
        }

        d.set_status(format!("tick={tick}  setup={setup_done}").into());

        // 復元適用が確定するまで（数tick）は保存しない＝初期transient位置を保存しない
        if !setup_done || tick < setup_tick + SETTLE_TICKS {
            return;
        }

        // 位置/サイズが変わったら保存（差分があるときだけ書き込み）。
        // 非表示中の buffs は位置が当てにならないので最後の値を保持する。
        let dps_rect = window_state::capture(d.window());
        let buffs_rect = if d.get_buffs_visible() {
            Some(window_state::capture(bf.window()))
        } else {
            last_saved.borrow().buffs.clone()
        };
        let cur = window_state::Layout {
            dps: Some(dps_rect),
            buffs: buffs_rect,
        };
        let mut ls = last_saved.borrow_mut();
        if *ls != cur {
            window_state::save(&cur);
            *ls = cur;
        }
    });

    slint::run_event_loop()?;
    Ok(())
}
