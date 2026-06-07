// Blue Protocol: Star Resonance DPSチェッカー（Slint版・移行中）
// S1: core→Slint のライブ配線（capture スレッド→共有 EncounterMutex→UIポーリング）。
// リリースではコンソールを出さない（CJK の ICU 行分割警告は dev 時のみ・実害なし）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod capture;
mod overlay;
mod window_state;

use bpsr_core::compute;
use bpsr_core::engine;
use bpsr_core::engine::encounter::EncounterMutex;
use slint::{ComponentHandle, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

const SETTLE_TICKS: u64 = 5;

/// 最小ロガー。core の capture / 本体の診断ログを stderr へ出す。
/// （Slint/parley の CJK 警告は log ではなく直接 eprintln のため別物・ここでは触れない）
struct ConsoleLog;
impl log::Log for ConsoleLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        eprintln!("[{}] {}: {}", r.level(), r.target(), r.args());
    }
    fn flush(&self) {}
}

/// 二重起動防止（Windows 名前付き Mutex）。既に起動済みなら true。
#[cfg(windows)]
fn already_running() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::PCWSTR;
    let name: Vec<u16> = "Global\\bpsr-checker-slint-instance\0"
        .encode_utf16()
        .collect();
    unsafe {
        match CreateMutexW(None, true, PCWSTR(name.as_ptr())) {
            // ハンドルは閉じない＝プロセス寿命まで mutex を保持する。
            Ok(_handle) => GetLastError() == ERROR_ALREADY_EXISTS,
            Err(_) => false,
        }
    }
}

#[cfg(not(windows))]
fn already_running() -> bool {
    false
}

fn data_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(base).join("bpsr-checker")
}

fn fmt_value(v: f64) -> SharedString {
    if v >= 1_000_000.0 {
        format!("{:.2}M", v / 1e6).into()
    } else if v >= 1_000.0 {
        format!("{:.1}k", v / 1e3).into()
    } else {
        format!("{:.0}", v).into()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    static LOGGER: ConsoleLog = ConsoleLog;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    if already_running() {
        log::warn!("another instance is already running; exiting");
        return Ok(());
    }

    // winit backend（透明合成可能＋タスクバー非表示）
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
    slint::platform::set_platform(Box::new(backend)).map_err(|e| format!("set_platform: {e:?}"))?;

    // 永続キャッシュ初期化
    let dir = data_dir();
    engine::name_cache::init(dir.join("name_cache.json"));
    engine::selected_uid::init(dir.join("selected_uid.json"));

    // 共有エンカウンター＋パケット観測スレッド
    let enc = Arc::new(EncounterMutex::default());
    if let Some(uid) = engine::selected_uid::get() {
        if let Ok(mut e) = enc.lock() {
            e.local_player_uid = uid;
        }
    }
    capture::spawn(enc.clone());

    // メイン窓
    let main = MainWindow::new()?;
    // 言語選択（既定は日本語。将来は設定から）。最初のコンポーネント生成後に呼ぶ。
    let _ = slint::select_bundled_translation("ja");
    let rows = Rc::new(VecModel::<Row>::default());
    main.set_rows(rows.clone().into());

    main.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    {
        let w = main.as_weak();
        main.on_start_drag(move || {
            if let Some(m) = w.upgrade() {
                overlay::start_drag(m.window());
            }
        });
    }

    main.show()?;

    // 周期ポーリング＋初回セットアップ（位置復元）＋自動保存
    let main_w = main.as_weak();
    let enc_poll = enc.clone();
    let saved = window_state::load();
    let last_saved = Rc::new(RefCell::new(saved.clone()));
    let mut tick: u64 = 0;
    let mut setup_tick: u64 = 0;
    let mut setup_done = false;

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(300), move || {
        tick += 1;
        let Some(m) = main_w.upgrade() else {
            return;
        };

        // 初回: winit 実体化後に位置復元
        if !setup_done {
            let mons = overlay::monitors(m.window());
            if !mons.is_empty() {
                window_state::restore(m.window(), saved.main.as_ref(), &mons, 0, (520, 350));
                setup_done = true;
                setup_tick = tick;
                log::info!("window restored on {} monitor(s)", mons.len());
            }
        }

        // ライブ集計を反映
        let header = compute::get_header_info(&enc_poll);
        m.set_total_dps_text(fmt_value(header.total_dps));
        m.set_elapsed_text(format!("{:.0}s", header.elapsed_ms / 1000.0).into());

        let pw = compute::get_dps_players(&enc_poll);
        let top = pw.top_value.max(1.0);
        let new_rows: Vec<Row> = pw
            .player_rows
            .iter()
            .map(|p| Row {
                name: p.name.clone().into(),
                class_name: p.class_name.clone().into(),
                dps_text: fmt_value(p.value_per_sec),
                total_text: fmt_value(p.total_value),
                pct: ((p.total_value / top) * 100.0) as f32,
            })
            .collect();
        rows.set_vec(new_rows);

        // レイアウト自動保存（復元確定後・差分時のみ）
        if !setup_done || tick < setup_tick + SETTLE_TICKS {
            return;
        }
        let cur = window_state::Layout {
            main: Some(window_state::capture(m.window())),
            ..last_saved.borrow().clone()
        };
        let mut ls = last_saved.borrow_mut();
        if *ls != cur {
            window_state::save(&cur);
            *ls = cur;
        }
    });

    slint::run_event_loop()?;

    engine::name_cache::flush();
    engine::selected_uid::flush();
    Ok(())
}
