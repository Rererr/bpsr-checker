mod bridge;
mod capture;
mod engine;
mod error;
mod protocol;

use bridge::commands;
use engine::encounter::EncounterMutex;
use log::{info, warn};
use std::fs;
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, Position, Size, Window, WindowEvent};
use tauri_plugin_log::fern::colors::ColoredLevelConfig;
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
use tauri_specta::{Builder, collect_commands};

pub const WINDOW_MAIN_LABEL: &str = "main";

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
    ]);

    #[cfg(debug_assertions)]
    {
        use specta_typescript::Typescript;
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
        .invoke_handler(builder.invoke_handler())
        .setup(|app| {
            info!("starting bpsr-checker v{}", app.package_info().version);

            let app_handle = app.handle().clone();
            setup_logs(&app_handle)?;
            setup_tray(&app_handle)?;

            app.manage(EncounterMutex::default());

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
                #[cfg(target_os = "windows")]
                {
                    info!("App closing, cleaning up WinDivert...");
                    // WinDivert cleanup would go here
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
            "quit" => tray_app.exit(0),
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
            api.prevent_close();
            let _ = window.hide();
        }
        WindowEvent::Focused(false) => {
            let _ = window.app_handle().save_window_state(StateFlags::all());
        }
        _ => {}
    }
}
