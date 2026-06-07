// Blue Protocol: Star Resonance DPSチェッカー（Slint版・移行中）
// S1: core→Slint のライブ配線（capture スレッド→共有 EncounterMutex→UIポーリング）。
// リリースではコンソールを出さない（CJK の ICU 行分割警告は dev 時のみ・実害なし）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod capture;
mod format;
mod overlay;
mod window_state;

use bpsr_core::compute;
use bpsr_core::engine;
use bpsr_core::engine::encounter::EncounterMutex;
use slint::{ComponentHandle, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
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

// 名前列テンプレート（既定）。将来は設定から差し替える。
const NAME_TEMPLATE: &str = "{name} {spec}({score} - {seasonLv} - {seasonStr})";

/// タブ(0=dps 1=heal 2=taken 3=history)に応じてプレイヤー一覧を取得。
/// history(3) は S5 実装まで dps を表示する暫定。
fn fetch_players(enc: &EncounterMutex, tab: i32) -> bpsr_core::models::PlayersWindow {
    match tab {
        1 => compute::get_heal_players(enc),
        2 => compute::get_dmg_taken_players(enc),
        _ => compute::get_dps_players(enc),
    }
}

fn build_rows(pw: &bpsr_core::models::PlayersWindow) -> Vec<Row> {
    let top = pw.top_value.max(1.0);
    let local = pw.local_player_uid;
    pw.player_rows
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let rank = (i + 1) as i32;
            Row {
                rank,
                uid_str: format!("{}", p.uid as i64).into(),
                name: format::format_row_name(
                    &p.name,
                    &p.class_name,
                    &p.class_spec_name,
                    p.ability_score,
                    p.season_level,
                    p.season_strength,
                    rank,
                    NAME_TEMPLATE,
                    false,
                )
                .into(),
                class_color: format::class_color(&p.class_name),
                dmg_text: format::format_number(p.total_value).into(),
                dps_text: format::format_dps(p.value_per_sec).into(),
                pct_text: format::format_pct(p.value_pct).into(),
                pct: ((p.total_value / top) * 100.0) as f32,
                is_local: p.uid == local,
            }
        })
        .collect()
}

fn build_skill_rows(sw: &bpsr_core::models::SkillsWindow) -> Vec<SkillRowUi> {
    let top = sw.top_value.max(1.0);
    sw.skill_rows
        .iter()
        .map(|s| {
            let (en, ec) = format::element_label(s.element);
            SkillRowUi {
                name: s.name.clone().into(),
                elem_text: en.into(),
                elem_color: ec,
                total_text: format::format_number(s.total_value).into(),
                dps_text: format::format_dps(s.value_per_sec).into(),
                pct_text: format::format_pct(s.value_pct).into(),
                pct: ((s.total_value / top) * 100.0) as f32,
            }
        })
        .collect()
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

    // 現在タブ（UIスレッド共有）。タブクリックと周期ポーリングの両方が参照する。
    let tab_cell = Rc::new(Cell::new(0i32));

    // スキル内訳ビュー用モデル＋対象プレイヤー uid（0=なし）
    let skill_rows = Rc::new(VecModel::<SkillRowUi>::default());
    main.set_skill_rows(skill_rows.clone().into());
    let inspected_uid = Rc::new(Cell::new(0i64));

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
    {
        let w = main.as_weak();
        main.on_start_resize(move |dir| {
            if let Some(m) = w.upgrade() {
                overlay::start_resize(m.window(), dir);
            }
        });
    }
    // タブ選択: 共有セルを更新し、即時に再取得して反映（ポーリング待ちにしない）
    {
        let w = main.as_weak();
        let enc_sel = enc.clone();
        let rows_sel = rows.clone();
        let tab_sel = tab_cell.clone();
        main.on_select_tab(move |n| {
            tab_sel.set(n);
            if let Some(m) = w.upgrade() {
                m.set_tab(n);
                rows_sel.set_vec(build_rows(&fetch_players(&enc_sel, n)));
            }
        });
    }
    // 行クリック → スキル内訳へ
    {
        let w = main.as_weak();
        let enc_sk = enc.clone();
        let sk_rows = skill_rows.clone();
        let insp = inspected_uid.clone();
        main.on_open_skills(move |uid_str| {
            let uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            if uid == 0 {
                return;
            }
            let Some(m) = w.upgrade() else {
                return;
            };
            match compute::get_skills(&enc_sk, uid) {
                Ok(sw) => {
                    insp.set(uid);
                    m.set_inspected_name(sw.inspected_player.name.clone().into());
                    sk_rows.set_vec(build_skill_rows(&sw));
                    m.set_view(1);
                }
                Err(e) => log::warn!("get_skills({uid}) failed: {e}"),
            }
        });
    }
    // 戻る → 一覧へ
    {
        let w = main.as_weak();
        let insp = inspected_uid.clone();
        main.on_back(move || {
            insp.set(0);
            if let Some(m) = w.upgrade() {
                m.set_view(0);
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
    let tab_cell_poll = tab_cell.clone();
    let inspected_uid_poll = inspected_uid.clone();
    let skill_rows_poll = skill_rows.clone();

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

        // ライブ集計を反映（共有セルの現在タブに応じて取得）
        let header = compute::get_header_info(&enc_poll);
        m.set_total_text(format::format_dps(header.total_dps).into());
        m.set_elapsed_text(format::format_elapsed(header.elapsed_ms).into());

        let pw = fetch_players(&enc_poll, tab_cell_poll.get());
        rows.set_vec(build_rows(&pw));

        // スキル内訳ビュー中はライブ更新
        if m.get_view() == 1 {
            let uid = inspected_uid_poll.get();
            if uid != 0 {
                if let Ok(sw) = compute::get_skills(&enc_poll, uid) {
                    skill_rows_poll.set_vec(build_skill_rows(&sw));
                }
            }
        }

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
