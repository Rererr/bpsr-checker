mod bridge;
mod capture;
mod engine;
mod error;
mod protocol;

use bridge::commands;
use engine::encounter::EncounterMutex;
use engine::name_cache;
use engine::selected_uid;
use log::{info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, Position, Size, Window, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_log::fern::colors::ColoredLevelConfig;
use tauri_plugin_window_state::{AppHandleExt, Builder as WindowStateBuilder, StateFlags};
use tauri_specta::{Builder, collect_commands};

pub const WINDOW_MAIN_LABEL: &str = "main";

pub static IS_EXITING: AtomicBool = AtomicBool::new(false);

pub fn begin_exit() {
    IS_EXITING.store(true, Ordering::Relaxed);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = Builder::<tauri::Wry>::new().commands(collect_commands![
        commands::get_header_info,
        commands::get_dps_players,
        commands::get_dps_boss_players,
        commands::get_heal_players,
        commands::get_dmg_taken_players,
        commands::get_dmg_taken_attackers,
        commands::get_dmg_taken_skills,
        commands::get_skills,
        commands::reset_encounter,
        commands::toggle_pause,
        commands::quit_app,
        commands::set_combat_exit_timeout,
        commands::set_history_limit,
        commands::get_history,
        commands::clear_history,
        commands::set_time_series_config,
        commands::get_time_series,
        commands::set_always_on_top,
        commands::set_click_through,
        commands::get_selected_uid,
        commands::set_selected_uid,
        commands::lookup_name_cache,
        commands::start_3min_measure_mode,
        commands::cancel_3min_measure_mode,
        commands::finalize_3min_measure_mode,
        commands::get_measure_mode_status,
        commands::get_tracked_buffs,
        commands::set_imagine_only_mode,
        commands::set_buffs_window_visible,
        commands::get_self_buff_status,
        commands::set_self_status_window_visible,
        commands::set_main_opacity,
    ]);

    #[cfg(debug_assertions)]
    {
        use specta_typescript::Typescript;
        use std::fs;
        builder
            .export(Typescript::default(), "../src/lib/bindings.ts")
            .expect("Failed to export typescript bindings");

        let bindings_path = "../src/lib/bindings.ts";
        if let Ok(content) = fs::read_to_string(bindings_path) {
            if !content.starts_with("// @ts-nocheck") {
                let updated = format!("// @ts-nocheck\n{content}");
                let _ = fs::write(bindings_path, updated);
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, _argv, _cwd| {}))
        .plugin(
            WindowStateBuilder::default()
                .with_state_flags(
                    StateFlags::POSITION
                        .union(StateFlags::SIZE)
                        .union(StateFlags::MAXIMIZED),
                )
                .build(),
        )
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(builder.invoke_handler())
        .setup(|app| {
            info!("starting bpsr-checker v{}", app.package_info().version);

            let app_handle = app.handle().clone();
            setup_logs(&app_handle)?;
            setup_tray(&app_handle)?;

            app.manage(EncounterMutex::default());

            if let Ok(dir) = app_handle.path().app_local_data_dir() {
                name_cache::init(dir.join("name_cache.json"));
                selected_uid::init(dir.join("selected_uid.json"));
            } else {
                warn!("Name cache: could not resolve app local data dir; cache disabled");
            }

            let app_handle_for_shortcut = app_handle.clone();
            app.global_shortcut()
                .on_shortcut("Ctrl+Shift+Z", move |_, _, event| {
                    if event.state == ShortcutState::Pressed {
                        if let Some(w) =
                            app_handle_for_shortcut.get_webview_window(WINDOW_MAIN_LABEL)
                        {
                            let _ = w.set_ignore_cursor_events(false);
                            let _ = w.emit("click-through-disabled", ());
                        }
                    }
                })?;

            let app_handle_for_minimize = app_handle.clone();
            app.global_shortcut()
                .on_shortcut("Ctrl+Shift+H", move |_, _, event| {
                    if event.state == ShortcutState::Pressed {
                        if let Some(w) =
                            app_handle_for_minimize.get_webview_window(WINDOW_MAIN_LABEL)
                        {
                            let _ = w.minimize();
                        }
                    }
                })?;

            // Start packet capture pipeline
            let handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = capture::start(handle).await {
                    log::error!("Capture pipeline error: {e}");
                }
            });

            // window-state プラグインの座標復元後に、main が画面外へはみ出していたら
            // プライマリモニタ内へ収め直す（マルチモニタ構成変更で「消えた/細くなった」対策）
            let app_handle_for_geom = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                if let Some(w) = app_handle_for_geom.get_webview_window(WINDOW_MAIN_LABEL) {
                    ensure_on_screen(&w);
                    // window-state が旧い最小高さ未満のサイズを復元することがある。
                    // 混在DPIではトグル後リロードの描画ズレが低い窓で出るため、
                    // 最小高さ(論理300)を下回っていたら既定値へ底上げする。
                    if let (Ok(sz), Ok(sf)) = (w.inner_size(), w.scale_factor()) {
                        let logical_h = f64::from(sz.height) / sf;
                        if logical_h < 300.0 {
                            let logical_w = f64::from(sz.width) / sf;
                            let _ = w.set_size(LogicalSize {
                                width: logical_w,
                                height: 350.0,
                            });
                            warn!("main 高さが最小未満({logical_h:.0}<300)のため 350 へ底上げ");
                        }
                    }
                }
                log_windows(&app_handle_for_geom, "startup");
            });

            Ok(())
        })
        .on_window_event(on_window_event)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                begin_exit();
                name_cache::flush();
                selected_uid::flush();
                #[cfg(target_os = "windows")]
                {
                    capture::windivert::request_shutdown();
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_millis(1500);
                    while !capture::windivert::is_handle_closed()
                        && std::time::Instant::now() < deadline
                    {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    if let Err(e) = capture::windivert::force_uninstall_service() {
                        warn!("WinDivert uninstall failed: {e}");
                    }
                }
            }
        });
}

fn setup_logs(app: &tauri::AppHandle) -> tauri::Result<()> {
    let version = &app.package_info().version;
    let timestamp = chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S");
    let log_name = format!("bpsr-checker-v{version}-{timestamp}");

    app.plugin(
        tauri_plugin_log::Builder::new()
            .clear_targets()
            .with_colors(ColoredLevelConfig::default())
            .targets([
                #[cfg(debug_assertions)]
                tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout)
                    .filter(|m| m.level() <= log::LevelFilter::Info),
                tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                    file_name: Some(log_name),
                })
                .filter(|m| m.level() <= log::LevelFilter::Info),
            ])
            .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
            .max_file_size(50_000_000)
            .build(),
    )?;
    Ok(())
}

fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    fn show_window(window: &tauri::WebviewWindow) -> tauri::Result<()> {
        ensure_on_screen(window);
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
        Ok(())
    }

    let menu = MenuBuilder::new(app)
        .text("toggle-settings", "設定")
        .text("show", "表示")
        .text("reset-window", "ウィンドウリセット")
        .separator()
        .text("quit", "終了")
        .build()?;

    let mut tray_builder = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray_builder = tray_builder.icon(icon.clone());
    }
    let _tray = tray_builder
        .on_menu_event(|tray_app, event| match event.id.as_ref() {
            "toggle-settings" => {
                let handle = tray_app.app_handle();
                if let Some(w) = handle.get_webview_window(WINDOW_MAIN_LABEL) {
                    let _ = w.emit("toggle-settings", ());
                }
            }
            "show" => {
                if let Some(w) = tray_app.app_handle().get_webview_window(WINDOW_MAIN_LABEL) {
                    let _ = show_window(&w);
                }
            }
            "reset-window" => {
                if let Some(w) = tray_app.get_webview_window(WINDOW_MAIN_LABEL) {
                    let _ = w.set_size(Size::Logical(LogicalSize {
                        width: 520.0,
                        height: 350.0,
                    }));
                    let _ =
                        w.set_position(Position::Logical(LogicalPosition { x: 100.0, y: 100.0 }));
                    let _ = show_window(&w);
                }
            }
            "quit" => {
                begin_exit();
                tray_app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(w) = tray.app_handle().get_webview_window(WINDOW_MAIN_LABEL) {
                    let _ = show_window(&w);
                }
            }
        })
        .build(app)?;
    Ok(())
}

/// ウインドウの過半が画面外に出ている場合、プライマリモニタ内へ収め直す。
///
/// マルチモニタ構成の変更（サブモニタの取り外し・配置変更）後に、保存済み座標の
/// ままだとウインドウが画面外の細い帯になり「消えた」ように見える問題への安全網。
/// 起動時の main と、オーバーレイ表示ON時に呼ぶ。
pub fn ensure_on_screen(window: &tauri::WebviewWindow) {
    use tauri::{PhysicalPosition, PhysicalSize};

    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        return;
    };
    let Ok(monitors) = window.available_monitors() else {
        return;
    };
    if monitors.is_empty() {
        return;
    }

    let win_area = i64::from(size.width) * i64::from(size.height);
    if win_area <= 0 {
        return;
    }
    let win_left = pos.x;
    let win_top = pos.y;
    let win_right = pos.x + size.width as i32;
    let win_bottom = pos.y + size.height as i32;

    // 全モニタと重なる可視面積を合算
    let mut visible: i64 = 0;
    for m in &monitors {
        let mp = m.position();
        let ms = m.size();
        let ix = (win_right.min(mp.x + ms.width as i32) - win_left.max(mp.x)).max(0);
        let iy = (win_bottom.min(mp.y + ms.height as i32) - win_top.max(mp.y)).max(0);
        visible += i64::from(ix) * i64::from(iy);
    }

    // 診断ログ: ウインドウ実座標・各モニタ配置・可視率。混在DPI/画面外切り分け用。
    let mon_dump: Vec<String> = monitors
        .iter()
        .map(|m| {
            let p = m.position();
            let s = m.size();
            format!(
                "[{},{} {}x{} sf={:.2}]",
                p.x,
                p.y,
                s.width,
                s.height,
                m.scale_factor()
            )
        })
        .collect();
    info!(
        "ensure_on_screen[{}]: pos=({},{}) size=({}x{}) 可視={}/{} ({}%) monitors={}",
        window.label(),
        win_left,
        win_top,
        size.width,
        size.height,
        visible,
        win_area,
        if win_area > 0 { visible * 100 / win_area } else { 0 },
        mon_dump.join(" "),
    );

    // 過半数が画面内に収まっていれば触らない
    if visible * 2 >= win_area {
        return;
    }

    let Some(mon) = window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.first().cloned())
    else {
        return;
    };
    let mp = mon.position();
    let ms = mon.size();
    let w = size.width.min(ms.width.saturating_sub(80)).max(1);
    let h = size.height.min(ms.height.saturating_sub(80)).max(1);
    let _ = window.set_size(PhysicalSize { width: w, height: h });
    let _ = window.set_position(PhysicalPosition {
        x: mp.x + 40,
        y: mp.y + 40,
    });
    warn!(
        "{} ウインドウが画面外でした (可視 {visible}/{win_area} px)。プライマリモニタ内へ移動しました",
        window.label()
    );
}

/// main の半透明(ゲーム透過)を OS のレイヤードウィンドウで実現する。
///
/// main を `transparent: true`(WS_EX_NOREDIRECTIONBITMAP/DirectComposition)に
/// すると、混在DPIマルチモニタで兄弟ウインドウの合成変化のたびに main の合成面が
/// 不可逆に壊れて不可視化する。そこで main は非透明にし、ウインドウ全体の
/// 一様アルファ(`WS_EX_LAYERED` + `SetLayeredWindowAttributes`)で透過を出す。
/// この方式は redirection bitmap 経由で堅牢。
#[cfg(target_os = "windows")]
pub fn set_window_alpha(window: &tauri::WebviewWindow, opacity: f64) {
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW,
        WS_EX_LAYERED,
    };

    let raw = match window.hwnd() {
        Ok(h) => h.0 as isize,
        Err(e) => {
            warn!("set_window_alpha: hwnd 取得失敗: {e}");
            return;
        }
    };
    let hwnd = HWND(raw);
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = ex | WS_EX_LAYERED.0 as isize;
        if ex != want {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
        }
        if !SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA).as_bool() {
            warn!("set_window_alpha: SetLayeredWindowAttributes 失敗");
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn set_window_alpha(_window: &tauri::WebviewWindow, _opacity: f64) {}

/// 全ウインドウの可視状態・座標・サイズをログ出力する診断用ヘルパ。
/// 「main が消える」系の調査で、各操作の前後に呼んで状態を残す。
pub fn log_windows(app: &tauri::AppHandle, ctx: &str) {
    for label in [WINDOW_MAIN_LABEL, "buffs", "self_status"] {
        match app.get_webview_window(label) {
            Some(w) => {
                let vis = w.is_visible().unwrap_or(false);
                let min = w.is_minimized().unwrap_or(false);
                let pos = w.outer_position().ok();
                let size = w.outer_size().ok();
                info!("[win:{ctx}] {label}: visible={vis} minimized={min} pos={pos:?} size={size:?}");
            }
            None => info!("[win:{ctx}] {label}: <not found>"),
        }
    }
}

fn on_window_event(window: &Window, event: &WindowEvent) {
    match event {
        WindowEvent::Resized(_) => {
            if let Ok(minimized) = window.is_minimized() {
                let _ = window.set_skip_taskbar(!minimized);
            }
        }
        WindowEvent::Focused(false) => {
            let _ = window.app_handle().save_window_state(
                StateFlags::POSITION
                    .union(StateFlags::SIZE)
                    .union(StateFlags::MAXIMIZED),
            );
        }
        _ => {}
    }
}
