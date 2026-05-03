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
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
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
        .plugin(tauri_plugin_window_state::Builder::default().build())
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
            app.global_shortcut().on_shortcut("Ctrl+Shift+Z", move |_, _, event| {
                if event.state == ShortcutState::Pressed {
                    if let Some(w) = app_handle_for_shortcut.get_webview_window(WINDOW_MAIN_LABEL) {
                        let _ = w.set_ignore_cursor_events(false);
                        let _ = w.emit("click-through-disabled", ());
                    }
                }
            })?;

            let app_handle_for_minimize = app_handle.clone();
            app.global_shortcut().on_shortcut("Ctrl+Shift+H", move |_, _, event| {
                if event.state == ShortcutState::Pressed {
                    if let Some(w) = app_handle_for_minimize.get_webview_window(WINDOW_MAIN_LABEL) {
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
                    info!("App closing, releasing WinDivert handle...");
                    // Step 1: abort the blocking recv so the capture task
                    // can return and drop its WinDivert handle.
                    capture::windivert::request_shutdown();
                    // Step 2: give the capture task a moment to actually
                    // drop the handle. Without this, uninstall() races the
                    // task and ControlService(STOP) fails because the
                    // handle is still open, which leaves WinDivert64.sys
                    // locked and prevents version updates from overwriting
                    // it.
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    if let Err(e) = windivert::WinDivert::uninstall() {
                        warn!("WinDivert uninstall failed (best-effort): {e}");
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

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .icon(app.default_window_icon().unwrap().clone())
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
                    let _ = w.set_position(Position::Logical(LogicalPosition {
                        x: 100.0,
                        y: 100.0,
                    }));
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

fn on_window_event(window: &Window, event: &WindowEvent) {
    match event {
        WindowEvent::CloseRequested { api, .. } => {
            if IS_EXITING.load(Ordering::Relaxed) {
                // Quit was requested explicitly — let the close proceed so the
                // process actually terminates and file handles are released.
                return;
            }
            api.prevent_close();
            let _ = window.hide();
        }
        WindowEvent::Focused(false) => {
            let _ = window.app_handle().save_window_state(StateFlags::all());
        }
        _ => {}
    }
}
