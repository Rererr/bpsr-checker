// Blue Protocol: Star Resonance DPSチェッカー（Slint版・移行中）
// S1: core→Slint のライブ配線（capture スレッド→共有 EncounterMutex→UIポーリング）。
// リリースではコンソールを出さない（CJK の ICU 行分割警告は dev 時のみ・実害なし）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod buff_names;
mod capture;
mod format;
mod overlay;
mod settings;
mod watchlist;
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
    watched: &[i64],
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
                watched: watched.contains(&(p.uid as i64)),
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
        self_status: c.show_self_status_overlay,
        buff_overlay: c.show_buff_overlay,
        aot: c.always_on_top,
        three_min_auto_open: c.three_min_auto_open,
    });
    let int_str = |v: f64| -> slint::SharedString { format!("{}", v as i64).into() };
    m.set_nums(SettingsNumUi {
        combat_exit: int_str(c.combat_exit_sec),
        poll_interval: int_str(c.poll_interval_ms),
        history_limit: int_str(c.history_limit),
        ts_samples: int_str(c.time_series_samples),
        ts_interval: int_str(c.time_series_interval_ms),
        three_min_dur: int_str(c.three_min_duration_sec),
        graph_count: int_str(c.graph_player_count),
        font_size: int_str(c.font_size),
    });
}

/// SelfStatusEntry 群を UI 行へ変換（BuffIconCell 相当）。
fn build_status_entries(entries: &[bpsr_core::models::SelfStatusEntry]) -> Vec<StatusEntryUi> {
    entries
        .iter()
        .map(|e| {
            let is_debuff = e.category == "debuff";
            let is_low = e.duration_ms > 0 && e.remaining_ms < 3000;
            let ratio = if e.duration_ms == 0 {
                1.0
            } else {
                (e.remaining_ms as f32 / e.duration_ms as f32).clamp(0.0, 1.0)
            };
            let bar_color = if is_low {
                slint::Color::from_rgb_u8(0xff, 0x70, 0x43)
            } else if is_debuff {
                slint::Color::from_rgb_u8(0xef, 0x53, 0x50)
            } else {
                slint::Color::from_rgb_u8(0x4f, 0xc3, 0xf7)
            };
            let border_color = match e.priority.as_str() {
                "alert" => slint::Color::from_argb_u8(0xff, 0xff, 0xd5, 0x4f),
                "high" => slint::Color::from_argb_u8(0x40, 0xff, 0xff, 0xff),
                "low" => slint::Color::from_argb_u8(0x0f, 0xff, 0xff, 0xff),
                "hidden" => slint::Color::from_argb_u8(0x00, 0xff, 0xff, 0xff),
                _ => slint::Color::from_argb_u8(0x1f, 0xff, 0xff, 0xff),
            };
            StatusEntryUi {
                name: buff_names::label(e.base_id).into(),
                remaining_text: format::format_remaining(e.remaining_ms, e.duration_ms).into(),
                bar_ratio: ratio,
                bar_color,
                layer_text: if e.layer > 1 {
                    format!("×{}", e.layer).into()
                } else {
                    "".into()
                },
                is_low,
                border_color,
            }
        })
        .collect()
}

/// 円形タイマーの進捗アーク SVG（viewbox 28、中心14,14、半径12.5、上端から時計回り）。
fn buff_arc(ratio: f32) -> String {
    let p = ratio.clamp(0.0, 0.9999);
    let theta = p * std::f32::consts::TAU;
    let (cx, cy, r) = (14.0_f32, 14.0_f32, 12.5_f32);
    let end_x = cx + r * theta.sin();
    let end_y = cy - r * theta.cos();
    let large = if p > 0.5 { 1 } else { 0 };
    format!("M {cx} {} A {r} {r} 0 {large} 1 {end_x:.2} {end_y:.2}", cy - r)
}

fn buff_cell(snap: Option<&bpsr_core::models::SelfBuffSnapshot>, kind_hex: u32) -> BuffCell {
    let color =
        slint::Color::from_rgb_u8((kind_hex >> 16) as u8, (kind_hex >> 8) as u8, kind_hex as u8);
    match snap {
        Some(b) => {
            let active = b.remaining_ms > 0 || b.duration_ms <= 0;
            let ratio = if b.duration_ms <= 0 {
                0.0
            } else {
                (b.remaining_ms as f32 / b.duration_ms as f32).clamp(0.0, 1.0)
            };
            let text = if b.duration_ms <= 0 {
                "∞".to_string()
            } else if b.remaining_ms <= 0 {
                "OK".to_string()
            } else {
                let s = (b.remaining_ms as f64 / 1000.0).ceil() as i64;
                if s > 999 {
                    "999+".to_string()
                } else {
                    s.to_string()
                }
            };
            let text_color = if !active || b.remaining_ms <= 0 {
                slint::Color::from_rgb_u8(0x88, 0x88, 0x88)
            } else if b.duration_ms > 0 && b.remaining_ms < 3000 {
                slint::Color::from_rgb_u8(0xff, 0x52, 0x52)
            } else {
                slint::Color::from_rgb_u8(0xdd, 0xdd, 0xdd)
            };
            BuffCell {
                active,
                arc_commands: buff_arc(ratio).into(),
                color,
                text: text.into(),
                text_color,
            }
        }
        None => BuffCell {
            active: false,
            arc_commands: "".into(),
            color,
            text: "".into(),
            text_color: slint::Color::from_rgb_u8(0x88, 0x88, 0x88),
        },
    }
}

fn build_buff_rows(
    tracked: &bpsr_core::models::TrackedBuffsData,
    watched: &[i64],
) -> Vec<BuffPlayerRow> {
    watched
        .iter()
        .map(|&uid| {
            let snap = tracked.players.iter().find(|p| p.uid as i64 == uid);
            let name = snap.map(|s| s.name.clone()).unwrap_or_default();
            let display = if name.is_empty() {
                format!("{}", uid & 0xffff)
            } else {
                name
            };
            let find = |kind: &str| snap.and_then(|s| s.buffs.iter().find(|b| b.kind == kind));
            BuffPlayerRow {
                name: display.into(),
                tina: buff_cell(find("Tina"), 0xff4d6d),
                aluna: buff_cell(find("Aluna"), 0x5fd35f),
                tarta: buff_cell(find("Tarta"), 0xb98bff),
                basilisk: buff_cell(find("Basilisk"), 0xd9a05b),
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

    // 設定（%APPDATA%\bpsr-checker\settings.json）。UIスレッドで共有・編集する。
    let cfg = Rc::new(RefCell::new(settings::load()));
    // ウォッチリスト（バフタイマー追跡対象）。
    let wl = Rc::new(RefCell::new(watchlist::Watchlist::load()));

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

    // 自キャラ バフ/デバフ オーバーレイ（別ウィンドウ）
    let self_overlay = SelfStatusOverlay::new()?;
    let self_buffs = Rc::new(VecModel::<StatusEntryUi>::default());
    let self_debuffs = Rc::new(VecModel::<StatusEntryUi>::default());
    self_overlay.set_buffs(self_buffs.clone().into());
    self_overlay.set_debuffs(self_debuffs.clone().into());
    {
        let w = self_overlay.as_weak();
        self_overlay.on_start_drag(move || {
            if let Some(o) = w.upgrade() {
                overlay::start_drag(o.window());
            }
        });
    }

    // バフタイマー オーバーレイ（別ウィンドウ）
    let buff_overlay = BuffOverlay::new()?;
    let buff_players = Rc::new(VecModel::<BuffPlayerRow>::default());
    buff_overlay.set_players(buff_players.clone().into());
    {
        let w = buff_overlay.as_weak();
        buff_overlay.on_start_drag(move || {
            if let Some(o) = w.upgrade() {
                overlay::start_drag(o.window());
            }
        });
    }

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
        let wl_sel = wl.clone();
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
                    &wl_sel.borrow().watched,
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
    // ウォッチ切替（DPS一覧のピン）→ watchlist 更新・保存・即再描画
    {
        let w = main.as_weak();
        let wl_t = wl.clone();
        let enc_t = enc.clone();
        let rows_t = rows.clone();
        let cfg_t = cfg.clone();
        let tab_t = tab_cell.clone();
        main.on_toggle_watch(move |uid_str| {
            let uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            if uid == 0 {
                return;
            }
            {
                let mut wl = wl_t.borrow_mut();
                wl.toggle(uid);
                wl.save();
            }
            if w.upgrade().is_some() {
                let c = cfg_t.borrow();
                let pw = fetch_players(&enc_t, tab_t.get());
                rows_t.set_vec(build_rows(
                    &pw,
                    &c.name_template,
                    c.abbreviate_scores,
                    c.privacy_mask_names,
                    &wl_t.borrow().watched,
                ));
            }
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
        let self_ov = self_overlay.as_weak();
        let buff_ov = buff_overlay.as_weak();
        main.on_set_bool(move |key, val| {
            {
                let mut c = cfg_b.borrow_mut();
                match key.as_str() {
                    "self-status-overlay" => c.show_self_status_overlay = val,
                    "buff-overlay" => c.show_buff_overlay = val,
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
                    "three-min-auto-open" => c.three_min_auto_open = val,
                    other => log::warn!("unknown setting key: {other}"),
                }
            }
            let c = cfg_b.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
            }
            settings::save(&c);
            if key.as_str() == "self-status-overlay" {
                if let Some(o) = self_ov.upgrade() {
                    if c.show_self_status_overlay {
                        let _ = o.show();
                    } else {
                        let _ = o.hide();
                    }
                }
            }
            if key.as_str() == "buff-overlay" {
                if let Some(o) = buff_ov.upgrade() {
                    if c.show_buff_overlay {
                        let _ = o.show();
                    } else {
                        let _ = o.hide();
                    }
                }
            }
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
    // 数値設定ステッパー（key と方向 dir=±1）。キー毎に step/範囲を持ち、必要なら即適用。
    // poll-interval はポーリングタイマー再構築が要るため次回起動時に反映（永続化のみ）。
    {
        let w = main.as_weak();
        let cfg_n = cfg.clone();
        main.on_bump_num(move |key, dir| {
            let d = dir as f64;
            {
                let mut c = cfg_n.borrow_mut();
                match key.as_str() {
                    "combat-exit" => {
                        c.combat_exit_sec = (c.combat_exit_sec + d).clamp(0.0, 60.0);
                        compute::set_combat_exit_timeout(c.combat_exit_sec);
                    }
                    "poll-interval" => {
                        c.poll_interval_ms = (c.poll_interval_ms + d * 50.0).clamp(50.0, 2000.0);
                    }
                    "three-min-dur" => {
                        c.three_min_duration_sec = (c.three_min_duration_sec + d * 30.0).clamp(30.0, 1800.0);
                    }
                    "history-limit" => {
                        c.history_limit = (c.history_limit + d * 5.0).clamp(0.0, 100.0);
                        compute::set_history_limit(c.history_limit);
                    }
                    "ts-samples" => {
                        c.time_series_samples = (c.time_series_samples + d * 10.0).clamp(10.0, 200.0);
                        compute::set_time_series_config(c.time_series_samples, c.time_series_interval_ms);
                    }
                    "ts-interval" => {
                        c.time_series_interval_ms = (c.time_series_interval_ms + d * 250.0).clamp(250.0, 5000.0);
                        compute::set_time_series_config(c.time_series_samples, c.time_series_interval_ms);
                    }
                    "graph-count" => {
                        c.graph_player_count = (c.graph_player_count + d).clamp(0.0, 10.0);
                    }
                    "font-size" => {
                        c.font_size = (c.font_size + d).clamp(10.0, 18.0);
                    }
                    other => log::warn!("unknown num key: {other}"),
                }
            }
            let c = cfg_n.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
            }
            settings::save(&c);
        });
    }

    main.show()?;
    if cfg.borrow().show_self_status_overlay {
        let _ = self_overlay.show();
    }
    if cfg.borrow().show_buff_overlay {
        let _ = buff_overlay.show();
    }

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
    let self_overlay_w = self_overlay.as_weak();
    let self_buffs_poll = self_buffs.clone();
    let self_debuffs_poll = self_debuffs.clone();
    let buff_overlay_w = buff_overlay.as_weak();
    let buff_players_poll = buff_players.clone();
    let cfg_poll = cfg.clone();
    let wl_poll = wl.clone();
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
                &wl_poll.borrow().watched,
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

        // 自キャラ オーバーレイ更新（表示中のみ）
        if cfg_poll.borrow().show_self_status_overlay {
            if let Some(o) = self_overlay_w.upgrade() {
                let s = compute::get_self_buff_status(&enc_poll);
                o.set_waiting(s.local_player_uid == 0.0);
                self_buffs_poll.set_vec(build_status_entries(&s.buffs));
                self_debuffs_poll.set_vec(build_status_entries(&s.debuffs));
            }
        }

        // バフタイマー オーバーレイ更新（表示中のみ）
        if cfg_poll.borrow().show_buff_overlay {
            if let Some(o) = buff_overlay_w.upgrade() {
                let watched_i: Vec<i64> = wl_poll.borrow().watched.clone();
                o.set_empty(watched_i.is_empty());
                if !watched_i.is_empty() {
                    let uids: Vec<f64> = watched_i.iter().map(|&u| u as f64).collect();
                    let t = compute::get_tracked_buffs(&enc_poll, uids);
                    buff_players_poll.set_vec(build_buff_rows(&t, &watched_i));
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
