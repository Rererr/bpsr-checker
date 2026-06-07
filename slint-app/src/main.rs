// Blue Protocol: Star Resonance DPSチェッカー（Slint版・移行中）
// S1: core→Slint のライブ配線（capture スレッド→共有 EncounterMutex→UIポーリング）。
// リリースではコンソールを出さない（CJK の ICU 行分割警告は dev 時のみ・実害なし）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod capture;
mod format;
mod overlay;
mod settings;
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

/// タブ(0=dps 1=heal 2=taken 3=history)に応じてプレイヤー一覧を取得。
/// history(3) は S5 実装まで dps を表示する暫定。
fn fetch_players(enc: &EncounterMutex, tab: i32) -> bpsr_core::models::PlayersWindow {
    match tab {
        1 => compute::get_heal_players(enc),
        2 => compute::get_dmg_taken_players(enc),
        _ => compute::get_dps_players(enc),
    }
}

fn build_rows(
    pw: &bpsr_core::models::PlayersWindow,
    template: &str,
    abbreviate: bool,
    privacy: bool,
) -> Vec<Row> {
    let top = pw.top_value.max(1.0);
    let local = pw.local_player_uid;
    pw.player_rows
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let rank = (i + 1) as i32;
            let display = if privacy {
                format::mask_player_name(p.uid as i64)
            } else {
                p.name.clone()
            };
            Row {
                rank,
                uid_str: format!("{}", p.uid as i64).into(),
                name: format::format_row_name(
                    &display,
                    &p.class_name,
                    &p.class_spec_name,
                    p.ability_score,
                    p.season_level,
                    p.season_strength,
                    rank,
                    template,
                    abbreviate,
                )
                .into(),
                class_color: format::class_color(&p.class_name),
                dmg_text: format::format_number(p.total_value).into(),
                dps_text: format::format_dps(p.value_per_sec).into(),
                pct_text: format::format_pct(p.value_pct).into(),
                pct: ((p.total_value / top) * 100.0) as f32,
                is_local: p.uid == local,
                crit_text: format::format_pct(p.crit_rate).into(),
                crit_value_text: format::format_pct(p.crit_value_rate).into(),
                lucky_text: format::format_pct(p.lucky_rate).into(),
                lucky_value_text: format::format_pct(p.lucky_value_rate).into(),
                hits_text: format!("{}", p.hits as i64).into(),
                hpm_text: format!("{:.1}", p.hits_per_minute).into(),
                score_text: if p.ability_score > 0.0 {
                    format::format_score(p.ability_score, abbreviate)
                } else {
                    "-".to_string()
                }
                .into(),
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
                uid_str: format!("{}", s.uid as i64).into(),
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

/// ドリルダウン状態。
#[derive(Clone, Copy)]
enum Drill {
    None,
    Skills(i64),            // dps/heal: そのプレイヤーの技別
    TakenAttackers(i64),    // 被ダメ: 被害者の攻撃元一覧
    TakenSkills(i64, i64),  // 被ダメ: (被害者, 攻撃元) の技別
}

/// SkillsWindow を内訳ビューへ反映する共通処理。
fn show_drill(
    m: &MainWindow,
    sk_rows: &slint::VecModel<SkillRowUi>,
    sw: &bpsr_core::models::SkillsWindow,
    clickable: bool,
) {
    m.set_inspected_name(sw.inspected_player.name.clone().into());
    sk_rows.set_vec(build_skill_rows(sw));
    m.set_skills_clickable(clickable);
    m.set_view(1);
}

/// 設定の表示系を UI へ反映（列フラグ・自分強調・最前面・パネルのトグル状態）。
fn apply_settings(m: &MainWindow, c: &settings::Settings) {
    m.set_cols(ColumnFlags {
        crit: c.show_crit,
        crit_value: c.show_crit_value,
        lucky: c.show_lucky,
        lucky_value: c.show_lucky_value,
        hits: c.show_hits,
        hpm: c.show_hpm,
        score: c.show_score,
    });
    m.set_highlight_local(c.highlight_local_player);
    m.set_aot(c.always_on_top);
    m.set_win_opacity(c.opacity as f32);
    m.set_cfg_ui(SettingsUi {
        show_crit: c.show_crit,
        show_crit_value: c.show_crit_value,
        show_lucky: c.show_lucky,
        show_lucky_value: c.show_lucky_value,
        show_hits: c.show_hits,
        show_hpm: c.show_hpm,
        show_score: c.show_score,
        highlight_local: c.highlight_local_player,
        abbreviate_scores: c.abbreviate_scores,
        privacy_mask: c.privacy_mask_names,
        aot: c.always_on_top,
    });
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

    // 設定（%APPDATA%\bpsr-checker\settings.json）。UIスレッドで共有・編集する。
    let cfg = Rc::new(RefCell::new(settings::load()));

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
    let drill = Rc::new(Cell::new(Drill::None));

    // 設定を起動時に適用（列フラグ・自分強調・最前面・起動タブ・runtime settings）
    {
        let c = cfg.borrow();
        apply_settings(&main, &c);
        let init_tab = match c.startup_tab.as_str() {
            "heal" => 1,
            "taken" => 2,
            "history" => 3,
            _ => 0,
        };
        main.set_tab(init_tab);
        tab_cell.set(init_tab);
        compute::set_combat_exit_timeout(c.combat_exit_sec);
        compute::set_history_limit(c.history_limit);
        compute::set_time_series_config(c.time_series_samples, c.time_series_interval_ms);
        compute::set_imagine_only_mode(&enc, c.imagine_only_mode);
    }

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
        let cfg_sel = cfg.clone();
        main.on_select_tab(move |n| {
            tab_sel.set(n);
            if let Some(m) = w.upgrade() {
                m.set_tab(n);
                let c = cfg_sel.borrow();
                rows_sel.set_vec(build_rows(
                    &fetch_players(&enc_sel, n),
                    &c.name_template,
                    c.abbreviate_scores,
                    c.privacy_mask_names,
                ));
            }
        });
    }
    // 行クリック → ドリルダウン（dps/heal: 技別 / 被ダメ: 攻撃元一覧）
    {
        let w = main.as_weak();
        let enc_sk = enc.clone();
        let sk_rows = skill_rows.clone();
        let drill_h = drill.clone();
        let tab_h = tab_cell.clone();
        main.on_open_drill(move |uid_str| {
            let uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            if uid == 0 {
                return;
            }
            let Some(m) = w.upgrade() else {
                return;
            };
            if tab_h.get() == 2 {
                match compute::get_dmg_taken_attackers(&enc_sk, uid) {
                    Ok(sw) => {
                        drill_h.set(Drill::TakenAttackers(uid));
                        show_drill(&m, &sk_rows, &sw, true);
                    }
                    Err(e) => log::warn!("get_dmg_taken_attackers({uid}) failed: {e}"),
                }
            } else {
                match compute::get_skills(&enc_sk, uid) {
                    Ok(sw) => {
                        drill_h.set(Drill::Skills(uid));
                        show_drill(&m, &sk_rows, &sw, false);
                    }
                    Err(e) => log::warn!("get_skills({uid}) failed: {e}"),
                }
            }
        });
    }
    // 攻撃元クリック（被ダメ）→ その攻撃元の技別へ
    {
        let w = main.as_weak();
        let enc_sk = enc.clone();
        let sk_rows = skill_rows.clone();
        let drill_h = drill.clone();
        main.on_drill_row(move |uid_str| {
            let attacker: i64 = uid_str.as_str().parse().unwrap_or(0);
            if attacker == 0 {
                return;
            }
            let Some(m) = w.upgrade() else {
                return;
            };
            if let Drill::TakenAttackers(player) = drill_h.get() {
                match compute::get_dmg_taken_skills(&enc_sk, player, attacker) {
                    Ok(sw) => {
                        drill_h.set(Drill::TakenSkills(player, attacker));
                        show_drill(&m, &sk_rows, &sw, false);
                    }
                    Err(e) => log::warn!("get_dmg_taken_skills failed: {e}"),
                }
            }
        });
    }
    // 戻る（被ダメ技別→攻撃元一覧、それ以外→一覧へ）
    {
        let w = main.as_weak();
        let enc_b = enc.clone();
        let sk_rows = skill_rows.clone();
        let drill_h = drill.clone();
        main.on_back(move || {
            let Some(m) = w.upgrade() else {
                return;
            };
            if let Drill::TakenSkills(player, _) = drill_h.get() {
                if let Ok(sw) = compute::get_dmg_taken_attackers(&enc_b, player) {
                    drill_h.set(Drill::TakenAttackers(player));
                    show_drill(&m, &sk_rows, &sw, true);
                    return;
                }
            }
            drill_h.set(Drill::None);
            m.set_view(0);
        });
    }
    // 設定パネルの開閉
    {
        let w = main.as_weak();
        main.on_toggle_settings(move || {
            if let Some(m) = w.upgrade() {
                m.set_settings_open(!m.get_settings_open());
            }
        });
    }
    // 設定トグル変更 → cfg 更新・即適用・保存
    {
        let w = main.as_weak();
        let cfg_b = cfg.clone();
        main.on_set_bool(move |key, val| {
            {
                let mut c = cfg_b.borrow_mut();
                match key.as_str() {
                    "show-crit" => c.show_crit = val,
                    "show-crit-value" => c.show_crit_value = val,
                    "show-lucky" => c.show_lucky = val,
                    "show-lucky-value" => c.show_lucky_value = val,
                    "show-hits" => c.show_hits = val,
                    "show-hpm" => c.show_hpm = val,
                    "show-score" => c.show_score = val,
                    "highlight-local" => c.highlight_local_player = val,
                    "abbreviate-scores" => c.abbreviate_scores = val,
                    "privacy-mask" => c.privacy_mask_names = val,
                    "aot" => c.always_on_top = val,
                    other => log::warn!("unknown setting key: {other}"),
                }
            }
            let c = cfg_b.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
            }
            settings::save(&c);
        });
    }
    // 不透明度スライダー
    {
        let w = main.as_weak();
        let cfg_o = cfg.clone();
        main.on_set_opacity(move |v| {
            let clamped = v.clamp(0.05, 1.0) as f64;
            cfg_o.borrow_mut().opacity = clamped;
            if let Some(m) = w.upgrade() {
                m.set_win_opacity(clamped as f32);
            }
            settings::save(&cfg_o.borrow());
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
    let drill_poll = drill.clone();
    let skill_rows_poll = skill_rows.clone();
    let cfg_poll = cfg.clone();
    let poll_ms = cfg.borrow().poll_interval_ms.max(50.0) as u64;

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(poll_ms), move || {
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
        {
            let c = cfg_poll.borrow();
            rows.set_vec(build_rows(
                &pw,
                &c.name_template,
                c.abbreviate_scores,
                c.privacy_mask_names,
            ));
        }

        // ドリルダウン中はライブ更新
        match drill_poll.get() {
            Drill::Skills(uid) => {
                if let Ok(sw) = compute::get_skills(&enc_poll, uid) {
                    skill_rows_poll.set_vec(build_skill_rows(&sw));
                }
            }
            Drill::TakenAttackers(uid) => {
                if let Ok(sw) = compute::get_dmg_taken_attackers(&enc_poll, uid) {
                    skill_rows_poll.set_vec(build_skill_rows(&sw));
                }
            }
            Drill::TakenSkills(p, a) => {
                if let Ok(sw) = compute::get_dmg_taken_skills(&enc_poll, p, a) {
                    skill_rows_poll.set_vec(build_skill_rows(&sw));
                }
            }
            Drill::None => {}
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
