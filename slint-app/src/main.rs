// Blue Protocol: Star Resonance DPSチェッカー（Slint版・移行中）
// S1: core→Slint のライブ配線（capture スレッド→共有 EncounterMutex→UIポーリング）。
// リリースではコンソールを出さない（CJK の ICU 行分割警告は dev 時のみ・実害なし）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod best_records;
mod buff_names;
mod capture;
mod consumable_names;
mod dps_bar;
mod format;
#[cfg(windows)]
mod hotkey;
mod overlay;
mod settings;
#[cfg(windows)]
mod tray;
mod update;
mod watchlist;
mod window_state;

use bpsr_core::compute;
use bpsr_core::engine;
use bpsr_core::engine::encounter::EncounterMutex;
use slint::{ComponentHandle, Model, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const SETTLE_TICKS: u64 = 5;

/// オーバーレイ窓の枠操作コールバック(ドラッグ/リサイズ/最小化/×閉じ)を一括配線する。
/// self_overlay と buff_overlay で同一の配線を共有するためのマクロ。
/// `$close_key` は ×閉じで OFF にする設定トグルのキー(invoke_set_bool 経由)。
macro_rules! wire_overlay_chrome {
    ($overlay:expr, $main:expr, $close_key:literal) => {{
        {
            let w = $overlay.as_weak();
            $overlay.on_start_drag(move || {
                if let Some(o) = w.upgrade() {
                    overlay::start_drag(o.window());
                }
            });
        }
        {
            let w = $overlay.as_weak();
            $overlay.on_start_resize(move |dir| {
                if let Some(o) = w.upgrade() {
                    overlay::start_resize(o.window(), dir);
                }
            });
        }
        // ×閉じる → 設定トグルOFFと連動（invoke_set_bool で既存ハンドラを再利用）
        {
            let mw = $main.as_weak();
            $overlay.on_close_window(move || {
                if let Some(m) = mw.upgrade() {
                    m.invoke_set_bool($close_key.into(), false);
                }
            });
        }
        // 最小化（タスクバー常駐モード時のみボタン表示）→ OS最小化でタスクバーへ格納
        {
            let w = $overlay.as_weak();
            $overlay.on_minimize(move || {
                if let Some(o) = w.upgrade() {
                    overlay::minimize_window(o.window());
                }
            });
        }
    }};
}

/// 最小ロガー。core の capture / 本体の診断ログを stderr へ出す。
/// （Slint/parley の CJK 警告は log ではなく直接 eprintln のため別物・ここでは触れない）
struct ConsoleLog;
impl log::Log for ConsoleLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        let line = format!("[{}] {}: {}", r.level(), r.target(), r.args());
        eprintln!("{line}");
        // 管理者権限(UAC)起動では cargo の端末に stderr が届かないため、診断用にファイルへも出す。
        // 起動ごとに truncate して 1 起動 = 1 ファイルにする。
        log_to_file(&line);
    }
    fn flush(&self) {}
}

/// ログ出力先ファイルのパス（%APPDATA%\bpsr-checker\bpsr-checker.log）。
fn log_file_path() -> std::path::PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(base)
        .join("bpsr-checker")
        .join("bpsr-checker.log")
}

/// ログ1行をファイルへ追記（初回呼び出しで truncate して開く）。
fn log_to_file(line: &str) {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    static FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
    let m = FILE.get_or_init(|| {
        let path = log_file_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .ok();
        Mutex::new(f)
    });
    if let Ok(mut g) = m.lock() {
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// イマジン専用モード案内の拡大率（1.0〜1.6）。窓の論理サイズから算出する。
/// 900x620 以下で 1.0（＝他の空状態案内の約2倍の基準サイズ）、広いほど伸ばして 1.6 で頭打ち。
/// 幅と高さの小さい方に合わせ、横長・縦潰れの窓で文字がはみ出さないようにする。
fn notice_scale(win: &MainWindow) -> f32 {
    let factor = win.window().scale_factor();
    if factor <= 0.0 {
        return 1.0;
    }
    let size = win.window().size();
    let w = size.width as f32 / factor;
    let h = size.height as f32 / factor;
    (w / 900.0).min(h / 620.0).clamp(1.0, 1.6)
}

/// 3分計測 結果モーダルの文字倍率（1.0〜1.3）。notice_scale と同じ理由（.slint 側で窓サイズ
/// から font-size を計算すると「文字サイズ→preferred-height→レイアウト→窓サイズ」の束縛
/// ループになる）で Rust 側が窓サイズから算出して毎ポーリング set する。設定パネルの
/// 「フォントサイズ」(font_scale。本体テーブル用)には一切連動させない＝独立した軸。
/// 基準は app.slint の MainWindow.preferred-width/height（520x360）＝リサイズ前の既定サイズ。
/// これ以下では 1.0 のまま＝普段リサイズしないユーザーの見た目は変わらない。notice_scale
/// （案内文1つ用・上限1.6）より上限を抑えているのは、結果モーダルは表・凡例・折れ線グラフの
/// 軸ラベルなど固定px幅の列を多数抱えており、拡大しすぎると数値やボタン文言がはみ出す
/// リスクがあるため（アクション行のボタン幅は文字と一緒には拡げていない）。
fn result_scale(win: &MainWindow) -> f32 {
    let factor = win.window().scale_factor();
    if factor <= 0.0 {
        return 1.0;
    }
    let size = win.window().size();
    let w = size.width as f32 / factor;
    let h = size.height as f32 / factor;
    (w / 520.0).min(h / 360.0).clamp(1.0, 1.3)
}

/// 二重起動防止（Windows 名前付き Mutex）。既に起動済みなら true。
#[cfg(windows)]
fn already_running() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::PCWSTR;
    // The MCP debug process must coexist with the normal UI process.
    let mutex_name = if cfg!(feature = "mcp") && std::env::var_os("SLINT_MCP_PORT").is_some() {
        "Global\\bpsr-checker-slint-mcp-instance\0"
    } else {
        "Global\\bpsr-checker-slint-instance\0"
    };
    let name: Vec<u16> = mutex_name
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

/// 既定ブラウザで URL を開く（フッターの「お問い合わせ」「GitHubで報告」用）。
/// 失敗しても UI は落とさず warn ログのみ出す。
#[cfg(windows)]
fn open_url(url: &str) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;
    let op: Vec<u16> = "open\0".encode_utf16().collect();
    let file: Vec<u16> = format!("{url}\0").encode_utf16().collect();
    unsafe {
        let result = ShellExecuteW(
            HWND(0),
            PCWSTR(op.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        // ShellExecute は成功時 32 より大きい値（HINSTANCE 相当）を返す。
        if result.0 <= 32 {
            log::warn!("open_url failed: {url} (code {})", result.0);
        }
    }
}

#[cfg(not(windows))]
fn open_url(url: &str) {
    log::warn!("open_url not supported on this platform: {url}");
}

/// 通知カードの世代カウンタ。UI スレッドだけが触るが、通信スレッド経由で UI へ戻る
/// 閉包に入る（＝Send が要る）ため Rc/Cell ではなく Arc/Atomic を使う。
type ToastGen = Arc<AtomicU64>;

/// 終了時に落としておく状態（名前キャッシュ・自キャラUID・食事/シロップ）。
/// 通常終了とアプリ内更新の両方から呼ぶ。何度呼んでも同じ状態を書き直すだけで副作用はない。
fn persist_state(enc: &EncounterMutex) {
    engine::name_cache::flush();
    engine::selected_uid::flush();
    compute::save_consumables(enc);
}

/// 更新の通知カードを開き、15 秒後に自動で閉じる。
///
/// 閉じるのは「OK」かこのタイマーのどちらか早い方。カードを開き直すと世代が上がり、
/// 古いタイマーは何もしない（＝先に張られたタイマーが後のカードを早く閉じない）。
/// ダウンロード中はユーザーが結果を待っている最中なので、時間切れでも閉じない。
fn open_update_toast(m: &MainWindow, toast_gen: &ToastGen) {
    let generation = toast_gen.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    m.set_update_toast_open(true);
    let w = m.as_weak();
    let toast_gen = toast_gen.clone();
    Timer::single_shot(Duration::from_secs(UPDATE_TOAST_SECS), move || {
        let Some(m) = w.upgrade() else { return };
        if toast_gen.load(Ordering::Relaxed) == generation
            && m.get_update_state() != UpdateState::Downloading
        {
            m.set_update_toast_open(false);
        }
    });
}

/// 通知カードを自動で閉じるまでの秒数（app.slint の説明文と対で読むこと）。
const UPDATE_TOAST_SECS: u64 = 15;

/// 「更新を確認」を開始する（実処理は update.rs、通信は専用スレッド）。
///
/// `auto` は起動時の自動確認。自動確認では「最新でした」を通知せず、失敗もログだけに残す
/// （ユーザーが押していない確認の結果でカードを出すのは邪魔になるため）。手動確認では逆に、
/// 押した以上は結果を必ずカードで返す。
fn start_update_check(
    m: &MainWindow,
    releases: &Arc<std::sync::Mutex<Vec<update::Release>>>,
    toast_gen: &ToastGen,
    auto: bool,
) {
    // 確認中・ダウンロード中の多重起動を防ぐ（ボタン側の enabled と二重の歯止め）。
    let state = m.get_update_state();
    if state == UpdateState::Checking || state == UpdateState::Downloading {
        return;
    }
    m.set_update_state(UpdateState::Checking);
    m.set_update_error(slint::SharedString::new());
    let releases = releases.clone();
    let toast_gen = toast_gen.clone();
    let w = m.as_weak();
    std::thread::spawn(move || {
        let res = update::fetch_releases(update::RELEASE_LIST_LIMIT);
        // インストーラ版のみワンクリック更新の対象（ポータブルは実行中 exe の自己置換になる）。
        let installed_build = update::is_installed_build();
        let _ = w.upgrade_in_event_loop(move |m| match res {
            Ok(list) => {
                // 一覧の先頭が最新版（update.rs がバージョン降順に整えている）。
                let newest = list[0].clone();
                let newer = newest.is_newer_than_current();
                m.set_update_releases(release_entries(&list));
                if let Ok(mut slot) = releases.lock() {
                    *slot = list;
                }
                m.set_update_version(newest.version.text().into());
                // 「インストーラ版で動いているか」と「その版を入れ替えられるか」は別条件。
                // 前者は行の文言（入れ替える/ページを開く）、後者は各行の可否に効く。
                m.set_update_installed_build(installed_build);
                m.set_update_installable(installed_build && newest.can_install());
                m.set_update_state(if newer { UpdateState::Available } else { UpdateState::Latest });
                if newer || !auto {
                    open_update_toast(&m, &toast_gen);
                }
            }
            Err(e) => {
                log::warn!("更新確認に失敗: {e}");
                if auto {
                    m.set_update_state(UpdateState::Idle);
                } else {
                    m.set_update_error(e.user_message().into());
                    m.set_update_state(UpdateState::Failed);
                    open_update_toast(&m, &toast_gen);
                }
            }
        });
    });
}

/// リリース一覧 → 設定パネルのバージョン選択に出す行。
/// `installable` はその版の配布物だけで決まる（インストーラ版で動いているかは
/// `update-installed-build` として UI へ別に渡す）。
fn release_entries(list: &[update::Release]) -> slint::ModelRc<ReleaseEntryUi> {
    let rows: Vec<ReleaseEntryUi> = list
        .iter()
        .map(|r| ReleaseEntryUi {
            version: r.version.text().into(),
            current: r.is_current(),
            newer: r.is_newer_than_current(),
            installable: r.can_install(),
        })
        .collect();
    slint::ModelRc::new(VecModel::from(rows))
}

/// 選んだバージョンをダウンロード→SHA-256 検証→インストーラ起動→アプリ終了。
/// `index` は確認で得たリリース一覧の位置（0＝最新）。バージョン選択からの過去版も同じ経路。
///
/// 検証に通るまでアプリは終了させない（未署名の配布物が Defender に隔離された場合に
/// 「アプリだけ終了して更新は入らない」状態を作らないため。update.rs の契約と対）。
fn start_update_install(
    m: &MainWindow,
    releases: &Arc<std::sync::Mutex<Vec<update::Release>>>,
    enc: &Arc<EncounterMutex>,
    toast_gen: &ToastGen,
    index: usize,
) {
    let state = m.get_update_state();
    if state == UpdateState::Downloading || state == UpdateState::Launched {
        return;
    }
    let asset = releases
        .lock()
        .ok()
        .and_then(|list| list.get(index).and_then(|r| r.installer.clone()));
    let Some(asset) = asset else {
        let e = update::UpdateError::NoInstaller;
        log::warn!("更新の適用を中止: {e}");
        m.set_update_error(e.user_message().into());
        m.set_update_state(UpdateState::Failed);
        open_update_toast(m, toast_gen);
        return;
    };
    m.set_update_state(UpdateState::Downloading);
    m.set_update_progress(0.0);
    // 通知カードは開いたままにする。設定パネルを開いていないユーザーにとっては、ここが
    // 進捗と失敗を知る唯一の場所になる（カードから「更新する」を押した場合が該当）。
    open_update_toast(m, toast_gen);
    let enc = enc.clone();
    let toast_gen = toast_gen.clone();
    let w = m.as_weak();
    std::thread::spawn(move || {
        // 進捗は 1% 刻みでのみ UI へ返す（チャンク毎に invoke するとイベントループを溢れさせる）。
        let wp = w.clone();
        let mut last_pct = -1i32;
        let res = update::download_verified(&asset, move |done, total| {
            if total == 0 {
                return;
            }
            let pct = ((done * 100 / total) as i32).min(100);
            if pct == last_pct {
                return;
            }
            last_pct = pct;
            let _ = wp.upgrade_in_event_loop(move |m| {
                m.set_update_progress(pct as f32 / 100.0);
            });
        });
        let _ = w.upgrade_in_event_loop(move |m| {
            // インストーラは .onInit で taskkill するため、起動した瞬間からこちらは
            // いつ落とされてもおかしくない。永続化は**起動より前**に済ませる
            // （main の末尾にある通常終了時の保存は、強制終了されると走らない）。
            let outcome = res.and_then(|path| {
                persist_state(&enc);
                update::launch_installer(&path)
            });
            match outcome {
                Ok(()) => {
                    m.set_update_state(UpdateState::Launched);
                    // 状態表示を一瞬見せてから畳む。ここで落とされても保存済み。
                    Timer::single_shot(Duration::from_millis(800), || {
                        let _ = slint::quit_event_loop();
                    });
                }
                Err(e) => {
                    log::warn!("更新の適用に失敗: {e}");
                    m.set_update_error(e.user_message().into());
                    m.set_update_state(UpdateState::Failed);
                    open_update_toast(&m, &toast_gen);
                }
            }
        });
    });
}

fn data_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(base).join("bpsr-checker")
}

/// タブが表す集計指標の種別。タブ番号→意味の対応は `tab_stat` 1箇所に集約し、`fetch_players`
/// （どの集計を取得するか）・`get_header_info`/`get_skills` 呼び出し（core 側の
/// `bpsr_core::compute::StatType` を返す）が同じ判定から導出する（タブ追加時に片方だけ直しても
/// 動いてしまう＝レビューで初めて発覚する類の乖離を防ぐ）。
use bpsr_core::compute::StatType;

/// タブ番号(0=dps 1=heal 2=taken 3=history)→集計指標。history(3) は S5 実装まで dmg を暫定表示。
/// 設定から計測スコープを組む。計測開始のたびにここを通し、`start_3min_measure_mode` へ渡した
/// 値は `MeasureMode` が計測終了まで運ぶ（走行中に設定を変えても結果がぶれない）。
fn measure_scope(c: &settings::Settings) -> bpsr_core::engine::encounter::MeasureScope {
    bpsr_core::engine::encounter::MeasureScope {
        first_target_only: c.measure_first_target_only,
        self_only: c.measure_self_only,
    }
}

fn tab_stat(tab: i32) -> StatType {
    match tab {
        1 => StatType::Heal,
        2 => StatType::DmgTaken,
        _ => StatType::Dmg,
    }
}

/// タブに応じてプレイヤー一覧を取得。
fn fetch_players(enc: &EncounterMutex, tab: i32) -> bpsr_core::models::PlayersWindow {
    match tab_stat(tab) {
        StatType::Heal => compute::get_heal_players(enc),
        StatType::DmgTaken => compute::get_dmg_taken_players(enc),
        StatType::Dmg => compute::get_dps_players(enc),
        // tab_stat は生成しないが、core と共有の StatType を網羅させる。
        StatType::DmgBossOnly => compute::get_dps_boss_players(enc),
    }
}

/// ヘッダー・合計行（issue #9 PR1）の合計DPS/経過時間/合計ダメージ量をUIへ反映する。
/// poll（定期更新）とタブ切替（on_select_tab）の両方から呼ぶこと。タブ切替は行だけ即時
/// 再構築しヘッダー反映はしていなかったため、切替直後は次のpollまで前タブの値が残っていた。
fn refresh_header(m: &MainWindow, enc: &EncounterMutex, tab: i32) {
    let header = compute::get_header_info(enc, tab_stat(tab));
    m.set_total_text(format::format_dps(header.total_dps).into());
    m.set_elapsed_text(format::format_elapsed(header.elapsed_ms).into());
    m.set_total_dmg_text(format::format_number(header.total_dmg).into());
    m.set_content_name(
        engine::content_names::content_label(
            header.fight_level_map_id,
            0,
            engine::runtime_settings::display_lang(),
        )
        .into(),
    );
}

/// グラフ列(DPS推移)を出すか。graph設定が有効で、被ダメ(tab=2)以外。
fn graph_col_active(c: &settings::Settings, tab: i32) -> bool {
    (c.graph_player_count > 0.0 || c.graph_for_local_player) && tab != 2
}

/// 通常 rows へ反映しつつ、軽量分割表示用に前半/後半カラムへも分配する。行の集合からしか
/// 導けない表示フラグ（自分基準ガイド線・自キャラ名の未取得ヒント）も同時に反映する
/// （バー計算のフォールバック条件とガイド線の表示条件を1つの値から導出するため）。
fn apply_player_rows(
    m: &MainWindow,
    rows: &slint::VecModel<Row>,
    left: &slint::VecModel<Row>,
    right: &slint::VecModel<Row>,
    built: BuiltPlayerRows,
) {
    let half = built.rows.len().div_ceil(2);
    sync_rows(left, &built.rows[..half]);
    sync_rows(right, &built.rows[half..]);
    sync_rows(rows, &built.rows);
    m.set_self_guide_has_data(built.self_guide_has_data);
    m.set_local_name_unresolved(built.local_name_unresolved);
}

/// 行数が同じならデリゲートを再生成せず in-place 更新する。
/// set_vec はモデルをリセットしリピータが要素を作り直すため、ホバー中の
/// 食事/シロップ ツールチップ（PopupWindow）が毎poll閉じ開きしてちらつく。
/// 行数が一致する間は set_row_data でデータのみ差し替え、ホバー状態を保つ。
fn sync_rows(model: &slint::VecModel<Row>, data: &[Row]) {
    if model.row_count() == data.len() {
        for (i, r) in data.iter().enumerate() {
            model.set_row_data(i, r.clone());
        }
    } else {
        model.set_vec(data.to_vec());
    }
}

/// テンプレート展開結果を Slint の名前列モデルへ変換する。
fn ui_name_parts(parts: Vec<format::NamePart>) -> slint::ModelRc<NamePart> {
    slint::ModelRc::new(VecModel::from(
        parts
            .into_iter()
            .map(|part| NamePart {
                text: part.text.into(),
                class_icon: part.class_icon,
                shrink_rank: part.shrink_rank,
                has_name: part.has_name,
            })
            .collect::<Vec<_>>(),
    ))
}

/// 食事/シロップの残り時間の警告閾値（ms）。1分未満=危険、5分未満=注意。
/// build_status_entries の is_low（残り3秒未満）とはスケールが異なる別判定
/// （消耗品の duration は30分オーダーのため、同じ「残りわずか」でも基準が違う）。
const CONSUMABLE_CRITICAL_MS: f64 = 60_000.0;
const CONSUMABLE_CAUTION_MS: f64 = 300_000.0;
/// 警告色は既存パレットを踏襲する。危険=build_status_entries の is_low(#ff7043) と同色、
/// 注意=完全透明オーバーレイ警告バナー(app.slint)と同色。
const CONSUMABLE_CRITICAL_RGB: (u8, u8, u8) = (0xff, 0x70, 0x43);
const CONSUMABLE_CAUTION_RGB: (u8, u8, u8) = (0xff, 0xb4, 0x54);
/// 食事/シロップアイコンの基準色（警告閾値に掛からない通常時の色）。
/// 旧来 app.slint 側にリテラルで持たせていたが、閾値判定と1箇所にまとめるため Rust 側へ移設。
const FOOD_TINT_RGB: (u8, u8, u8) = (0x66, 0xbb, 0x6a);
const SYRUP_TINT_RGB: (u8, u8, u8) = (0xb0, 0x7c, 0xff);

#[allow(clippy::too_many_arguments)]
/// 食事/シロップ等の消耗バフの表示用派生値を計算する。
/// 戻り値: (アクティブか, 残量割合0..1, 残り時間テキスト, 種類ラベル, アイコン/ラベル色)。
/// duration/remaining いずれかが 0 以下なら未使用扱い(空文字・0)。
///
/// 残り時間の警告閾値判定はここに集約する。色は app.slint 側でアイコンの塗りと
/// ホバー時の種類ラベル文字色（tip-label の `color: root.tint`）の両方に使われるため、
/// 判定をこの1箇所だけに置けば両方の表示が自動的に揃う（条件式を2箇所に書かない）。
fn consumable_display(
    remaining_ms: f64,
    duration_ms: f64,
    base_id: i32,
    base_tint_rgb: (u8, u8, u8),
) -> (bool, f32, String, String, slint::Color) {
    let rgb = |c: (u8, u8, u8)| slint::Color::from_rgb_u8(c.0, c.1, c.2);
    if duration_ms <= 0.0 || remaining_ms <= 0.0 {
        return (false, 0.0, String::new(), String::new(), rgb(base_tint_rgb));
    }
    let ratio = (remaining_ms / duration_ms).clamp(0.0, 1.0) as f32;
    let time = format::format_consumable_remaining(remaining_ms as i64, duration_ms as i64);
    let label = consumable_names::label(base_id).unwrap_or_default();
    let tint = if remaining_ms < CONSUMABLE_CRITICAL_MS {
        rgb(CONSUMABLE_CRITICAL_RGB)
    } else if remaining_ms < CONSUMABLE_CAUTION_MS {
        rgb(CONSUMABLE_CAUTION_RGB)
    } else {
        rgb(base_tint_rgb)
    };
    (true, ratio, time, label, tint)
}

/// [`build_rows`] の結果。行データに加えて、行の集合からしか導けない表示フラグを返す。
struct BuiltPlayerRows {
    rows: Vec<Row>,
    /// 自分基準モードの 50% ガイド線を出せるか（＝自キャラの実績があるか）。
    self_guide_has_data: bool,
    /// 自キャラの名前が未取得（「プレイヤー#XXXX」表示のまま）か。名前の取得経路は入場時の
    /// EnterScene / SyncContainerData だけで、他プレイヤーのように視界の出入りでは補充されない。
    /// このためゲーム起動後にアプリを立ち上げると自分の名前だけ埋まらず、ゾーン入場
    /// （＝読み込みが入る移動・再ログイン）まで回復しない。
    /// 名前マスク中は伏せ字が正常な状態なのでヒントを出さない。
    local_name_unresolved: bool,
}

/// 行の組み立てに必要な、一覧と履歴の展開行で共通の文脈。
struct RowCtx<'a> {
    template: &'a str,
    abbreviate: bool,
    privacy: bool,
    bar_cfg: &'a dps_bar::DpsBarConfig,
    /// バー比率の分母（最大値。1 未満は 1 に丸め済み）。
    top: f64,
    /// 自キャラの total_value（自分基準バー用。[`self_total_of`]）。
    self_total: Option<f64>,
    /// 名前から職アイコンを除く（履歴の展開行）。
    strip_icons: bool,
}

/// 自キャラ行の total_value。`dps_bar::bar_pct` の SelfRelative フォールバック判定の入力。
fn self_total_of(rows: &[bpsr_core::models::PlayerRow], local_uid: f64) -> Option<f64> {
    rows.iter().find(|p| p.uid == local_uid).map(|p| p.total_value)
}

/// 有効DPSの表示。有効DPS追加（2026-08-16）以前の履歴は 0 で保存されており、DPS があるのに
/// 0 と出すと「有効DPS 0」と誤読されるため「-」にする。
fn eff_dps_text(p: &bpsr_core::models::PlayerRow) -> String {
    if p.active_value_per_sec == 0.0 && p.value_per_sec > 0.0 {
        "-".to_string()
    } else {
        format::format_dps(p.active_value_per_sec)
    }
}

/// 1プレイヤー分の [`Row`] を組み立てる。一覧（build_rows）と履歴の展開行が共有し、
/// 名前・数値の書式を2か所に持たない。`spark` と `watched` は一覧固有の表示状態。
fn build_row(
    p: &bpsr_core::models::PlayerRow,
    rank: i32,
    is_local: bool,
    watched: bool,
    spark: String,
    ctx: &RowCtx,
) -> Row {
    // 食事/シロップの残量割合（0..1。アイコンの色が上から縦に抜ける）＋ホバー用の
    // 残り時間テキスト・種類ラベル（base_id→日本語効果名）。
    let (food_act, food_remaining, food_time, food_label, food_tint) =
        consumable_display(p.food_remaining_ms, p.food_duration_ms, p.food_base_id, FOOD_TINT_RGB);
    let (syrup_act, syrup_remaining, syrup_time, syrup_label, syrup_tint) =
        consumable_display(p.syrup_remaining_ms, p.syrup_duration_ms, p.syrup_base_id, SYRUP_TINT_RGB);
    let display = if ctx.privacy {
        format::mask_player_name(p.uid as i64)
    } else {
        p.name.clone()
    };
    let name_parts = format::format_row_name_parts(
        &display,
        &p.class_name,
        &p.class_spec_name,
        p.ability_score,
        p.season_level,
        p.season_strength,
        &p.imagine_suffix,
        &p.role_skill_suffix,
        rank,
        ctx.template,
        ctx.abbreviate,
    );
    let name_parts = if ctx.strip_icons {
        format::strip_icon_parts(name_parts, &display)
    } else {
        name_parts
    };
    Row {
        rank,
        uid_str: format!("{}", p.uid as i64).into(),
        name_parts: ui_name_parts(name_parts),
        class_color: format::class_color(&p.class_name),
        class_icon_id: format::class_icon_id(&p.class_name),
        class_role_color: format::class_role_color(&p.class_name),
        dmg_text: format::format_number(p.total_value).into(),
        dps_text: format::format_dps(p.value_per_sec).into(),
        pct_text: format::format_pct(p.value_pct).into(),
        pct: dps_bar::bar_pct(ctx.bar_cfg, p, ctx.top, ctx.self_total),
        is_local,
        crit_text: format::format_pct(p.crit_rate).into(),
        crit_value_text: format::format_pct(p.crit_value_rate).into(),
        lucky_text: format::format_pct(p.lucky_rate).into(),
        lucky_value_text: format::format_pct(p.lucky_value_rate).into(),
        hits_text: format!("{}", p.hits as i64).into(),
        hpm_text: format!("{:.1}", p.hits_per_minute).into(),
        score_text: if p.ability_score > 0.0 {
            format::format_score(p.ability_score, ctx.abbreviate)
        } else {
            "-".to_string()
        }
        .into(),
        eff_dps_text: eff_dps_text(p).into(),
        watched,
        spark_commands: spark.into(),
        food_active: food_act,
        food_remaining,
        food_time: food_time.into(),
        food_label: food_label.into(),
        food_tint,
        syrup_active: syrup_act,
        syrup_remaining,
        syrup_time: syrup_time.into(),
        syrup_label: syrup_label.into(),
        syrup_tint,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_rows(
    pw: &bpsr_core::models::PlayersWindow,
    template: &str,
    abbreviate: bool,
    privacy: bool,
    watched: &[i64],
    graph_count: i32,
    graph_for_local: bool,
    bar_cfg: &dps_bar::DpsBarConfig,
) -> BuiltPlayerRows {
    let local = pw.local_player_uid;
    // 自分基準モード以外では未使用だが、行数は高々十数人なので線形探索のコストは無視できる
    // （モード判定を dps_bar::bar_pct 側の1箇所に集約するため、ここでは無条件に求める）。
    let self_total = self_total_of(&pw.player_rows, local);
    // dps_bar::bar_pct の SelfRelative フォールバック条件と同一の判定（app.slint の
    // self-guide-has-data へ渡し、ガイド線の表示条件をバー計算のフォールバックと一致させる）。
    let self_guide_has_data = self_total.is_some_and(|s| s > 0.0);
    let ctx = RowCtx {
        template,
        abbreviate,
        privacy,
        bar_cfg,
        top: pw.top_value.max(1.0),
        self_total,
        strip_icons: false,
    };
    // 非ローカルの上位 graph_count 人＋（設定時）ローカルにグラフを出す。
    let mut non_local_above: i32 = 0;
    let mut local_name_unresolved = false;
    let mut out = Vec::with_capacity(pw.player_rows.len());
    for (i, p) in pw.player_rows.iter().enumerate() {
        let rank = (i + 1) as i32;
        let is_local = p.uid == local;
        if is_local && !p.name_resolved {
            local_name_unresolved = true;
        }
        let show_spark = if is_local {
            graph_for_local
        } else {
            non_local_above < graph_count
        };
        let spark = if show_spark {
            build_spark_commands(&p.time_series)
        } else {
            String::new()
        };
        if !is_local {
            non_local_above += 1;
        }
        out.push(build_row(
            p,
            rank,
            is_local,
            watched.contains(&(p.uid as i64)),
            spark,
            &ctx,
        ));
    }
    BuiltPlayerRows {
        rows: out,
        self_guide_has_data,
        // マスク中は全員が伏せ字＝自分だけ名前が出ない状態ではないため、ヒントは出さない。
        local_name_unresolved: local_name_unresolved && !privacy,
    }
}

fn build_skill_rows(sw: &bpsr_core::models::SkillsWindow) -> Vec<SkillRowUi> {
    let top = sw.top_value.max(1.0);
    sw.skill_rows
        .iter()
        .map(|s| {
            let (_, ec) = format::element_label(s.element);
            SkillRowUi {
                uid_str: format!("{}", s.uid as i64).into(),
                name: s.name.clone().into(),
                elem_id: s.element as i32,
                elem_color: ec,
                total_text: format::format_number(s.total_value).into(),
                dps_text: format::format_dps(s.value_per_sec).into(),
                pct_text: format::format_pct(s.value_pct).into(),
                pct: ((s.total_value / top) * 100.0) as f32,
            }
        })
        .collect()
}

/// 日時表示の書式（履歴見出し・シェア画像の透かしで共有）。
const DATETIME_DISPLAY_FORMAT: &str = "%Y-%m-%d %H:%M";

/// 履歴見出しのタイトル: "{YYYY-MM-DD HH:MM} {コンテンツ名}"（issue #9 PR2b）。
/// 旧 history.json（levelMapId フィールド自体が無く0扱い）は content_name が空になるため
/// 日付のみになる。start-ms が 0（実際には起きない想定外値への防御）なら日付を省き
/// content_name のみ、両方欠けていれば空文字。
fn build_history_title(start_ms: f64, content_name: &str) -> String {
    use chrono::TimeZone;
    let date = if start_ms > 0.0 {
        chrono::Local
            .timestamp_millis_opt(start_ms as i64)
            .single()
            .map(|dt| dt.format(DATETIME_DISPLAY_FORMAT).to_string())
    } else {
        None
    };
    match (date, content_name.is_empty()) {
        (Some(d), true) => d,
        (Some(d), false) => format!("{d} {content_name}"),
        (None, true) => String::new(),
        (None, false) => content_name.to_string(),
    }
}

/// 履歴ビューの行高（px。font-scale を掛ける前）。プレイヤー行は一覧の行高(20px)と同じ。
const HISTORY_HEADER_ROW_H: f32 = 24.0;
const HISTORY_PLAYER_ROW_H: f32 = 20.0;
const HISTORY_SKILL_ROW_H: f32 = 18.0;

/// 履歴ビューのフラット行を構築（見出し → プレイヤー → スキル）。
fn build_history_rows(
    hist: &[bpsr_core::models::EncounterSnapshot],
    expanded: Option<i64>,
    expanded_player: Option<(i64, i64)>,
    c: &settings::Settings,
) -> Vec<HistoryRowUi> {
    let privacy = c.privacy_mask_names;
    let bar_cfg = dps_bar_config(c);
    let mut out = Vec::new();
    for snap in hist {
        let id = snap.id as i64;
        let is_exp = expanded == Some(id);
        let player_names = snap
            .player_rows
            .iter()
            .map(|p| {
                if privacy {
                    format::mask_player_name(p.uid as i64)
                } else {
                    p.name.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" / ");
        out.push(HistoryRowUi {
            is_header: true,
            is_skill: false,
            snap_id: format!("{id}").into(),
            toggle_key: format!("h:{id}").into(),
            expanded: is_exp,
            duration_text: format::format_elapsed(snap.duration_ms).into(),
            dps_text: format::format_dps(snap.total_dps).into(),
            dmg_text: format::format_number(snap.total_dmg).into(),
            count_text: format!("{}", snap.player_rows.len()).into(),
            name: player_names.into(),
            title: build_history_title(
                snap.start_ms,
                &engine::content_names::content_label(
                    snap.level_map_id,
                    0,
                    engine::runtime_settings::display_lang(),
                ),
            )
            .into(),
            // 条件付きの計測は通常の計測と直接比較できない。一覧の時点でそれが分かるようにする。
            scope_text: measure_scope_label(snap.measure_scope).into(),
            ..Default::default()
        });
        if is_exp {
            // 一覧と同じ組み立て（build_row）へ、記録時点の自キャラ・最大値・設定を渡す。
            let ctx = RowCtx {
                template: &c.name_template,
                abbreviate: c.abbreviate_scores,
                privacy,
                bar_cfg: &bar_cfg,
                top: snap.player_rows.iter().map(|p| p.total_value).fold(1.0, f64::max),
                self_total: self_total_of(&snap.player_rows, snap.local_player_uid),
                strip_icons: true,
            };
            for (i, p) in snap.player_rows.iter().enumerate() {
                let player_uid = p.uid as i64;
                let has_skills = snap
                    .player_skill_rows
                    .iter()
                    .find(|s| s.player_uid as i64 == player_uid)
                    .is_some_and(|s| !s.skill_rows.is_empty());
                let player_is_expanded = has_skills && expanded_player == Some((id, player_uid));
                out.push(HistoryRowUi {
                    is_header: false,
                    is_skill: false,
                    rank_text: format!("{}.", i + 1).into(),
                    // 履歴では自キャラのマーカー・ピン・スパークは出さない。
                    row: build_row(p, (i + 1) as i32, false, false, String::new(), &ctx),
                    toggle_key: if has_skills {
                        format!("p:{id}:{player_uid}").into()
                    } else {
                        String::new().into()
                    },
                    expanded: player_is_expanded,
                    ..Default::default()
                });

                if player_is_expanded {
                    if let Some(skill_snapshot) = snap
                        .player_skill_rows
                        .iter()
                        .find(|s| s.player_uid as i64 == player_uid)
                    {
                        for skill in &skill_snapshot.skill_rows {
                            let (_, elem_color) = format::element_label(skill.element);
                            out.push(HistoryRowUi {
                                is_header: false,
                                is_skill: true,
                                name: skill.name.clone().into(),
                                elem_id: skill.element as i32,
                                elem_color,
                                skill_total_text: format::format_number(skill.total_value).into(),
                                skill_dps_text: format::format_dps(skill.value_per_sec).into(),
                                skill_pct_text: format::format_pct(skill.value_pct).into(),
                                skill_pct: skill.value_pct.clamp(0.0, 100.0) as f32,
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }
    }
    // 行高(font-scale 倍前)と積算位置。.slint は行を y で直接置く。
    let mut y = 0.0_f32;
    for row in &mut out {
        row.h_units = if row.is_header {
            HISTORY_HEADER_ROW_H
        } else if row.is_skill {
            HISTORY_SKILL_ROW_H
        } else {
            HISTORY_PLAYER_ROW_H
        };
        row.y_units = y;
        y += row.h_units;
    }
    out
}

/// 折れ線パスの座標系。**0..1 の正規化座標**で生成し、.slint 側の Path が
/// `viewbox-width/height: 1` ＋ `fit: fill` で要素サイズへ引き伸ばす。
/// 実寸(px)を Rust 側へ渡す経路を持たないため、レイアウト前に生成したパスでも縮尺がズレない
/// （issue #7 の回帰防止。旧実装はプロット実寸px座標で生成し `changed width/height` で
/// 再生成していたが、モーダルを開いた初回は発火せず暫定値のまま描かれていた）。
///
/// **この値を変えるときは app.slint 側の `viewbox-width` / `viewbox-height`
/// （SparkGraph の Path と PlayerRowItem の推移列 Path）も同じ値にすること。**
/// 食い違うと fit:fill が別の倍率で引き伸ばし、折れ線だけが軸とズレる。
const SPARK_VB: f32 = 1.0;

/// 正規化座標1点を M/L コマンドへ。座標系が 0..1 のため小数4桁で出す
/// （1500px 幅でも 0.15px 相当の分解能があり、目視では等倍と区別できない）。
fn spark_point(s: &mut String, first: bool, x: f32, y: f32) {
    if first {
        s.push_str(&format!("M {x:.4} {y:.4}"));
    } else {
        s.push_str(&format!(" L {x:.4} {y:.4}"));
    }
}

/// 時系列を正規化座標(0..1)の折れ線 SVG パスへ。値抽出は sel で指定。Sparkline.tsx 移植。
/// 点が2未満なら空文字（呼び出し側で非表示判定に使う）。
fn build_spark_with(
    points: &[bpsr_core::models::TimeSeriesPoint],
    sel: impl Fn(&bpsr_core::models::TimeSeriesPoint) -> f64,
) -> String {
    if points.len() < 2 {
        return String::new();
    }
    let max = points.iter().map(&sel).fold(1.0_f64, f64::max);
    let step = SPARK_VB / (points.len() - 1) as f32;
    let mut s = String::with_capacity(points.len() * 18);
    for (i, p) in points.iter().enumerate() {
        let x = i as f32 * step;
        let y = SPARK_VB - (sel(p) / max) as f32 * SPARK_VB;
        spark_point(&mut s, i == 0, x, y);
    }
    s
}

/// 窓DPS の折れ線（ヘッダー/プレイヤースパークライン＋3分計測 結果のキャラ/スキル推移用）。
/// 累積ダメージだと単調右肩上がりでバースト区間が判別できないため、区間DPS で起伏を見せる。
fn build_spark_commands(points: &[bpsr_core::models::TimeSeriesPoint]) -> String {
    build_spark_with(points, |p| p.total_dps)
}

/// PlayerRow → コピーテンプレ用データ（copy-list / 結果コピーで共用）。
fn copy_row_data(p: &bpsr_core::models::PlayerRow, rank: i32) -> format::CopyRowData<'_> {
    format::CopyRowData {
        rank,
        name: &p.name,
        class_name: &p.class_name,
        class_spec_name: &p.class_spec_name,
        total_value: p.total_value,
        value_per_sec: p.value_per_sec,
        value_pct: p.value_pct,
        crit_rate: p.crit_rate,
        crit_value_rate: p.crit_value_rate,
        lucky_rate: p.lucky_rate,
        lucky_value_rate: p.lucky_value_rate,
        hits: p.hits,
        hits_per_minute: p.hits_per_minute,
        ability_score: p.ability_score,
        season_level: p.season_level,
        season_strength: p.season_strength,
    }
}

/// スキル内訳 円グラフのパレット（スキル毎に色を変えて区別する）。
const SKILL_PIE_PALETTE: [u32; 10] = [
    0x4fc3f7, 0xff7043, 0x66bb6a, 0xffca28, 0xab47bc, 0x26c6da, 0xec407a, 0x9ccc65, 0xff8a65,
    0x7e57c2,
];

fn palette_color(i: usize) -> slint::Color {
    let hex = SKILL_PIE_PALETTE[i % SKILL_PIE_PALETTE.len()];
    slint::Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// 「その他」スライスの色（TOP10 以外の集約・グレー）。パレットと合わせて全11色。
const OTHER_SLICE_COLOR: u32 = 0x9e9e9e;

fn rgb(hex: u32) -> slint::Color {
    slint::Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// 着色済み (色, 値) リストから扇形＋灰色の内向き区切り線を生成。viewbox 100x100。
fn pie_slices(entries: &[(slint::Color, f64)]) -> Vec<PieSlice> {
    let sum: f64 = entries.iter().map(|e| e.1).sum();
    if sum <= 0.0 {
        return Vec::new();
    }
    let (cx, cy, r) = (50.0_f32, 50.0_f32, 48.0_f32);
    let mut out = Vec::new();
    // 扇形（塗り・stroke なし）
    let mut start = -std::f32::consts::FRAC_PI_2;
    for (color, v) in entries.iter() {
        let frac = (*v / sum) as f32;
        if frac <= 0.0 {
            continue;
        }
        if frac >= 0.999 {
            out.push(PieSlice {
                commands: format!(
                    "M {cx} {} A {r} {r} 0 1 1 {cx} {} A {r} {r} 0 1 1 {cx} {} Z",
                    cy - r,
                    cy + r,
                    cy - r
                )
                .into(),
                color: *color,
                is_line: false,
            });
            return out; // 単独100%は区切り線不要
        }
        let sweep = frac * std::f32::consts::TAU;
        let end = start + sweep;
        let x0 = cx + r * start.cos();
        let y0 = cy + r * start.sin();
        let x1 = cx + r * end.cos();
        let y1 = cy + r * end.sin();
        let large = if sweep > std::f32::consts::PI { 1 } else { 0 };
        out.push(PieSlice {
            commands: format!(
                "M {cx} {cy} L {x0:.2} {y0:.2} A {r} {r} 0 {large} 1 {x1:.2} {y1:.2} Z"
            )
            .into(),
            color: *color,
            is_line: false,
        });
        start = end;
    }
    // 区切り線（中心→外周手前 0.86r・灰色）。各スライス境界に1本。
    let r_out = r * 0.86;
    let line_color = rgb(0x8a8a8a);
    let mut a = -std::f32::consts::FRAC_PI_2;
    for (_, v) in entries.iter() {
        let frac = (*v / sum) as f32;
        if frac <= 0.0 {
            continue;
        }
        let lx = cx + r_out * a.cos();
        let ly = cy + r_out * a.sin();
        out.push(PieSlice {
            commands: format!("M {cx} {cy} L {lx:.2} {ly:.2}").into(),
            color: line_color,
            is_line: true,
        });
        a += frac * std::f32::consts::TAU;
    }
    out
}

/// 表示言語が日本語か。Rust 側で組み立てる動的文字列の ja/en 切替に使う
/// （ja 以外は en。zh/ko は保留中のため en にフォールバック）。
fn is_ja() -> bool {
    engine::runtime_settings::display_lang() == engine::runtime_settings::Lang::Ja
}

/// スキル内訳を TOP10（詳細名）＋「その他」(残り集約) の (表示名, 色, 値) へ集約。
/// 円グラフ・凡例で共用。skills は降順ソート済を前提。
fn top10_with_other(skills: &[bpsr_core::models::SkillRow]) -> Vec<(String, slint::Color, f64)> {
    let mut out: Vec<(String, slint::Color, f64)> = skills
        .iter()
        .take(10)
        .enumerate()
        .map(|(i, s)| (s.name.clone(), palette_color(i), s.total_value))
        .collect();
    let other: f64 = skills.iter().skip(10).map(|s| s.total_value).sum();
    if other > 0.0 {
        let other_label = if is_ja() { "その他" } else { "Other" };
        out.push((other_label.to_string(), rgb(OTHER_SLICE_COLOR), other));
    }
    out
}

/// 集約済みエントリ (表示名, 色, 値) から凡例行を生成（色・割合は円グラフと一致）。
fn legend_from(entries: &[(String, slint::Color, f64)]) -> Vec<SkillLegendUi> {
    let sum = entries.iter().map(|e| e.2).sum::<f64>().max(1.0);
    entries
        .iter()
        .map(|(name, color, v)| SkillLegendUi {
            name: name.clone().into(),
            color: *color,
            pct_text: format!("{:.1}%", v / sum * 100.0).into(),
        })
        .collect()
}

/// 結果パネルのプレイヤー行（uid・選択状態付き）。エリア1 は最大5キャラ。
fn build_result_rows(
    snap: &bpsr_core::models::EncounterSnapshot,
    selected_uid: i64,
    privacy: bool,
) -> Vec<ResultRowUi> {
    snap.player_rows
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, p)| {
            let uid = p.uid as i64;
            let name = if privacy {
                format::mask_player_name(uid)
            } else {
                p.name.clone()
            };
            ResultRowUi {
                rank_text: format!("{}.", i + 1).into(),
                rank: (i + 1) as i32,
                name: name.into(),
                class_color: format::class_color(&p.class_name),
                dps_text: format::format_dps(p.value_per_sec).into(),
                dmg_text: format::format_number(p.total_value).into(),
                pct_text: format::format_pct(p.value_pct).into(),
                pct: p.value_pct as f32,
                uid_str: format!("{uid}").into(),
                selected: uid == selected_uid,
            }
        })
        .collect()
}

/// 結果パネル エリア2 のスキル行（属性色・選択状態付き）。skills は降順ソート済。
fn build_result_skill_rows(
    skills: &[bpsr_core::models::SkillRow],
    selected_skill_uid: i64,
) -> Vec<ResultSkillRowUi> {
    skills
        .iter()
        .map(|s| {
            let (_, ec) = format::element_label(s.element);
            let uid = s.uid as i64;
            ResultSkillRowUi {
                uid_str: format!("{uid}").into(),
                name: s.name.clone().into(),
                elem_id: s.element as i32,
                elem_color: ec,
                dmg_text: format::format_number(s.total_value).into(),
                pct_text: format::format_pct(s.value_pct).into(),
                selected: uid == selected_skill_uid,
            }
        })
        .collect()
}

/// 区間DPS の折れ線を「時間軸」で配置（x = t_ms/duration）。結果画面の X軸時間ラベルと整合させる。
/// バーストの起伏は実時間位置で描く（index 等間隔だと全幅へ引き伸ばして誤解を招くため）。
/// ただし左端は 0:00 へ接地する: 初使用が 0:00 より後の系列（途中から使ったスキル/途中参戦
/// キャラ）は (x=0, dps=0) から初使用直前まで底辺の平坦線を引き、折れ線を必ず左端へ届かせる。
/// 右端は確定時の終端サンプル（compute::seal_3min_series）で計測末尾へ接地済み。
/// 座標は正規化(0..1。`SPARK_VB`)で、要素サイズへの引き伸ばしは .slint の `fit: fill` が行う。
fn build_spark_dps_time(
    points: &[bpsr_core::models::TimeSeriesPoint],
    duration_ms: f64,
) -> String {
    if points.len() < 2 {
        return String::new();
    }
    let max = points.iter().map(|p| p.total_dps).fold(1.0_f64, f64::max);
    let dur = duration_ms.max(1.0);
    let mut s = String::with_capacity((points.len() + 2) * 18);

    // 左端接地: 最初のサンプルが 0:00 より後（途中から使ったスキル/途中参戦キャラ）なら、
    // (x=0, dps=0) から初使用直前まで底辺の平坦線を引き、折れ線を必ず左端へ届かせる。
    // 未使用区間=0 の表現なので誤解はなく、右端は終端サンプルで既に接地している。
    // 閾値は「およそ0.5px相当」（幅1000px前後のプロットを想定）。これ未満のズレで接地線を
    // 引くと、左端に潰れた縦線が出るだけで情報が増えないため描かない。
    const GROUND_EPS: f32 = 0.0005 * SPARK_VB;
    let first_x = (points[0].t_ms / dur).clamp(0.0, 1.0) as f32 * SPARK_VB;
    let mut drawn = false;
    if first_x > GROUND_EPS {
        spark_point(&mut s, true, 0.0, SPARK_VB);
        spark_point(&mut s, false, first_x, SPARK_VB);
        drawn = true;
    }
    for p in points {
        let x = (p.t_ms / dur).clamp(0.0, 1.0) as f32 * SPARK_VB;
        let y = SPARK_VB - (p.total_dps / max) as f32 * SPARK_VB;
        spark_point(&mut s, !drawn, x, y);
        drawn = true;
    }
    s
}

/// 選択キャラの区間DPS折れ線（エリア1）。snap の player_rows[uid] の time_series から。
fn build_char_spark(snap: &bpsr_core::models::EncounterSnapshot, uid: i64) -> String {
    snap.player_rows
        .iter()
        .find(|p| p.uid as i64 == uid)
        .map(|p| build_spark_dps_time(&p.time_series, snap.duration_ms))
        .unwrap_or_default()
}

/// 選択スキルの区間DPS折れ線（エリア4）。duration は計測全体（snap.duration_ms）で統一。
fn build_skill_spark(
    skills: &[bpsr_core::models::SkillRow],
    selected_skill_uid: i64,
    duration_ms: f64,
) -> String {
    skills
        .iter()
        .find(|s| s.uid as i64 == selected_skill_uid)
        .map(|s| build_spark_dps_time(&s.time_series, duration_ms))
        .unwrap_or_default()
}

/// 折れ線の Y軸目安ラベル (上=最大DPS, 中=その半分)。下端は UI 側で常に "0"。
fn spark_axis_labels(points: &[bpsr_core::models::TimeSeriesPoint]) -> (String, String) {
    let max = points.iter().map(|p| p.total_dps).fold(0.0_f64, f64::max);
    (format::format_number(max), format::format_number(max / 2.0))
}

fn char_axis_labels(snap: &bpsr_core::models::EncounterSnapshot, uid: i64) -> (String, String) {
    snap.player_rows
        .iter()
        .find(|p| p.uid as i64 == uid)
        .map(|p| spark_axis_labels(&p.time_series))
        .unwrap_or_default()
}

fn skill_axis_labels(
    skills: &[bpsr_core::models::SkillRow],
    selected_skill_uid: i64,
) -> (String, String) {
    skills
        .iter()
        .find(|s| s.uid as i64 == selected_skill_uid)
        .map(|s| spark_axis_labels(&s.time_series))
        .unwrap_or_default()
}

/// 選択スキルに応じて スキル行ハイライト・スキル折れ線・ラベル のみ更新（円グラフ/凡例は据置）。
fn apply_result_skill_selection(
    m: &MainWindow,
    skills: &[bpsr_core::models::SkillRow],
    selected_skill_uid: i64,
    result_skill_rows: &slint::VecModel<ResultSkillRowUi>,
    duration_ms: f64,
) {
    result_skill_rows.set_vec(build_result_skill_rows(skills, selected_skill_uid));
    let spark = build_skill_spark(skills, selected_skill_uid, duration_ms);
    m.set_result_skill_spark_visible(!spark.is_empty());
    m.set_result_skill_spark(spark.into());
    let (stop, smid) = skill_axis_labels(skills, selected_skill_uid);
    m.set_result_skill_axis_top(stop.into());
    m.set_result_skill_axis_mid(smid.into());
    // エリア4 折れ線ラベル: 選択スキル名
    let skill_name = skills
        .iter()
        .find(|s| s.uid as i64 == selected_skill_uid)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    m.set_result_skill_name(skill_name.into());
}

/// 選択プレイヤーに応じて 行ハイライト・キャラ折れ線・タイトル・スキル行/折れ線・円グラフ/凡例 を更新。
/// 既定スキルは内訳の先頭（最大）を選択する。
#[allow(clippy::too_many_arguments)]
fn apply_result_selection(
    m: &MainWindow,
    uid: i64,
    snap: &bpsr_core::models::EncounterSnapshot,
    captured: &std::collections::HashMap<i64, Vec<bpsr_core::models::SkillRow>>,
    result_rows: &slint::VecModel<ResultRowUi>,
    result_skill_rows: &slint::VecModel<ResultSkillRowUi>,
    result_pie: &slint::VecModel<PieSlice>,
    result_legend: &slint::VecModel<SkillLegendUi>,
    selected_player: &std::cell::Cell<i64>,
    selected_skill: &std::cell::Cell<i64>,
    privacy: bool,
) {
    selected_player.set(uid);
    result_rows.set_vec(build_result_rows(snap, uid, privacy));
    // エリア1: 選択キャラの区間DPS折れ線
    let char_spark = build_char_spark(snap, uid);
    m.set_result_char_spark_visible(!char_spark.is_empty());
    m.set_result_char_spark(char_spark.into());
    let (ctop, cmid) = char_axis_labels(snap, uid);
    m.set_result_char_axis_top(ctop.into());
    m.set_result_char_axis_mid(cmid.into());
    // タイトル
    let pname = snap
        .player_rows
        .iter()
        .find(|p| p.uid as i64 == uid)
        .map(|p| {
            if privacy {
                format::mask_player_name(uid)
            } else {
                p.name.clone()
            }
        })
        .unwrap_or_default();
    let pie_title = if is_ja() {
        format!("{pname} のスキル内訳")
    } else {
        format!("{pname} — Skill Breakdown")
    };
    m.set_result_pie_title(pie_title.into());
    m.set_result_char_name(pname.clone().into()); // エリア1 折れ線ラベル
    // エリア2/3: スキル内訳。既定選択=先頭(最大)。
    let empty = Vec::new();
    let skills = captured.get(&uid).unwrap_or(&empty);
    let default_skill = skills.first().map(|s| s.uid as i64).unwrap_or(0);
    selected_skill.set(default_skill);
    apply_result_skill_selection(m, skills, default_skill, result_skill_rows, snap.duration_ms);
    // エリア3: TOP10＋その他 の円グラフ＋凡例
    let entries = top10_with_other(skills);
    let colored: Vec<(slint::Color, f64)> = entries.iter().map(|(_, c, v)| (*c, *v)).collect();
    result_pie.set_vec(pie_slices(&colored));
    result_legend.set_vec(legend_from(&entries));
}

/// 3分計測 結果パネルへスナップショットを反映して開く。
#[allow(clippy::too_many_arguments)]
fn show_result(
    m: &MainWindow,
    snap: &bpsr_core::models::EncounterSnapshot,
    captured: &std::collections::HashMap<i64, Vec<bpsr_core::models::SkillRow>>,
    default_uid: i64,
    result_rows: &slint::VecModel<ResultRowUi>,
    result_skill_rows: &slint::VecModel<ResultSkillRowUi>,
    result_pie: &slint::VecModel<PieSlice>,
    result_legend: &slint::VecModel<SkillLegendUi>,
    selected_player: &std::cell::Cell<i64>,
    selected_skill: &std::cell::Cell<i64>,
    privacy: bool,
) {
    m.set_result_dps(format::format_dps(snap.total_dps).into());
    m.set_result_dmg(format::format_number(snap.total_dmg).into());
    m.set_result_duration(format::format_elapsed(snap.duration_ms).into());
    m.set_result_duration_ms(snap.duration_ms as f32); // X軸 時間ラベル用
    apply_result_selection(
        m,
        default_uid,
        snap,
        captured,
        result_rows,
        result_skill_rows,
        result_pie,
        result_legend,
        selected_player,
        selected_skill,
        privacy,
    );
    m.set_result_open(true);
}

/// カウントアップ演出の進捗(0..1)をイージング(ease-out-cubic)する。
fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

const RESULT_COUNTUP_MS: u64 = 500;
const RESULT_COUNTUP_TICK_MS: u64 = 16;

/// 結果パネルの DPS/総ダメを 0 から最終値へ ease-out-cubic でカウントアップする（新規計測確定時のみ）。
/// `show_result` が直後に最終値を即時セットしているため、この呼び出しは同一tick内でそれを
/// 0 へ上書きしてからアニメーションを開始する（再描画前に上書きされるため点滅しない）。
/// timer はこの用途専有の Rc<Timer>（呼び出し側が持ち回して寿命を保つ）。完了で必ず stop する。
/// final_store は進行中の最終値を共有するための Rc<Cell>（画像コピー直前に即完了させる用途）。
fn start_result_countup(
    m: &MainWindow,
    timer: &Rc<Timer>,
    final_store: &Rc<Cell<(f64, f64)>>,
    final_dps: f64,
    final_dmg: f64,
) {
    final_store.set((final_dps, final_dmg));
    m.set_result_dps(format::format_dps(0.0).into());
    m.set_result_dmg(format::format_number(0.0).into());
    let start = std::time::Instant::now();
    let w = m.as_weak();
    let timer_stop = timer.clone();
    timer.start(
        TimerMode::Repeated,
        Duration::from_millis(RESULT_COUNTUP_TICK_MS),
        move || {
            let Some(m) = w.upgrade() else {
                timer_stop.stop();
                return;
            };
            let t = start.elapsed().as_secs_f32() * 1000.0 / RESULT_COUNTUP_MS as f32;
            let eased = ease_out_cubic(t) as f64;
            if t >= 1.0 {
                m.set_result_dps(format::format_dps(final_dps).into());
                m.set_result_dmg(format::format_number(final_dmg).into());
                timer_stop.stop();
                return;
            }
            m.set_result_dps(format::format_dps(final_dps * eased).into());
            m.set_result_dmg(format::format_number(final_dmg * eased).into());
        },
    );
}

/// カウントアップ演出を即座に完了させる（最終値を確定表示してタイマーを止める）。
/// 共有画像コピーなど、表示中の値をそのままレンダリングへ焼き込む操作の直前に呼ぶ
/// （呼ばなければ計測確定から500ms以内のコピーでカウントアップ途中の小さい値が写り込む）。
/// カウントアップが既に完了済み/未開始でも副作用は無い(同じ最終値を再セット・stopは無害)。
fn finish_result_countup(m: &MainWindow, timer: &Rc<Timer>, final_store: &Rc<Cell<(f64, f64)>>) {
    let (final_dps, final_dmg) = final_store.get();
    m.set_result_dps(format::format_dps(final_dps).into());
    m.set_result_dmg(format::format_number(final_dmg).into());
    timer.stop();
}

/// 計測の絞り込み条件を短いラベルにする。条件なしは空文字。
/// 履歴の見出し行と結果画面の透かしが同じ条件表現を共有するための単一定義
/// （片方だけ表記を足すと、同じ計測が画面によって別条件に見える）。
fn measure_scope_label(scope: bpsr_core::engine::encounter::MeasureScope) -> String {
    let ja = is_ja();
    let mut parts: Vec<&str> = Vec::new();
    if scope.self_only {
        parts.push(if ja { "自分のみ" } else { "self only" });
    }
    if scope.first_target_only {
        parts.push(if ja { "初撃の敵のみ" } else { "first target only" });
    }
    parts.join(if ja { "・" } else { ", " })
}

/// シェア画像の透かし（アクション行左）: "bpsr-checker vX.X.X ・ YYYY-MM-DD HH:MM ・ 計測 M:SS"。
/// 日時は計測終了時点のローカル時刻。
///
/// 絞り込み条件（自分のみ / 最初の敵のみ）が効いていた計測は、条件を末尾へ足す。
/// 条件付きの数値と通常の数値は直接比較できないため、スクリーンショットだけを見た人が
/// 同じ土俵の記録だと誤解しないようにする。
fn build_result_watermark(
    app_version: &str,
    snap: &bpsr_core::models::EncounterSnapshot,
) -> String {
    let scope = snap.measure_scope;
    let now = chrono::Local::now().format(DATETIME_DISPLAY_FORMAT);
    let dur = format::format_elapsed(snap.duration_ms);
    let ja = is_ja();
    let mut base = if ja {
        format!("{app_version} ・ {now} ・ 計測 {dur}")
    } else {
        format!("{app_version} ・ {now} ・ {dur} measured")
    };
    let conditions = measure_scope_label(scope);
    if !conditions.is_empty() {
        base.push_str(" ・ ");
        base.push_str(&conditions);
    }
    base
}

/// 計測確定時の自己ベスト判定・更新。auto-open 設定(モーダルを開くか)に依らず、finalize の
/// 都度これを呼ぶ想定＝auto-open OFF でも記録は静かに積み上がる。自キャラ(local_uid)が
/// そのエンカウントに不在なら記録せず None を返す。デモモードでは保存しない(persist=false)
/// がメモリ上の比較・判定は行う。戻り値は (新記録か, 自己ベストDPS)。
fn record_best(
    best: &Rc<RefCell<best_records::BestRecords>>,
    snap: &bpsr_core::models::EncounterSnapshot,
    local_uid: i64,
    persist: bool,
) -> Option<(bool, f64)> {
    let self_row = snap.player_rows.iter().find(|p| p.uid as i64 == local_uid)?;
    // 条件はスナップショットが運ぶ。finalize の前に別途採る必要は無い。
    let scope = snap.measure_scope;
    let duration_sec = (snap.duration_ms / 1000.0).round().max(0.0) as u32;
    let recorded_at_ms = chrono::Utc::now().timestamp_millis();
    let mut recs = best.borrow_mut();
    let is_new = recs.try_update(
        duration_sec,
        scope,
        self_row.value_per_sec,
        self_row.total_value,
        recorded_at_ms,
    );
    if is_new && persist {
        recs.save();
    }
    let best_dps = recs
        .get(duration_sec, scope)
        .map(|r| r.dps)
        .unwrap_or(self_row.value_per_sec);
    Some((is_new, best_dps))
}

/// `record_best` の結果を結果モーダルの表示プロパティ（新記録バッジ/自己ベスト併記）へ反映する。
/// モーダルを実際に開く(auto-open)ときだけ呼ぶ。
fn apply_result_best_record_ui(m: &MainWindow, outcome: Option<(bool, f64)>) {
    match outcome {
        Some((is_new, best_dps)) => {
            m.set_result_is_new_record(is_new);
            m.set_result_best_dps_text(format::format_dps(best_dps).into());
        }
        None => {
            m.set_result_is_new_record(false);
            m.set_result_best_dps_text("".into());
        }
    }
}

/// uid の下4桁（候補ラベル用。元 UI の String(uid).slice(-4) 相当）。
fn last4(uid: i64) -> String {
    let s = uid.to_string();
    s[s.len().saturating_sub(4)..].to_string()
}

/// 自キャラUID 候補（現在の DPS プレイヤー）を再構築。selected も反映。
/// 入力欄(selected-uid-value)は触らない＝設定パネルを開いたまま poll で呼んでも
/// 入力中をクロバーしない。
fn refresh_uid_candidates(
    enc: &EncounterMutex,
    candidates: &slint::VecModel<UidCandidate>,
) {
    let sel = compute::get_selected_uid().map(|v| v as i64);
    let pw = compute::get_dps_players(enc);
    let cands: Vec<UidCandidate> = pw
        .player_rows
        .iter()
        .take(12)
        .map(|p| {
            let uid = p.uid as i64;
            UidCandidate {
                uid_str: format!("{uid}").into(),
                label: format!("{} #{}", p.name, last4(uid)).into(),
                selected: Some(uid) == sel,
            }
        })
        .collect();
    candidates.set_vec(cands);
}

/// 入力欄・解決名・候補をまとめて更新（パネル開時／確定時のみ。入力欄を push する）。
fn refresh_selected_uid(
    m: &MainWindow,
    enc: &EncounterMutex,
    candidates: &slint::VecModel<UidCandidate>,
) {
    let sel = compute::get_selected_uid();
    let sel_i64 = sel.map(|v| v as i64);
    m.set_selected_uid_value(
        sel_i64
            .map(|u| u.to_string())
            .unwrap_or_default()
            .into(),
    );
    let name = sel.and_then(compute::lookup_name_cache).map(|d| d.name);
    // 対象クライアント未特定の間は表示が空になる。「戦闘していない」と区別できるよう明示する
    // （特定は自プレイヤー専用デルタの受信で自動的に完了する）。
    let resolved = compute::selected_conn_resolved(enc);
    m.set_selected_uid_name(match (name, sel_i64) {
        (_, Some(_)) if !resolved => if is_ja() {
            "（対象クライアント特定中…）"
        } else {
            "(identifying target client...)"
        }
        .into(),
        (Some(n), _) => n.into(),
        (None, Some(_)) => if is_ja() { "（名前未解決）" } else { "(name unresolved)" }.into(),
        (None, None) => if is_ja() { "（未設定）" } else { "(none)" }.into(),
    });
    refresh_uid_candidates(enc, candidates);
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
    // 名前列テンプレートで {imagine}/{roleSkill} が消されていても、見出しは装備中イマジンと
    // ロールスキルを強制表示する（compute 側で別フィールドに分けているため両方を連結する）。
    m.set_inspected_name(
        format!(
            "{}{}{}",
            sw.inspected_player.name,
            sw.inspected_player.imagine_suffix,
            sw.inspected_player.role_skill_suffix
        )
        .into(),
    );
    sk_rows.set_vec(build_skill_rows(sw));
    m.set_skills_clickable(clickable);
    m.set_view(1);
}

/// Settings → build_rows へ渡すバー表示方式の設定一式（呼び出し4箇所の重複を避ける）。
/// タブ非依存（PlayerRow.time_series は build_rows 呼び出し元が現在タブと一致する指標
/// （与ダメ/回復/被ダメ）で取得済みのため、固定基準モードの窓平均も全タブで使える）。
fn dps_bar_config(c: &settings::Settings) -> dps_bar::DpsBarConfig {
    dps_bar::DpsBarConfig {
        mode: dps_bar::DpsBarMode::parse(&c.dps_bar_mode),
        fixed_max: c.dps_bar_fixed_max,
        window_secs: c.dps_bar_window_secs,
    }
}

/// アクセントカラー設定 → (accent, accent-strong)（0xAARRGGBB）。
/// "#rrggbb" 指定時は彩度を上げ明度を落とした派生色を accent-strong（塗り）に用いる。
/// プリセット名（旧設定）も後方互換で受け付け、未知値は既定 sky にフォールバックする。
fn accent_colors(theme: &str) -> (u32, u32) {
    if let Some(hex) = theme.strip_prefix('#') {
        if hex.len() == 6 {
            if let Ok(val) = u32::from_str_radix(hex, 16) {
                let (r, g, b) = (
                    ((val >> 16) & 0xff) as u8,
                    ((val >> 8) & 0xff) as u8,
                    (val & 0xff) as u8,
                );
                let (h, s, v) = rgb_to_hsv(r, g, b);
                let (sr, sg, sb) = hsv_to_rgb_u8(h, (s * 1.2).min(1.0), v * 0.62);
                let accent = 0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
                let strong = 0xff00_0000 | ((sr as u32) << 16) | ((sg as u32) << 8) | sb as u32;
                return (accent, strong);
            }
        }
    }
    match theme {
        "emerald" => (0xff34d399, 0xff0f9d6e),
        "amber" => (0xfffbc02d, 0xffb07a1e),
        "rose" => (0xfffb7185, 0xffc23a5c),
        "violet" => (0xffa78bfa, 0xff6d4fd9),
        "mono" => (0xffcfd2dc, 0xff5a5f6e),
        _ => (0xff4fc3f7, 0xff2d6cdf), // sky（既定）
    }
}

/// 設定の表示系を UI へ反映（列フラグ・自分強調・最前面・パネルのトグル状態）。
fn apply_settings(m: &MainWindow, c: &settings::Settings) {
    let (accent, accent_strong) = accent_colors(&c.accent_theme);
    let theme = m.global::<Theme>();
    theme.set_accent(slint::Color::from_argb_encoded(accent));
    theme.set_accent_strong(slint::Color::from_argb_encoded(accent_strong));
    // アクセント色HSVピッカーの位置（accent 色を HSV へ変換して反映）。
    {
        let (ar, ag, ab) = (
            ((accent >> 16) & 0xff) as u8,
            ((accent >> 8) & 0xff) as u8,
            (accent & 0xff) as u8,
        );
        let (ah, asat, av) = rgb_to_hsv(ar, ag, ab);
        m.set_accent_h(ah);
        m.set_accent_s(asat);
        m.set_accent_v(av);
    }
    m.set_cols(ColumnFlags {
        crit: c.show_crit,
        crit_value: c.show_crit_value,
        lucky: c.show_lucky,
        lucky_value: c.show_lucky_value,
        hits: c.show_hits,
        hpm: c.show_hpm,
        score: c.show_score,
        eff_dps: c.show_eff_dps,
    });
    m.set_highlight_local(c.highlight_local_player);
    m.set_aot(c.always_on_top);
    m.set_win_opacity(c.opacity as f32);
    m.set_overlay_opacity(c.overlay_opacity as f32);
    m.set_font_scale((c.font_size / 12.0) as f32);
    // 本体フォント（バフ/デバフ オーバーレイもこのファミリ・太字・サイズに追随する）。
    m.set_main_font(c.main_font.clone().into());
    m.set_main_font_bold(c.main_font_bold);
    m.set_show_consumable(c.show_consumable);
    m.set_cfg_ui(SettingsUi {
        show_crit: c.show_crit,
        show_crit_value: c.show_crit_value,
        show_lucky: c.show_lucky,
        show_lucky_value: c.show_lucky_value,
        show_hits: c.show_hits,
        show_hpm: c.show_hpm,
        show_score: c.show_score,
        show_eff_dps: c.show_eff_dps,
        highlight_local: c.highlight_local_player,
        abbreviate_scores: c.abbreviate_scores,
        privacy_mask: c.privacy_mask_names,
        self_status: c.show_self_status_overlay,
        stats_overlay: c.show_stats_overlay,
        buff_overlay: c.show_buff_overlay,
        imagine_only: c.imagine_only_mode,
        aot: c.always_on_top,
        three_min_auto_open: c.three_min_auto_open,
        compact_split: c.compact_split_mode,
        graph_for_local: c.graph_for_local_player,
        allow_solo_hotkeys: c.allow_solo_hotkeys,
        startup_tab: c.startup_tab.clone().into(),
        language: c.language.clone().into(),
        accent_theme: c.accent_theme.clone().into(),
        sync_timer: c.sync_timer_with_main,
        sync_order_follow: c.sync_order_follow,
        show_imagine_tina: c.show_imagine_tina,
        show_imagine_aluna: c.show_imagine_aluna,
        show_imagine_tarta: c.show_imagine_tarta,
        show_imagine_basilisk: c.show_imagine_basilisk,
        show_imagine_kartgriff: c.show_imagine_kartgriff,
        show_consumable: c.show_consumable,
        party_only_consumables: c.party_only_consumables,
        measure_self_only: c.measure_self_only,
        measure_first_target_only: c.measure_first_target_only,
        show_in_taskbar: c.show_in_taskbar,
        overlay_text_color: c.overlay_text_color.clone().into(),
        main_font: c.main_font.clone().into(),
        main_font_bold: c.main_font_bold,
        stats_overlay_font: c.stats_overlay_font.clone().into(),
        stats_overlay_font_bold: c.stats_overlay_font_bold,
        imagine_overlay_font: c.imagine_overlay_font.clone().into(),
        imagine_overlay_font_bold: c.imagine_overlay_font_bold,
        buff_overlay_font: c.buff_overlay_font.clone().into(),
        buff_overlay_font_bold: c.buff_overlay_font_bold,
        imagine_compact_rows: c.imagine_compact_rows,
        overlay_outline: c.overlay_outline,
        overlay_shadow: c.overlay_shadow,
        show_footer: c.show_footer,
        show_total_row: c.show_total_row,
        dps_bar_mode: c.dps_bar_mode.clone().into(),
        dps_bar_intensity: c.dps_bar_intensity.clone().into(),
        dps_bar_animate: c.dps_bar_animate,
        check_update_on_startup: c.check_update_on_startup,
    });
    // 文字色HSVピッカーの初期/同期位置（現在の文字色を HSV へ変換して反映）。
    {
        let col = resolve_overlay_text_color(&c.overlay_text_color);
        let (th, ts, tv) = rgb_to_hsv(col.red(), col.green(), col.blue());
        m.set_overlay_text_h(th);
        m.set_overlay_text_s(ts);
        m.set_overlay_text_v(tv);
    }
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
        stats_overlay_font_size: int_str(c.stats_overlay_font_size),
        imagine_overlay_font_size: int_str(c.imagine_overlay_font_size),
        buff_overlay_font_size: int_str(c.buff_overlay_font_size),
    });
}

/// グローバルショートカットの現在値（保存文字列＋登録エラー）を UI へ反映する。
/// 呼び出し箇所（apply() 直後・設定パネルを開いた時・起動時）で表示ロジックが分岐しない
/// よう、ここに一本化する。hk が None（マネージャ未生成/生成失敗）のときは全行のエラーを空にする
/// （その場合は apply() が全アクションへ初期化失敗のエラーを積んでいるはずなので通常は素通りしない）。
#[cfg(windows)]
fn push_shortcuts_to_ui(model: &VecModel<ShortcutUi>, hk: &Option<hotkey::Hotkeys>, c: &settings::Settings) {
    let rows: Vec<ShortcutUi> = hotkey::ACTIONS
        .iter()
        .map(|&action| ShortcutUi {
            key_text: action.key_text(c).to_string().into(),
            error: hk.as_ref().map(|h| h.error_for(action)).unwrap_or("").into(),
        })
        .collect();
    model.set_vec(rows);
}

/// オーバーレイ文字色文字列を実際の色へ解決する。
/// プリセットキー（white/warm/cool/green/amber）または "#rrggbb" 形式を受け付ける。
fn resolve_overlay_text_color(s: &str) -> slint::Color {
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            if let Ok(v) = u32::from_str_radix(hex, 16) {
                return slint::Color::from_rgb_u8(
                    ((v >> 16) & 0xff) as u8,
                    ((v >> 8) & 0xff) as u8,
                    (v & 0xff) as u8,
                );
            }
        }
    }
    let (r, g, b) = match s {
        "warm" => (0xff, 0xe9, 0xcf),
        "cool" => (0xcf, 0xe6, 0xff),
        "green" => (0xcd, 0xfb, 0xd8),
        "amber" => (0xff, 0xe7, 0xa8),
        _ => (0xff, 0xff, 0xff), // white（既定）
    };
    slint::Color::from_rgb_u8(r, g, b)
}

/// RGB(各 u8) → HSV（h/s/v とも 0..1）。h は色相を 0..1 に正規化（×360 で度）。
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (rf, gf, bf) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let d = max - min;
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { d / max };
    let h = if d <= 0.0 {
        0.0
    } else if (max - rf).abs() < f32::EPSILON {
        (((gf - bf) / d).rem_euclid(6.0)) / 6.0
    } else if (max - gf).abs() < f32::EPSILON {
        (((bf - rf) / d) + 2.0) / 6.0
    } else {
        (((rf - gf) / d) + 4.0) / 6.0
    };
    (h.rem_euclid(1.0), s, v)
}

/// HSV（各 0..1）→ RGB(各 u8)。
fn hsv_to_rgb_u8(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor() as i32;
    let f = h6 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let (rf, gf, bf) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let to_u8 = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(rf), to_u8(gf), to_u8(bf))
}

/// HSV（各 0..1）→ "#rrggbb"。
fn hsv_to_hex(h: f32, s: f32, v: f32) -> String {
    let (r, g, b) = hsv_to_rgb_u8(h, s, v);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// オーバーレイ3窓（バフ/デバフ・ステータス・イマジンタイマー）の外観を反映する。
/// 不透明度・基準テキスト色は3窓共通。文字サイズ・フォント・太字は窓ごとに独立した専用設定を使う。
/// 表示/非表示に関わらずプロパティは窓側に保持されるため、起動時と設定変更時のみ呼べばよい。
fn apply_overlay_appearance(
    c: &settings::Settings,
    self_o: &slint::Weak<SelfStatusOverlay>,
    buff_o: &slint::Weak<BuffOverlay>,
    stats_o: &slint::Weak<StatsOverlay>,
) {
    let op = c.overlay_opacity as f32;
    let text_col = resolve_overlay_text_color(&c.overlay_text_color);
    // 完全透明（不透明度0）のときは当該オーバーレイをクリック透過（HUD化）にし、
    // ゲーム側へクリックを通す＝オーバーレイ自体は移動/最小化/×できなくなる。
    // 不透明度を0より上げると hittest が戻り操作可能に復帰する。
    // （トレイの「クリックスルー」一括切替とは別系統。不透明度変更時に本値で上書きする。）
    let pass_through = op <= 0.001;
    // バフ/デバフ窓: 専用フォント設定（メインとは独立）。
    if let Some(o) = self_o.upgrade() {
        o.set_overlay_opacity(op);
        o.set_overlay_outline(c.overlay_outline);
        o.set_overlay_shadow(c.overlay_shadow);
        o.set_overlay_font(c.buff_overlay_font.clone().into());
        o.set_overlay_font_bold(c.buff_overlay_font_bold);
        o.set_text_base(text_col);
        o.set_font_scale((c.buff_overlay_font_size / 12.0) as f32);
        overlay::set_click_through(o.window(), pass_through);
    }
    // イマジンタイマー窓: 専用フォント設定。
    if let Some(o) = buff_o.upgrade() {
        o.set_overlay_opacity(op);
        o.set_overlay_outline(c.overlay_outline);
        o.set_overlay_shadow(c.overlay_shadow);
        o.set_overlay_font(c.imagine_overlay_font.clone().into());
        o.set_overlay_font_bold(c.imagine_overlay_font_bold);
        o.set_text_base(text_col);
        o.set_font_scale((c.imagine_overlay_font_size / 12.0) as f32);
        overlay::set_click_through(o.window(), pass_through);
    }
    // ステータス窓: 専用フォント設定。
    if let Some(o) = stats_o.upgrade() {
        o.set_overlay_opacity(op);
        o.set_overlay_outline(c.overlay_outline);
        o.set_overlay_shadow(c.overlay_shadow);
        o.set_overlay_font(c.stats_overlay_font.clone().into());
        o.set_overlay_font_bold(c.stats_overlay_font_bold);
        o.set_text_base(text_col);
        o.set_font_scale((c.stats_overlay_font_size / 12.0) as f32);
        overlay::set_click_through(o.window(), pass_through);
    }
}

/// テンプレートのプレビュー（固定サンプル行で name/copy 両テンプレを展開）。
fn template_previews(c: &settings::Settings) -> (slint::SharedString, slint::SharedString) {
    // プレビューのサンプル職業/特化名も表示言語に揃える。
    let (sample_class, sample_spec) = if is_ja() {
        ("ストームブレイド", "雷刃型")
    } else {
        ("Stormblade", "Iaido")
    };
    let name = format::format_row_name(
        "Sample",
        sample_class,
        sample_spec,
        12345.0,
        38.0,
        8200.0,
        "-タータ/アルーナ",
        " (R:ファルファラ)",
        1,
        &c.name_template,
        c.abbreviate_scores,
    );
    let copy = format::format_row_template(
        &format::CopyRowData {
            rank: 1,
            name: "Sample",
            class_name: sample_class,
            class_spec_name: sample_spec,
            total_value: 1_234_567.0,
            value_per_sec: 45678.0,
            value_pct: 35.5,
            crit_rate: 42.3,
            crit_value_rate: 18.7,
            lucky_rate: 5.5,
            lucky_value_rate: 2.1,
            hits: 124.0,
            hits_per_minute: 78.5,
            ability_score: 12345.0,
            season_level: 38.0,
            season_strength: 8200.0,
        },
        &c.copy_template,
        c.abbreviate_scores,
    );
    (name.into(), copy.into())
}

/// テンプレ入力欄・バー表示方式の数値入力欄の value を push（パネル開時／リセット時のみ）
/// ＋プレビュー更新。編集中に呼ぶと入力中の文字をクロバーするため、呼び出し箇所を限定する。
fn refresh_settings_inputs(m: &MainWindow, c: &settings::Settings) {
    m.set_name_template_value(c.name_template.clone().into());
    m.set_copy_template_value(c.copy_template.clone().into());
    push_dps_bar_inputs(m, c);
    let (np, cp) = template_previews(c);
    m.set_name_preview(np);
    m.set_copy_preview(cp);
}

/// バー表示方式の数値入力欄の value のみ push（クランプ・拒否後の実効値を表示へ同期し直す）。
/// 確定（Enter／フォーカスアウト）時に単独で呼ぶため、テンプレ側は触らない。
fn push_dps_bar_inputs(m: &MainWindow, c: &settings::Settings) {
    m.set_dps_bar_fixed_max_value(dps_bar::format_bar_num(c.dps_bar_fixed_max).into());
    m.set_dps_bar_window_secs_value(dps_bar::format_bar_num(c.dps_bar_window_secs).into());
}

/// バトルイマジン名メンテ一覧の1行を構築する（filter は小文字化して部分一致・非空時のみ絞り込む）。
/// スキル名は表示言語追従（skill_names::get_skill_name）。expanded はUIローカル状態（main.rs 側の
/// HashSet）を反映するだけで、モデル自体には永続しない。
fn build_imagine_rows(
    expanded: &std::collections::HashSet<String>,
    filter: &str,
) -> Vec<ImagineNameRowUi> {
    let filter_lower = filter.trim().to_lowercase();
    engine::imagine_skills::imagine_entries()
        .into_iter()
        .filter_map(|entry| {
            let main_skill = engine::skill_names::get_skill_name(entry.main_skill_id);
            let clone_skills: Vec<String> = entry
                .clone_skill_ids
                .iter()
                .map(|id| engine::skill_names::get_skill_name(*id))
                .collect();
            let ov = engine::imagine_overrides::get(&entry.canonical);
            let display_name = ov
                .as_ref()
                .and_then(|o| o.display_name.clone())
                .unwrap_or_default();
            let ignored = ov.as_ref().is_some_and(|o| o.ignored);

            if !filter_lower.is_empty() {
                let haystack = format!(
                    "{} {} {} {}",
                    entry.canonical,
                    main_skill,
                    clone_skills.join(" "),
                    display_name
                )
                .to_lowercase();
                if !haystack.contains(&filter_lower) {
                    return None;
                }
            }

            let has_clones = !clone_skills.is_empty();
            let is_expanded = expanded.contains(&entry.canonical);
            Some(ImagineNameRowUi {
                canonical: entry.canonical.clone().into(),
                imagine_name: entry.canonical.into(),
                main_skill: main_skill.into(),
                display_name: display_name.into(),
                ignored,
                has_clones,
                expanded: is_expanded,
                clone_skills: slint::ModelRc::new(VecModel::from(
                    clone_skills
                        .into_iter()
                        .map(slint::SharedString::from)
                        .collect::<Vec<_>>(),
                )),
            })
        })
        .collect()
}

/// 行数が同じなら in-place 更新（sync_rows と同じ理由＝編集中のTextInputのフォーカス/カーソル保持）。
fn sync_imagine_rows(model: &slint::VecModel<ImagineNameRowUi>, data: Vec<ImagineNameRowUi>) {
    if model.row_count() == data.len() {
        for (i, r) in data.into_iter().enumerate() {
            model.set_row_data(i, r);
        }
    } else {
        model.set_vec(data);
    }
}

/// イマジン名一覧モデルを再構築する。呼ぶのは設定パネルopen時と、展開/IGNORE/リセット/フィルタ/
/// devリネームなどの「非タイプ操作」のみ。表示名の TextInput 編集（`edited` 毎）では呼ばない
/// （TemplateField と同じ方針＝編集中に text: バインドを再送してクロバーしないため。詳細は
/// ImagineNamesSection の doc コメント参照）。
fn rebuild_imagine_rows(
    model: &slint::VecModel<ImagineNameRowUi>,
    expanded: &std::collections::HashSet<String>,
    filter: &str,
) {
    sync_imagine_rows(model, build_imagine_rows(expanded, filter));
}

/// devモード「GitHubへ反映」: イマジン名DBをワークツリーへ書き戻し→ git add/commit/push。
/// 他の未コミット変更を巻き込まないよう、add 対象は ImagineSkillNames.json のみに絞る。
/// commit で差分無し（nothing to commit）は成功として扱い push まで進める。各段の失敗は
/// 握り潰さず要点（stderr優先・無ければstdout）をステータス文字列として返す。
fn push_imagine_db() -> String {
    if let Err(e) = engine::imagine_skills::dev_save_to_working_tree() {
        log::warn!("イマジン名DB書き戻しに失敗: {e}");
        return if is_ja() {
            format!("書き戻しに失敗: {e}")
        } else {
            format!("Failed to write DB: {e}")
        };
    }
    // CARGO_MANIFEST_DIR は slint-app（bpsr-app crate）なので、親＝ワークスペースroot。
    let Some(workspace_root) = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent() else {
        return if is_ja() {
            "ワークツリーの検出に失敗".to_string()
        } else {
            "Failed to locate working tree".to_string()
        };
    };
    const REL_PATH: &str = "core/data/json/ImagineSkillNames.json";

    let run_git = |args: &[&str]| -> Result<(), String> {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(workspace_root)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !stderr.is_empty() {
            return Err(stderr);
        }
        // 「nothing to commit」等は stdout 側に出るため、stderr が空ならこちらを使う。
        Err(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };

    if let Err(e) = run_git(&["add", REL_PATH]) {
        log::warn!("git add に失敗: {e}");
        return if is_ja() {
            format!("git add に失敗: {e}")
        } else {
            format!("git add failed: {e}")
        };
    }
    // commit も pathspec で対象を限定する（pathspec無しはインデックス全体をコミットするため、
    // dev が別途 git add 済みの他ファイルが git add 一発では絞れていても巻き込まれてしまう）。
    if let Err(e) = run_git(&["commit", "-m", "chore: イマジン名DB更新", "--", REL_PATH]) {
        if !e.contains("nothing to commit") {
            log::warn!("git commit に失敗: {e}");
            return if is_ja() {
                format!("git commit に失敗: {e}")
            } else {
                format!("git commit failed: {e}")
            };
        }
    }
    if let Err(e) = run_git(&["push"]) {
        log::warn!("git push に失敗: {e}");
        return if is_ja() {
            format!("git push に失敗: {e}")
        } else {
            format!("git push failed: {e}")
        };
    }

    if is_ja() {
        "GitHubへ反映しました".to_string()
    } else {
        "Pushed to GitHub".to_string()
    }
}

/// 1段の変化が表す実時間（ms）。時間軸で量子化することで、duration の長短に関わらず
/// 「約100msごとに1段」変化する見た目になる。
/// （旧実装は 1/128 固定段数だった。duration=60秒のイマジンデバフでは1段が469ms≒1px未満で
/// 実質静止して見え、duration=256秒のバフでは1段が約2.0秒相当と粗すぎた＝診断済み。）
const QUANTIZE_STEP_MS: f32 = 100.0;

/// 残量割合を時間軸ベースの粒度へ量子化する（バー幅・アーク円周のサブピクセル変化を吸収し
/// 行の再描画を省きつつ、duration に依存せず視覚的に意味のある刻みにする）。
/// 量子化を完全に外すと remaining_ms の1ms単位の変動が毎tick sync_model_if_changed の差分
/// 判定に引っかかり CPU が増えるため、粒度そのものは必ず残す。
/// `duration_ms<=0`（無期限）は段の概念が無いためそのまま返す。
fn quantize_ratio(r: f32, duration_ms: i64) -> f32 {
    let ratio = r.clamp(0.0, 1.0);
    if duration_ms <= 0 {
        return ratio;
    }
    let step = (QUANTIZE_STEP_MS / duration_ms as f32).min(1.0);
    ((ratio / step).round() * step).clamp(0.0, 1.0)
}

/// セル種別ごとの「次に表示が変わるまでの時間」算出結果（`format::next_text_change_ms` や
/// `imagine_cell_next_change_ms` 等、セルの表示規則に対応する関数から得る）を複数セル分
/// たたみ込み、最小値（＝最初に表示が変わるセル）を残す。両方 None なら None
/// （＝算出できるセルが1つも無い＝フォールバック対象）。
fn merge_next_change_ms(acc: Option<u64>, v: Option<u64>) -> Option<u64> {
    match (acc, v) {
        (None, x) => x,
        (x, None) => x,
        (Some(a), Some(b)) => Some(a.min(b)),
    }
}

// オーバーレイ更新スケジューリングの定数。専用の Repeated `Timer`（overlay_timer。W1:
// メインpollタイマーに相乗りさせると発火機会が poll グリッドに縛られ 200/400ms が不規則に
// 交替する事故があったため分離済み）が、毎回のコールバック末尾で `overlay_next_delay_ms` の
// 結果を `set_interval` することでスケジュールを実現する。以下の二重の意図を持つ:
// (a) 秒表示テキストを「次に秒が変わる時刻」に同期させる（秒境界同期）。
// (b) 稼働中バフのアーク/バーは、それとは独立になめらかに動き続けてほしい
//     （ユーザーが負荷を許容してでも滑らかさを優先すると明示。アークが1秒に1回しか
//     動かないのは要望に反する）。
// `quantize_ratio` の量子化粒度は時間軸で約100ms相当（QUANTIZE_STEP_MS）だが、
// OVERLAY_MAX_DELAY_MS=200ms の方が粗いため、実効の更新粒度は発火間隔の上限
// （このOVERLAY_MAX_DELAY_MS）側で決まる。量子化はそれより細かい変化を捨てて
// 再描画コスト（sync_model_if_changed の差分ヒット）を抑える役割に留まる。
//
// 表示セルが無い/算出できない場合に必ずフォールバックする固定間隔。
const OVERLAY_FALLBACK_MS: u64 = 200;
// 発火予定ちょうどに来るとタイマーの粒度差でわずかに早く判定され得るため、実際の秒境界を
// 確実に跨いでから発火するための余裕（poll_ms 未満の小さな値）。
const OVERLAY_DUE_MARGIN_MS: u64 = 30;
// アーク/バーの滑らかさを担保する上限間隔（秒境界までの残りがこれより長くても、
// ここより長く待たない）。値は旧stride実装（poll=200ms時で実効約400ms=2.5Hz）より
// 高頻度にする狙いで200ms(5Hz相当)に設定。
const OVERLAY_MAX_DELAY_MS: u64 = 200;
// overlay_timer の初回発火まで（以降は自身が算出した値で set_interval し続けるため、
// この値は起動直後の1回だけ効く）。
const OVERLAY_INITIAL_DELAY_MS: u64 = 30;
// オーバーレイを1つも表示していないときの待ち時間の下限。この状態ではコールバックが
// 更新する対象が無く、200ms(5Hz)で起こし続けても何も滑らかにならない純粋な空回りになる。
// 実際の待ち時間は poll_interval_ms との大きい方を採る（＝CPUを削るために poll を
// 伸ばしたユーザーの意図をこの状態に限り尊重する。表示中は従来どおり poll から独立）。
// 代償はオーバーレイを表示に切り替えてから最初の更新までの遅れで、上限はこの待ち時間。
const OVERLAY_IDLE_MIN_MS: u64 = 500;

/// オーバーレイ更新を実施した回で、次回発火までの待ち時間(ms)を確定する
/// （overlay_timer が呼び出し末尾で `set_interval` に使う）。
/// 秒境界までの残り(`ms`)を先に `OVERLAY_MAX_DELAY_MS - OVERLAY_DUE_MARGIN_MS` でクランプ
/// してからマージンを足す（＝マージン加算を先にすると 171〜200ms の残りが 200ms 丁度へ
/// クランプされてマージンが 1〜29ms まで目減りし、境界の手前で発火しうる。min を先にすることで
/// 採用される延期時間が短い場合は必ずマージン30ms分だけ境界より後ろへ倒れることを保証する）。
/// `next_change_ms` が None（表示中セルが1つも無い/算出できない）場合は OVERLAY_FALLBACK_MS
/// へ必ずフォールバックする（このガードを外すと再武装漏れで更新が永久停止する事故になるため
/// 必須。ただし overlay_timer は Repeated のため、万一ここへ到達できなくても直前の周期で
/// 回り続け、更新の永久停止そのものは起きない＝二重の安全策）。
fn overlay_next_delay_ms(next_change_ms: Option<u64>) -> u64 {
    next_change_ms
        .map(|ms| {
            ms.min(OVERLAY_MAX_DELAY_MS.saturating_sub(OVERLAY_DUE_MARGIN_MS))
                .saturating_add(OVERLAY_DUE_MARGIN_MS)
        })
        .unwrap_or(OVERLAY_FALLBACK_MS)
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
            // 枠色は優先度に依らず固定（バフ/デバフで色分けしない）。
            let border_color = slint::Color::from_argb_u8(0x33, 0xff, 0xff, 0xff);
            StatusEntryUi {
                name: buff_names::label(e.base_id).into(),
                remaining_text: format::format_remaining(e.remaining_ms, e.duration_ms).into(),
                bar_ratio: quantize_ratio(ratio, e.duration_ms),
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

/// ステータス窓の表示項目トグル一覧（カタログ × 現在の有効集合）。
/// グループが切り替わる先頭項目にのみ group-head（見出し文字列）を設定する。
fn build_stat_catalog(enabled: &[String]) -> Vec<StatCatalogItem> {
    let mut last_group = "";
    let mut out = Vec::with_capacity(settings::STAT_CATALOG.len());
    for d in settings::STAT_CATALOG {
        let group = d.group();
        let head = if group != last_group {
            last_group = group;
            group
        } else {
            ""
        };
        out.push(StatCatalogItem {
            key: d.key.into(),
            label: d.label().into(),
            group_head: head.into(),
            enabled: enabled.iter().any(|e| e == d.key),
        });
    }
    out
}

/// カタログを2列へ分割する。グループ境界（group_head が非空＝グループ先頭）を跨がず、
/// 左右の項目数がなるべく均等になる境界で割る。各列の先頭グループ見出しを保持するため。
fn split_stat_catalog(items: &[StatCatalogItem]) -> (Vec<StatCatalogItem>, Vec<StatCatalogItem>) {
    let half = items.len() / 2;
    // half 以上に達した最初のグループ先頭で割る（無ければ末尾＝右列空）。
    let split = items
        .iter()
        .enumerate()
        .skip(1)
        .find(|(i, it)| *i >= half && !it.group_head.is_empty())
        .map(|(i, _)| i)
        .unwrap_or(items.len());
    (items[..split].to_vec(), items[split..].to_vec())
}

/// 3桁区切りの整数文字列（例: 28500 → "28,500"）。
fn group_int(n: i64) -> String {
    let neg = n < 0;
    let digits = n.unsigned_abs().to_string();
    let len = digits.len();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if neg { format!("-{out}") } else { out }
}

/// SelfStatsData と有効項目（表示順）から UI 行を生成。値が無い項目は "—"。
/// ※ 割合系ステータス（値/100=%）と実測率（命中由来・既に%）を区別して整形する。
fn build_stat_entries(s: &bpsr_core::models::SelfStatsData, enabled: &[String]) -> Vec<StatEntryUi> {
    let dash = "—".to_string();
    let int_v = |o: Option<i32>| o.map(|v| group_int(v as i64)).unwrap_or_else(|| dash.clone());
    // 割合系の生値は「整数 ×100 の %」（例: 2485 = 24.85%）。/100 は厳密に 2 桁なので
    // 全精度（小数 2 桁）で表示する。format_pct（1 桁）は丸めで実在桁を捨てるため使わない。
    let pct_v = |o: Option<i32>| {
        o.map(|v| format!("{:.2}%", v as f64 / 100.0))
            .unwrap_or_else(|| dash.clone())
    };
    enabled
        .iter()
        .map(|key| {
            let (value, accent) = match key.as_str() {
                "hp" => {
                    // HP は全桁（3 桁区切り）で表示する。略記（K/M）は実数を丸めて精度を捨てるため使わない。
                    let v = match (s.curr_hp, s.max_hp) {
                        (Some(c), Some(m)) => {
                            format!("{} / {}", group_int(c as i64), group_int(m as i64))
                        }
                        (Some(c), None) => group_int(c as i64),
                        _ => dash.clone(),
                    };
                    (v, false)
                }
                // 整数系
                "atk-phys" => (int_v(s.attack_power), false),
                "atk-magic" => (int_v(s.magic_attack), false),
                "def-phys" => (int_v(s.defense_power), false),
                "def-magic" => (int_v(s.magic_defense), false),
                "endurance" => (int_v(s.endurance), false),
                "strength" => (int_v(s.strength), false),
                "intelligence" => (int_v(s.intelligence), false),
                "agility" => (int_v(s.agility), false),
                "ability-score" => (int_v(s.ability_score), false),
                "season-strength" => (int_v(s.season_strength), false),
                // 割合系（値/100=%）
                "haste" => (pct_v(s.haste), false),
                "attack-speed" => (pct_v(s.attack_speed), false),
                "cast-speed" => (pct_v(s.cast_speed), false),
                "lucky" => (pct_v(s.lucky), false),
                "crit" => (pct_v(s.crit_stat), false),
                "versatility" => (pct_v(s.versatility), false),
                "resist" => (pct_v(s.resist), false),
                "dexterity" => (pct_v(s.dexterity), false),
                "crit-dmg" => (pct_v(s.crit_dmg), false),
                "lucky-dmg" => (pct_v(s.lucky_dmg), false),
                // 実測率（命中データ由来・%・強調）
                "crit-rate" => (
                    if s.has_combat {
                        format::format_pct(s.crit_rate_measured)
                    } else {
                        dash.clone()
                    },
                    true,
                ),
                "lucky-rate" => (
                    if s.has_combat {
                        format::format_pct(s.lucky_rate_measured)
                    } else {
                        dash.clone()
                    },
                    true,
                ),
                _ => (dash.clone(), false),
            };
            let label = settings::STAT_CATALOG
                .iter()
                .find(|d| d.key == key)
                .map(|d| d.label())
                .unwrap_or(key.as_str());
            StatEntryUi {
                label: label.into(),
                value: value.into(),
                accent,
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

/// `buff_cell` の残り秒表示（常に ceil・1000ms格子・duration に依らず10秒閾値を持たない）が
/// 次に変わるまでの時間(ms)。自キャラ バフ/デバフ オーバーレイの `format::next_text_change_ms`
/// とは表示規則が異なる（10秒以下でも0.1秒刻みにならない）ため、こちらはバトルイマジンタイマー
/// セル専用に持つ（表示ロジックと「次に変わる時刻」の算出を隣接させ、同じ判定を二重に書かない）。
/// 無期限(duration_ms<=0)・表示上ゼロ以下(remaining_ms<=0)は変化しないため None。
fn imagine_cell_next_change_ms(duration_ms: i64, remaining_ms: i64) -> Option<u64> {
    if duration_ms <= 0 || remaining_ms <= 0 {
        return None;
    }
    let rem_mod = (remaining_ms % 1000) as u64;
    Some(if rem_mod == 0 { 1000 } else { rem_mod })
}

/// `next_change_ms` は「次に表示が変わるまでの時間」の集計用アキュムレータ。呼び出し側
/// （`build_buff_rows`）が実際に描画するセルからのみ収集できるよう、このセル単位の関数で
/// 受け取って更新する（S3: 集計対象を表示セル集合と一致させ、別トラバーサルによる二重化を防ぐ）。
fn buff_cell(
    snap: Option<&bpsr_core::models::SelfBuffSnapshot>,
    kind_hex: u32,
    next_change_ms: &mut Option<u64>,
) -> BuffCell {
    let color =
        slint::Color::from_rgb_u8((kind_hex >> 16) as u8, (kind_hex >> 8) as u8, kind_hex as u8);
    match snap {
        Some(b) => {
            *next_change_ms =
                merge_next_change_ms(*next_change_ms, imagine_cell_next_change_ms(b.duration_ms, b.remaining_ms));
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
                arc_commands: buff_arc(quantize_ratio(ratio, b.duration_ms)).into(),
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

/// `uids` を自分(local_uid)を先頭固定＋以降 uid 昇順（チラつかせない安定順）に並べ替える。
/// 同期OFF時の並びの土台（項目3「並び順の追従」OFF時）・専用モードの名簿順の土台に使う。
fn order_local_first_stable(uids: &[i64], local_uid: i64) -> Vec<i64> {
    let mut rest: Vec<i64> = uids.iter().copied().filter(|&u| u != local_uid).collect();
    rest.sort_unstable();
    if local_uid != 0 && uids.contains(&local_uid) {
        let mut out = Vec::with_capacity(uids.len());
        out.push(local_uid);
        out.extend(rest);
        out
    } else {
        rest
    }
}

/// イマジンタイマーの行順をメインDPS画面の並び(`main_ordered`)へ追従させる。
/// `roster` のうち main_ordered に在る uid をその順で先頭に、main に居ない roster
/// (戦闘から外れた等)は従来順で末尾へ。main_ordered が空なら roster を素通し。
fn order_by_main(roster: &[i64], main_ordered: &[i64]) -> Vec<i64> {
    if main_ordered.is_empty() {
        return roster.to_vec();
    }
    let roster_set: std::collections::HashSet<i64> = roster.iter().copied().collect();
    let in_main: std::collections::HashSet<i64> = main_ordered.iter().copied().collect();
    let mut out: Vec<i64> = main_ordered
        .iter()
        .copied()
        .filter(|u| roster_set.contains(u))
        .collect();
    // main に居ない roster は従来順で末尾に温存（漏れ防止）。
    out.extend(roster.iter().copied().filter(|u| !in_main.contains(u)));
    out
}

/// イマジンタイマーに実際に表示するプレイヤーの uid 列（＝メイン一覧のピン点灯集合と同一）。
///
/// 名簿源は3分岐:
/// - 専用モードON: `buff_tracked_uids`（バフ追跡から自動・first-seen順）から excluded を除き、
///   自分を先頭固定＋以降 first-seen 順（=元の並びをそのまま使う。安定ソート不要）で上限内に。
/// - 専用OFF・同期ON: メイン名簿順(`main_ordered`)から excluded を除いたもの。
///   並びは `order_follow` 次第（ON=メイン順そのまま／OFF=自分先頭＋uid昇順の安定順）。
/// - 専用OFF・同期OFF: 手動ウォッチ(`wl.watched`)のみ。メイン順があれば追従させる。
#[allow(clippy::too_many_arguments)]
fn timer_roster(
    wl: &watchlist::Watchlist,
    imagine_only: bool,
    sync: bool,
    order_follow: bool,
    main_ordered: &[i64],
    buff_tracked_uids: &[i64],
    local_uid: i64,
) -> Vec<i64> {
    if imagine_only {
        let filtered: Vec<i64> = buff_tracked_uids
            .iter()
            .copied()
            .filter(|u| !wl.excluded.contains(u))
            .collect();
        return order_local_first_stable(&filtered, local_uid)
            .into_iter()
            .take(watchlist::MAX)
            .collect();
    }
    if sync {
        let filtered: Vec<i64> = main_ordered
            .iter()
            .copied()
            .filter(|u| !wl.excluded.contains(u))
            .collect();
        let ordered = if order_follow {
            filtered
        } else {
            order_local_first_stable(&filtered, local_uid)
        };
        return ordered.into_iter().take(watchlist::MAX).collect();
    }
    order_by_main(&wl.watched, main_ordered)
}

/// `roster` の表示順で行を組む。`privacy_mask`=true のとき名前は `format::mask_player_name` で
/// マスクする（メイン行 `build_rows` と同じ規約）。見つからない uid は uid 下16bit の数値表示。
/// `roster` の表示順で行を組む。戻り値の2つ目は、実際にセル化した（＝表示される）バフの
/// remaining_ms から集計した「次に表示が変わるまでの時間」（オーバーレイの秒境界同期用。
/// S3: 集計対象を表示セル集合そのものに揃えるため、別途 `tracked.players[].buffs[]` 全件を
/// 走査するのではなく、ここでの `buff_cell` 呼び出しから直接集める）。
fn build_buff_rows(
    tracked: &bpsr_core::models::TrackedBuffsData,
    roster: &[i64],
    privacy_mask: bool,
) -> (Vec<BuffPlayerRow>, Option<u64>) {
    let mut next_change_ms: Option<u64> = None;
    let mut rows = Vec::with_capacity(roster.len());
    for &uid in roster {
        let snap = tracked.players.iter().find(|p| p.uid as i64 == uid);
        let display = if privacy_mask {
            format::mask_player_name(uid)
        } else {
            let name = snap.map(|s| s.name.clone()).unwrap_or_default();
            if name.is_empty() {
                format!("{}", uid & 0xffff)
            } else {
                name
            }
        };
        let find = |kind: &str| snap.and_then(|s| s.buffs.iter().find(|b| b.kind == kind));
        rows.push(BuffPlayerRow {
            name: display.into(),
            tina: buff_cell(find("Tina"), 0xff4d6d, &mut next_change_ms),
            aluna: buff_cell(find("Aluna"), 0x5fd35f, &mut next_change_ms),
            tarta: buff_cell(find("Tarta"), 0xb98bff, &mut next_change_ms),
            basilisk: buff_cell(find("Basilisk"), 0xd9a05b, &mut next_change_ms),
            kartgriff: buff_cell(find("Kartgriff"), 0x4fc3f7, &mut next_change_ms),
        });
    }
    (rows, next_change_ms)
}

/// ポーリングループが tick 間で持ち越す可変状態（位置復元の進行管理）。
#[derive(Default)]
struct PollState {
    tick: u64,
    setup_tick: u64,
    setup_done: bool,
    // オーバーレイ復元tick（None=未復元）。復元前のデフォルト位置で保存上書きしないよう、
    // 復元から SETTLE_TICKS 経過後に保存対象へ含める。非表示で None に戻す。
    self_rtick: Option<u64>,
    buff_rtick: Option<u64>,
    stats_rtick: Option<u64>,
    // 復元が実際に適用した（クランプ後の）矩形。settle 期間中はこのサイズを毎tick 再適用。
    restored_main: Option<window_state::WinRect>,
    restored_self: Option<window_state::WinRect>,
    restored_buffs: Option<window_state::WinRect>,
    restored_stats: Option<window_state::WinRect>,
}

/// オーバーレイ(バフ/ステータス/イマジンタイマー)専用タイマー（overlay_timer）が呼び出し間で
/// 持ち越す状態。W1: メインpollタイマーの `PollState` とはスケジュール（秒境界同期＋滑らかさ
/// 上限クランプ）が独立しているため別構造体に分離している。
#[derive(Default)]
struct OverlayState {
    // オーバーレイへ最後に push した内容（前回と同一なら set_vec を省いて無駄な再描画を避ける）。
    // オーバーレイは sync_rows と違い set_vec で毎tick モデル全置換していたため、内容不変でも
    // 5Hz で再描画され CPU を浪費していた（実測: 2窓表示で約11%/1コア）。
    last_self_buffs: Vec<StatusEntryUi>,
    last_self_debuffs: Vec<StatusEntryUi>,
    last_stats_rows: Vec<StatEntryUi>,
    last_buff_players: Vec<BuffPlayerRow>,
}

/// 行数一致時は前回と異なる行だけ set_row_data で更新する（Slint のプロパティ
/// 重複排除により内容不変の行は再描画されない）。行数変化時のみ set_vec で全置換。
/// set_vec_if_changed（全置換）と違い、変化行のみ再描画するためオーバーレイの
/// 稼働中コストを大幅に削減できる。
fn sync_model_if_changed<T>(model: &slint::VecModel<T>, last: &mut Vec<T>, next: Vec<T>)
where
    T: Clone + PartialEq + 'static,
{
    sync_model_if_changed_by(model, last, next, T::eq);
}

/// [`sync_model_if_changed`] の等価判定差し替え版。`PartialEq` が使えない型（`ModelRc` を含む行は
/// ポインタ比較になり毎回不一致になる）向け。
fn sync_model_if_changed_by<T>(
    model: &slint::VecModel<T>,
    last: &mut Vec<T>,
    next: Vec<T>,
    same: impl Fn(&T, &T) -> bool,
) where
    T: Clone + 'static,
{
    if model.row_count() != next.len() {
        model.set_vec(next.clone());
        *last = next;
        return;
    }
    for (i, item) in next.iter().enumerate() {
        if !last.get(i).is_some_and(|prev| same(prev, item)) {
            model.set_row_data(i, item.clone());
        }
    }
    *last = next;
}

/// 履歴ビューの行モデルと、直近に流し込んだ内容。展開行は NameTemplate・RowStatColumns を持つため、
/// 毎 tick の全置換は重い。内容が変わった行だけ更新する（[`sync_model_if_changed_by`]）。
/// 流し込みは必ずこの型の [`HistoryRows::apply`] を通す（`last` とモデルを食い違わせないため）。
struct HistoryRows {
    model: Rc<VecModel<HistoryRowUi>>,
    last: RefCell<Vec<HistoryRowUi>>,
}

impl HistoryRows {
    fn new() -> Rc<Self> {
        Rc::new(Self { model: Rc::new(VecModel::default()), last: RefCell::new(Vec::new()) })
    }

    fn apply(&self, next: Vec<HistoryRowUi>) {
        sync_model_if_changed_by(&self.model, &mut self.last.borrow_mut(), next, history_row_same);
    }
}

/// 履歴行の内容比較。名前パーツは `ModelRc`（ポインタ比較）なので中身で比べ、残りは `==`。
fn history_row_same(a: &HistoryRowUi, b: &HistoryRowUi) -> bool {
    let parts_a = &a.row.name_parts;
    let parts_b = &b.row.name_parts;
    if parts_a.row_count() != parts_b.row_count()
        || !(0..parts_a.row_count()).all(|i| parts_a.row_data(i) == parts_b.row_data(i))
    {
        return false;
    }
    let (mut a, mut b) = (a.clone(), b.clone());
    a.row.name_parts = slint::ModelRc::default();
    b.row.name_parts = slint::ModelRc::default();
    a == b
}

/// 初回 tick: winit 実体化後にメイン窓を復元する。復元が完了した tick で true を返す
/// （呼び出し側はその tick でトレイ生成などの後処理を行う）。
fn poll_setup_once(m: &MainWindow, st: &mut PollState, saved: &window_state::Layout) -> bool {
    if st.setup_done {
        return false;
    }
    let mons = overlay::monitors(m.window());
    if mons.is_empty() {
        return false;
    }
    st.restored_main =
        Some(window_state::restore(m.window(), saved.main.as_ref(), &mons, 0, (520, 350)));
    st.setup_done = true;
    st.setup_tick = st.tick;
    log::info!("window restored on {} monitor(s)", mons.len());
    true
}

/// オーバーレイの位置/サイズ復元（表示された最初の tick で一度）。非表示で None に戻す。
fn poll_overlay_restore(
    st: &mut PollState,
    cfg: &RefCell<settings::Settings>,
    self_overlay_w: &slint::Weak<SelfStatusOverlay>,
    buff_overlay_w: &slint::Weak<BuffOverlay>,
    stats_overlay_w: &slint::Weak<StatsOverlay>,
    last_saved: &RefCell<window_state::Layout>,
) {
    let c = cfg.borrow();
    // 完全透明(不透明度0)のオーバーレイはクリック透過(HUD)にする。apply_overlay_appearance は
    // 起動時/表示トグル時に窓の winit 実体化より前へ走り set_cursor_hittest が空振りするため、
    // 実体化を確認できるこの復元 tick で改めて適用する（EXSTYLE 競合回避のため taskbar_mode の前）。
    let pass_through = c.overlay_opacity <= 0.001;
    if c.show_stats_overlay {
        if st.stats_rtick.is_none() {
            if let Some(o) = stats_overlay_w.upgrade() {
                let mons = overlay::monitors(o.window());
                if !mons.is_empty() {
                    st.restored_stats = Some(window_state::restore(
                        o.window(),
                        last_saved.borrow().stats.as_ref(),
                        &mons,
                        0,
                        (200, 220),
                    ));
                    st.stats_rtick = Some(st.tick);
                    overlay::set_click_through(o.window(), pass_through);
                    #[cfg(windows)]
                    overlay::apply_taskbar_mode(o.window(), c.show_in_taskbar);
                }
            }
        }
    } else {
        st.stats_rtick = None;
        st.restored_stats = None;
    }
    if c.show_self_status_overlay {
        if st.self_rtick.is_none() {
            if let Some(o) = self_overlay_w.upgrade() {
                let mons = overlay::monitors(o.window());
                if !mons.is_empty() {
                    st.restored_self = Some(window_state::restore(
                        o.window(),
                        last_saved.borrow().self_status.as_ref(),
                        &mons,
                        0,
                        (220, 180),
                    ));
                    st.self_rtick = Some(st.tick);
                    overlay::set_click_through(o.window(), pass_through);
                    // 実体化したオーバーレイへ現在のタスクバー常駐モードを適用。
                    #[cfg(windows)]
                    overlay::apply_taskbar_mode(o.window(), c.show_in_taskbar);
                }
            }
        }
    } else {
        st.self_rtick = None;
        st.restored_self = None;
    }
    if c.show_buff_overlay {
        if st.buff_rtick.is_none() {
            if let Some(o) = buff_overlay_w.upgrade() {
                let mons = overlay::monitors(o.window());
                if !mons.is_empty() {
                    st.restored_buffs = Some(window_state::restore(
                        o.window(),
                        last_saved.borrow().buffs.as_ref(),
                        &mons,
                        0,
                        // 5列全表示が収まる既定幅（BuffOverlay の preferred-width と同値）。
                        (350, 150),
                    ));
                    st.buff_rtick = Some(st.tick);
                    overlay::set_click_through(o.window(), pass_through);
                    // 実体化したオーバーレイへ現在のタスクバー常駐モードを適用。
                    #[cfg(windows)]
                    overlay::apply_taskbar_mode(o.window(), c.show_in_taskbar);
                }
            }
        }
    } else {
        st.buff_rtick = None;
        st.restored_buffs = None;
    }
}

/// トレイメニューのイベント処理（クリックスルー切替・表示/非表示・終了）。
#[cfg(windows)]
fn poll_tray_events(
    m: &MainWindow,
    cfg: &RefCell<settings::Settings>,
    self_overlay_w: &slint::Weak<SelfStatusOverlay>,
    buff_overlay_w: &slint::Weak<BuffOverlay>,
    stats_overlay_w: &slint::Weak<StatsOverlay>,
    tray_holder: &RefCell<Option<tray::Tray>>,
    main_visible: &Cell<bool>,
    click_through: &Cell<bool>,
) {
    let holder = tray_holder.borrow();
    let Some(tray) = holder.as_ref() else {
        return;
    };
    while let Ok(ev) = tray_icon::menu::MenuEvent::receiver().try_recv() {
        if ev.id == tray.id_quit {
            let _ = slint::quit_event_loop();
        } else if ev.id == tray.id_show_hide {
            let vis = !main_visible.get();
            main_visible.set(vis);
            let _ = if vis { m.show() } else { m.hide() };
            // オーバーレイもメインの表示/格納に追従（復帰時は設定で有効なものだけ）
            let c = cfg.borrow();
            if let Some(o) = self_overlay_w.upgrade() {
                let _ = if vis && c.show_self_status_overlay {
                    o.show()
                } else {
                    o.hide()
                };
            }
            if let Some(o) = buff_overlay_w.upgrade() {
                let _ = if vis && c.show_buff_overlay {
                    o.show()
                } else {
                    o.hide()
                };
            }
            if let Some(o) = stats_overlay_w.upgrade() {
                let _ = if vis && c.show_stats_overlay {
                    o.show()
                } else {
                    o.hide()
                };
            }
        } else if ev.id == tray.id_click_through {
            let on = !click_through.get();
            click_through.set(on);
            tray.click_through.set_checked(on);
            overlay::set_click_through(m.window(), on);
            if let Some(o) = self_overlay_w.upgrade() {
                overlay::set_click_through(o.window(), on);
            }
            if let Some(o) = buff_overlay_w.upgrade() {
                overlay::set_click_through(o.window(), on);
            }
            if let Some(o) = stats_overlay_w.upgrade() {
                overlay::set_click_through(o.window(), on);
            }
        }
    }
    // トレイアイコン左クリックでメインを復帰（トレイ格納モードの復帰口）。
    while let Ok(ev) = tray_icon::TrayIconEvent::receiver().try_recv() {
        if let tray_icon::TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = ev
        {
            main_visible.set(true);
            let _ = m.show();
            overlay::restore_window(m.window());
            // 設定で有効なオーバーレイも一緒に復帰させる
            let c = cfg.borrow();
            if c.show_self_status_overlay {
                if let Some(o) = self_overlay_w.upgrade() {
                    let _ = o.show();
                }
            }
            if c.show_buff_overlay {
                if let Some(o) = buff_overlay_w.upgrade() {
                    let _ = o.show();
                }
            }
            if c.show_stats_overlay {
                if let Some(o) = stats_overlay_w.upgrade() {
                    let _ = o.show();
                }
            }
        }
    }
}

/// グローバルショートカット発火の処理。発火した（Pressedのみ）アクションを対応する既存
/// コールバックの invoke へ回す（ボタン経由と完全に同じ経路を通す。専用の実処理関数へ
/// 切り出す必要はない）。
#[cfg(windows)]
fn poll_hotkey_events(m: &MainWindow, hotkeys_holder: &RefCell<Option<hotkey::Hotkeys>>) {
    // borrow は poll() 呼び出しの間だけに留め、invoke_* 実行中まで生存させない
    // （invoke_* からショートカット再適用 = hotkeys_holder.borrow_mut() が増えた瞬間に
    // BorrowMutError でパニックするのを避ける）。poll() は Vec<ShortcutAction> を
    // 所有権ごと返すため、fired は hotkeys_holder を借用し続けない。
    let fired = hotkeys_holder.borrow().as_ref().map(|hk| hk.poll()).unwrap_or_default();
    // 多重防御（C1）: suspend/revalidate の運用が万一崩れて OS 側に登録が残っていても、
    // ダイアログ表示中は発火を無視して本番アクションの誤爆だけは必ず止める。poll() 自体は
    // 上で呼び済み（＝チャネルは drain 済み）なので、ここで早期 return してもダイアログを
    // 閉じた直後に溜め込んだ分が一気に発火することはない。
    if m.get_shortcut_dialog_open() {
        return;
    }
    for action in fired {
        match action {
            hotkey::ShortcutAction::ResetEncounter => m.invoke_reset_encounter(),
            hotkey::ShortcutAction::TogglePause => m.invoke_toggle_pause(),
            hotkey::ShortcutAction::ToggleMeasure => m.invoke_toggle_measure(),
            hotkey::ShortcutAction::CopyList => m.invoke_copy_list(),
            hotkey::ShortcutAction::ToggleAlwaysOnTop => {
                m.invoke_set_bool("aot".into(), !m.get_aot())
            }
        }
    }
}

/// メイン窓の復元が完了し、settle 期間（起動直後の再アサート対策で毎tick 復元サイズを
/// 強制適用する期間）も終わったか。poll_window_settle の「まだ強制中」も
/// `setup_done && !main_settled` で導出し、settle 完了後にしか安全に行えない処理
/// （自動保存・grow_to_min）と判定を1か所に揃える。
fn main_settled(st: &PollState) -> bool {
    st.setup_done && st.tick >= st.setup_tick + SETTLE_TICKS
}

/// 起動/表示直後に Slint が preferred サイズを再アサートして保存サイズを上書きする
/// ことがあるため、settle 期間中は毎tick 復元サイズを再適用する（サイズ一致なら no-op）。
fn poll_window_settle(
    m: &MainWindow,
    st: &PollState,
    self_overlay_w: &slint::Weak<SelfStatusOverlay>,
    buff_overlay_w: &slint::Weak<BuffOverlay>,
    stats_overlay_w: &slint::Weak<StatsOverlay>,
) {
    if st.setup_done && !main_settled(st) {
        if let Some(r) = &st.restored_main {
            window_state::enforce_size(m.window(), r);
        }
    }
    if let (Some(rt), Some(r)) = (st.stats_rtick, st.restored_stats.as_ref()) {
        if st.tick < rt + SETTLE_TICKS {
            if let Some(o) = stats_overlay_w.upgrade() {
                window_state::enforce_size(o.window(), r);
            }
        }
    }
    if let (Some(rt), Some(r)) = (st.self_rtick, st.restored_self.as_ref()) {
        if st.tick < rt + SETTLE_TICKS {
            if let Some(o) = self_overlay_w.upgrade() {
                window_state::enforce_size(o.window(), r);
            }
        }
    }
    if let (Some(rt), Some(r)) = (st.buff_rtick, st.restored_buffs.as_ref()) {
        if st.tick < rt + SETTLE_TICKS {
            if let Some(o) = buff_overlay_w.upgrade() {
                window_state::enforce_size(o.window(), r);
            }
        }
    }
}

/// レイアウト自動保存（復元確定後・差分時のみ）。オーバーレイは復元から SETTLE_TICKS
/// 経過後のみ保存対象に含める（復元前の既定位置で上書き防止）。
fn poll_auto_save(
    m: &MainWindow,
    st: &PollState,
    cfg: &RefCell<settings::Settings>,
    self_overlay_w: &slint::Weak<SelfStatusOverlay>,
    buff_overlay_w: &slint::Weak<BuffOverlay>,
    stats_overlay_w: &slint::Weak<StatsOverlay>,
    last_saved: &RefCell<window_state::Layout>,
) {
    if !main_settled(st) {
        return;
    }
    let tick = st.tick;
    let settled = |rt: Option<u64>| rt.map(|t| tick >= t + SETTLE_TICKS).unwrap_or(false);
    let cur = {
        let c = cfg.borrow();
        let prev = last_saved.borrow();
        window_state::Layout {
            main: Some(window_state::capture(m.window())),
            self_status: if c.show_self_status_overlay && settled(st.self_rtick) {
                self_overlay_w
                    .upgrade()
                    .map(|o| window_state::capture(o.window()))
            } else {
                prev.self_status.clone()
            },
            buffs: if c.show_buff_overlay && settled(st.buff_rtick) {
                buff_overlay_w
                    .upgrade()
                    .map(|o| window_state::capture(o.window()))
            } else {
                prev.buffs.clone()
            },
            stats: if c.show_stats_overlay && settled(st.stats_rtick) {
                stats_overlay_w
                    .upgrade()
                    .map(|o| window_state::capture(o.window()))
            } else {
                prev.stats.clone()
            },
        }
    };
    let mut ls = last_saved.borrow_mut();
    if *ls != cur {
        window_state::save(&cur);
        *ls = cur;
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    static LOGGER: ConsoleLog = ConsoleLog;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
    log::info!("log file: {}", log_file_path().display());

    if already_running() {
        log::warn!("another instance is already running; exiting");
        return Ok(());
    }

    // 全ウィンドウ共通のタスクバー常駐フラグ。生成時 hook が参照するため設定を先読みする
    // （cfg より前。以降の切替は on_set_bool でこの Cell とウィンドウへ反映）。
    let taskbar_flag = Rc::new(Cell::new(settings::load().show_in_taskbar));

    // winit backend（透明合成可能。skip_taskbar は設定でメイン/オーバーレイ一括切替）
    let backend = {
        let tf = taskbar_flag.clone();
        i_slint_backend_winit::Backend::builder()
            .with_window_attributes_hook(move |attrs| {
                let attrs = attrs.with_transparent(true);
                #[cfg(target_os = "windows")]
                let attrs = {
                    use i_slint_backend_winit::winit::platform::windows::WindowAttributesExtWindows;
                    attrs.with_skip_taskbar(!tf.get())
                };
                attrs
            })
            .build()?
    };
    slint::platform::set_platform(Box::new(backend)).map_err(|e| format!("set_platform: {e:?}"))?;

    // ローカルデバッグ専用: Slint 埋め込み MCP サーバーを起動する（feature "mcp" 時のみ）。
    // 通常はこの init は backend-selector が呼ぶが、本アプリは set_platform で winit を直接
    // 注入し selector をバイパスしているため、ここで明示的に呼ぶ必要がある。
    // 実際に待受けるのは起動時 env `SLINT_MCP_PORT` 設定時のみ（未設定なら即 return）。
    #[cfg(feature = "mcp")]
    if let Err(e) = i_slint_backend_testing::mcp_server::init() {
        log::warn!("MCP サーバー初期化に失敗: {e:?}");
    }

    // 永続キャッシュ初期化
    let dir = data_dir();
    engine::name_cache::init(dir.join("name_cache.json"));
    engine::selected_uid::init(dir.join("selected_uid.json"));
    engine::consumables::init(dir.join("consumables.json"));
    engine::imagine_overrides::init(dir.join("imagine_overrides.json"));

    // 共有エンカウンター＋パケット観測スレッド
    // BPSR_DEMO=1 のときは観測の代わりに合成データを流す（撮影・UI確認用）
    let demo_mode = std::env::var("BPSR_DEMO").is_ok_and(|v| v == "1");
    let enc = Arc::new(EncounterMutex::default());
    if let Some(uid) = engine::selected_uid::get() {
        if let Ok(mut e) = enc.lock() {
            e.local_player_uid = uid;
        }
    }
    // 前回終了時の食事/シロップ残時間を復元（失効分は load 側で除去）。
    compute::load_consumables(&enc);
    if demo_mode {
        bpsr_core::engine::demo::spawn(enc.clone());
    } else {
        capture::spawn(enc.clone());
    }

    // 設定（%APPDATA%\bpsr-checker\settings.json）。UIスレッドで共有・編集する。
    let cfg = Rc::new(RefCell::new(settings::load()));
    // 中文UIは日本語フォント(Yu Gothic UI)に簡体字グリフが無く豆腐(□)になるため、
    // 既定フォントのままなら簡体字対応フォントへ差し替える（ユーザーが明示変更していれば尊重）。
    {
        let mut c = cfg.borrow_mut();
        if c.language == "zh" && c.main_font == settings::Settings::default().main_font {
            c.main_font = "Microsoft YaHei".to_string();
        }
    }
    // ウォッチリスト（バフタイマー追跡対象）。
    let wl = Rc::new(RefCell::new(watchlist::Watchlist::load()));

    // メイン窓
    let main = MainWindow::new()?;
    // 言語選択（設定 language。既定は日本語）。最初のコンポーネント生成後に呼ぶ。
    match slint::select_bundled_translation(&cfg.borrow().language) {
        Ok(()) => log::info!("translation: selected '{}'", cfg.borrow().language),
        Err(e) => log::warn!("translation: select '{}' failed: {e}", cfg.borrow().language),
    }
    // 名前辞書（スキル/モンスター/バフ）の表示言語も起動時に揃える（UI @tr と同じく再起動反映）。
    engine::runtime_settings::set_display_lang(engine::runtime_settings::Lang::from_code(
        &cfg.borrow().language,
    ));
    // バージョン表示（設定パネル最下部＋結果モーダルの透かし。画像コピーに写り込むクレジット）
    main.set_app_version(format!("bpsr-checker v{}", env!("CARGO_PKG_VERSION")).into());
    let rows = Rc::new(VecModel::<Row>::default());
    main.set_rows(rows.clone().into());
    // 軽量分割表示の左右カラム（rows を前半/後半に分配）
    let compact_left = Rc::new(VecModel::<Row>::default());
    let compact_right = Rc::new(VecModel::<Row>::default());
    main.set_compact_left(compact_left.clone().into());
    main.set_compact_right(compact_right.clone().into());

    // 現在タブ（UIスレッド共有）。タブクリックと周期ポーリングの両方が参照する。
    let tab_cell = Rc::new(Cell::new(0i32));

    // スキル内訳ビュー用モデル＋対象プレイヤー uid（0=なし）
    let skill_rows = Rc::new(VecModel::<SkillRowUi>::default());
    main.set_skill_rows(skill_rows.clone().into());
    let drill = Rc::new(Cell::new(Drill::None));

    // 履歴ビュー用モデル＋展開中エンカウンタ/プレイヤー（各 None=折りたたみ）
    let history_rows = HistoryRows::new();
    main.set_history_rows(history_rows.model.clone().into());
    let history_expanded = Rc::new(Cell::new(None::<i64>));
    let history_player_expanded = Rc::new(Cell::new(None::<(i64, i64)>));

    // 3分計測 結果パネル用モデル＋最後の結果スナップショット（コピー/再計測で参照）
    let result_rows = Rc::new(VecModel::<ResultRowUi>::default());
    main.set_result_rows(result_rows.clone().into());
    let result_skill_rows = Rc::new(VecModel::<ResultSkillRowUi>::default());
    main.set_result_skill_rows(result_skill_rows.clone().into());
    let result_pie = Rc::new(VecModel::<PieSlice>::default());
    main.set_result_pie(result_pie.clone().into());
    let result_legend = Rc::new(VecModel::<SkillLegendUi>::default());
    main.set_result_legend(result_legend.clone().into());
    let last_result = Rc::new(RefCell::new(None::<bpsr_core::models::EncounterSnapshot>));
    // finalize 前に捕捉した各プレイヤーのスキル内訳（uid → スキル行[降順・時系列付き]）
    let captured_skills = Rc::new(RefCell::new(std::collections::HashMap::<
        i64,
        Vec<bpsr_core::models::SkillRow>,
    >::new()));
    // 結果パネルの選択状態（エリア1=キャラ / エリア2=スキル）
    let selected_result_player = Rc::new(std::cell::Cell::<i64>::new(0));
    let selected_result_skill = Rc::new(std::cell::Cell::<i64>::new(0));
    // 自己ベスト記録（%APPDATA%\bpsr-checker\best_records.json）。デモモードでは実ファイルを
    // 汚染しないため load せず空から開始する（メモリ上の比較・新記録判定はデモでも動作する）。
    let best_records = Rc::new(RefCell::new(if demo_mode {
        best_records::BestRecords::default()
    } else {
        best_records::BestRecords::load()
    }));
    // 結果パネルの DPS/総ダメ カウントアップ演出専用タイマー（新規計測確定時のみ使う）。
    // Timer::start は呼び出すたびに再起動されるため、Rc で使い回して寿命を保つ
    // （完了時に自身を stop するため Rc clone を closure に渡す）。
    let result_countup_timer = Rc::new(Timer::default());
    // カウントアップ中の最終値（画像コピー時に即完了させ、途中値が写り込むのを防ぐため共有）。
    let result_countup_final = Rc::new(Cell::new((0.0_f64, 0.0_f64)));

    // 自キャラUID 候補モデル
    let uid_candidates = Rc::new(VecModel::<UidCandidate>::default());
    main.set_uid_candidates(uid_candidates.clone().into());

    // グローバルショートカット（issue #3）。行順は hotkey::ACTIONS と一致（index対応）。
    // モデル自体は全platform共通（ShortcutUiは.slint側の素の構造体）。実際のOS登録・ポーリングは
    // Windows専用（hotkey.rs）。非Windowsではhotkeys_holderが無くモデルは空のまま残る。
    let shortcuts_model = Rc::new(VecModel::<ShortcutUi>::default());
    main.set_shortcuts(shortcuts_model.clone().into());
    #[cfg(windows)]
    let hotkeys_holder: Rc<RefCell<Option<hotkey::Hotkeys>>> = Rc::new(RefCell::new(None));
    // 行数の正典は hotkey::ACTIONS（W3-4）。app.slint の shortcut-labels は @tr が必要なため
    // 別建てせざるを得ない配列なので、件数がズレていないかを起動時に検知する
    // （本番では黙って空ラベル行が出るだけになるため、開発中に気付けるよう debug_assert に留める）。
    #[cfg(windows)]
    debug_assert_eq!(
        hotkey::ACTIONS.len(),
        main.get_shortcut_label_count() as usize,
        "hotkey::ACTIONS と app.slint の shortcut-labels の件数が一致していません"
    );

    // バトルイマジン名メンテ一覧（別ウィンドウを開く時／編集時のみ再構築。戦闘ポーリングでは触らない）。
    // 開発者モード(BPSR_DEV=1)のときだけイマジン名列の編集・GitHub反映ボタンをUIに出す。
    let imagine_dev_mode = std::env::var("BPSR_DEV").is_ok_and(|v| v == "1");
    let imagine_rows_model = Rc::new(VecModel::<ImagineNameRowUi>::default());
    let imagine_expanded: Rc<RefCell<std::collections::HashSet<String>>> =
        Rc::new(RefCell::new(std::collections::HashSet::new()));
    let imagine_filter: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    // バトルイマジン名メンテ窓（通常の OS タイトルバー付き。MainWindow/オーバーレイと違い
    // no-frame にせず always-on-top にもしない。×閉じは Slint 既定で hide のみ＝プロセス継続）。
    let imagine_window = ImagineNamesWindow::new()?;
    imagine_window.set_imagine_dev_mode(imagine_dev_mode);
    imagine_window.set_imagine_rows(imagine_rows_model.clone().into());
    // 初回 show() 直後にのみ既定サイズを強制する（以後はユーザーのリサイズを尊重）。
    // Slint の preferred-width/height は winit のデフォルト初期サイズ(800x600)に負けることが
    // あるため、MainWindow/オーバーレイと同じ window_state::enforce_size(winit 直叩き)で確実に適用する。
    let imagine_window_sized = Rc::new(Cell::new(false));

    // ステータス窓の表示項目トグル一覧（設定の有効集合から生成・トグルで再生成）。
    // 設定パネルでは2列表示するため、グループ境界で左右へ分割した2モデルを持つ。
    let stat_catalog_left = Rc::new(VecModel::<StatCatalogItem>::default());
    let stat_catalog_right = Rc::new(VecModel::<StatCatalogItem>::default());
    {
        let (l, r) = split_stat_catalog(&build_stat_catalog(&cfg.borrow().stats_enabled));
        stat_catalog_left.set_vec(l);
        stat_catalog_right.set_vec(r);
    }
    main.set_stat_catalog_left(stat_catalog_left.clone().into());
    main.set_stat_catalog_right(stat_catalog_right.clone().into());

    // 自キャラ ステータス オーバーレイ（別ウィンドウ）
    let stats_overlay = StatsOverlay::new()?;
    let stats_rows = Rc::new(VecModel::<StatEntryUi>::default());
    stats_overlay.set_stats(stats_rows.clone().into());
    wire_overlay_chrome!(stats_overlay, main, "stats-overlay");
    stats_overlay.set_show_minimize(cfg.borrow().show_in_taskbar);

    // 自キャラ バフ/デバフ オーバーレイ（別ウィンドウ）
    let self_overlay = SelfStatusOverlay::new()?;
    let self_buffs = Rc::new(VecModel::<StatusEntryUi>::default());
    let self_debuffs = Rc::new(VecModel::<StatusEntryUi>::default());
    self_overlay.set_buffs(self_buffs.clone().into());
    self_overlay.set_debuffs(self_debuffs.clone().into());
    wire_overlay_chrome!(self_overlay, main, "self-status-overlay");
    self_overlay.set_show_minimize(cfg.borrow().show_in_taskbar);

    // バフタイマー オーバーレイ（別ウィンドウ）
    let buff_overlay = BuffOverlay::new()?;
    let buff_players = Rc::new(VecModel::<BuffPlayerRow>::default());
    buff_overlay.set_players(buff_players.clone().into());
    wire_overlay_chrome!(buff_overlay, main, "buff-overlay");
    buff_overlay.set_show_minimize(cfg.borrow().show_in_taskbar);

    // オーバーレイ共通の外観（不透明度・フォント・基準色・サイズ）を起動時に反映。
    apply_overlay_appearance(
        &cfg.borrow(),
        &self_overlay.as_weak(),
        &buff_overlay.as_weak(),
        &stats_overlay.as_weak(),
    );

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
        // 食事/シロップ0ダメージ特例行のPTフィルタ（runtime_settings atomic。IMAGINE_ONLY_MODE
        // と同じ流儀。消費箇所は compute::build_players_window_unsorted 1箇所のみ）。
        engine::runtime_settings::set_party_only_consumables(c.party_only_consumables);
    }
    // 戦闘履歴の永続化（%APPDATA%\bpsr-checker\history.json）。
    // set_history_limit 適用後に呼ぶことで、起動時 load が設定済みの上限件数で正しく
    // 切り詰められる。デモモードでは合成データで実ファイルを汚染しないため init 自体を
    // 呼ばない（consumables/name_cache 同様、init 未呼び出しなら load/save は no-op）。
    if !demo_mode {
        engine::history::init(dir.join("history.json"));
    }

    // デモモードの撮影補助: 設定パネルを開いた状態にする / 3分計測を自動開始する
    if demo_mode {
        if std::env::var("BPSR_DEMO_OPEN_SETTINGS").is_ok_and(|v| v == "1") {
            main.set_settings_open(true);
        }
        let demo_3min = std::env::var("BPSR_DEMO_3MIN")
            .ok()
            .and_then(|s| s.parse::<f64>().ok());
        if let Some(secs) = demo_3min {
            if secs > 0.0 {
                compute::start_3min_measure_mode(&enc, secs, measure_scope(&cfg.borrow()));
            }
        }
    }

    main.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    // タスクトレイ表示/非表示と共有する可視状態。最小化ボタンもこの経路でトレイへ格納する
    // （skip_taskbar 窓は OS 最小化だとタスクバーにもトレイにも復帰口が無いため hide で代替）。
    let main_visible = Rc::new(Cell::new(true));
    {
        let w = main.as_weak();
        let mv = main_visible.clone();
        let self_ov = self_overlay.as_weak();
        let buff_ov = buff_overlay.as_weak();
        let cfg_min = cfg.clone();
        main.on_minimize(move || {
            let taskbar = cfg_min.borrow().show_in_taskbar;
            if let Some(m) = w.upgrade() {
                if taskbar {
                    // タスクバー常駐: OS最小化（タスクバーボタンから復帰）。
                    // 復帰がトレイ経路を通らないため可視状態は維持し、
                    // オーバーレイ(HUD)も退避せずそのまま残す。
                    overlay::minimize_window(m.window());
                    return;
                }
                mv.set(false);
                let _ = m.hide();
            }
            // トレイ格納: メイン最小化にオーバーレイも追従して退避（設定フラグは変更しない）
            if let Some(o) = self_ov.upgrade() {
                let _ = o.hide();
            }
            if let Some(o) = buff_ov.upgrade() {
                let _ = o.hide();
            }
        });
    }
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
    // タブ選択: 共有セルを更新し、即時に再取得して反映（ポーリング待ちにしない）。
    // タブ切替時はドリルダウン/内訳ビューを解除して一覧へ戻す。
    {
        let w = main.as_weak();
        let enc_sel = enc.clone();
        let rows_sel = rows.clone();
        let tab_sel = tab_cell.clone();
        let cfg_sel = cfg.clone();
        let wl_sel = wl.clone();
        let drill_sel = drill.clone();
        let hist_rows_sel = history_rows.clone();
        let hist_exp_sel = history_expanded.clone();
        let hist_player_exp_sel = history_player_expanded.clone();
        let cl_sel = compact_left.clone();
        let cr_sel = compact_right.clone();
        main.on_select_tab(move |n| {
            tab_sel.set(n);
            drill_sel.set(Drill::None);
            if let Some(m) = w.upgrade() {
                m.set_tab(n);
                m.set_view(0);
                // 行は下のif/elseで即時再構築するが、合計行/ヘッダーの値もここで揃えないと
                // 次のpollまで前タブの合計DPS/経過が残ってしまう（issue #9 PR1レビュー対応）。
                refresh_header(&m, &enc_sel, n);
                if n == 3 {
                    let hist = compute::get_history();
                    hist_rows_sel.apply(build_history_rows(
                        &hist,
                        hist_exp_sel.get(),
                        hist_player_exp_sel.get(),
                        &cfg_sel.borrow(),
                    ));
                } else {
                    let c = cfg_sel.borrow();
                    m.set_show_graph_col(graph_col_active(&c, n));
                    let pw = fetch_players(&enc_sel, n);
                    let main_uids: Vec<i64> =
                        pw.player_rows.iter().map(|p| p.uid as i64).collect();
                    // 専用モード時はメイン一覧自体が空のため、ここでは常に非専用扱いでよい
                    // （main_uids が空なら結果も空集合になるだけ）。
                    let pin_uids = timer_roster(
                        &wl_sel.borrow(),
                        false,
                        c.sync_timer_with_main,
                        c.sync_order_follow,
                        &main_uids,
                        &[],
                        pw.local_player_uid as i64,
                    );
                    apply_player_rows(
                        &m,
                        &rows_sel,
                        &cl_sel,
                        &cr_sel,
                        build_rows(
                            &pw,
                            &c.name_template,
                            c.abbreviate_scores,
                            c.privacy_mask_names,
                            &pin_uids,
                            c.graph_player_count as i32,
                            c.graph_for_local_player,
                            &dps_bar_config(&c),
                        ),
                    );
                }
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
                match compute::get_skills(&enc_sk, uid, tab_stat(tab_h.get())) {
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
        let cl_t = compact_left.clone();
        let cr_t = compact_right.clone();
        main.on_toggle_watch(move |uid_str| {
            let uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            if uid == 0 {
                return;
            }
            let sync = cfg_t.borrow().sync_timer_with_main;
            {
                let mut wl = wl_t.borrow_mut();
                // 同期ON: ピンは「タイマーから隠す/表示」(excluded の出し入れ)。
                // 同期OFF: 従来の手動ウォッチ(watched の出し入れ)。
                if sync {
                    wl.toggle_excluded(uid);
                } else {
                    wl.toggle(uid);
                }
                wl.save();
            }
            if let Some(m) = w.upgrade() {
                let c = cfg_t.borrow();
                let pw = fetch_players(&enc_t, tab_t.get());
                let main_uids: Vec<i64> =
                    pw.player_rows.iter().map(|p| p.uid as i64).collect();
                let pin_uids = timer_roster(
                    &wl_t.borrow(),
                    false,
                    c.sync_timer_with_main,
                    c.sync_order_follow,
                    &main_uids,
                    &[],
                    pw.local_player_uid as i64,
                );
                apply_player_rows(
                    &m,
                    &rows_t,
                    &cl_t,
                    &cr_t,
                    build_rows(
                        &pw,
                        &c.name_template,
                        c.abbreviate_scores,
                        c.privacy_mask_names,
                        &pin_uids,
                        c.graph_player_count as i32,
                        c.graph_for_local_player,
                        &dps_bar_config(&c),
                    ),
                );
            }
        });
    }
    // ウォッチ一括クリア（手動運用＝同期OFF時のみ設定UIに導線あり）。
    // watched・excluded を両方消し、過去の幽霊エントリ（離脱済プレイヤー等）を掃除する。
    {
        let w = main.as_weak();
        let wl_cw = wl.clone();
        let enc_cw = enc.clone();
        let rows_cw = rows.clone();
        let cfg_cw = cfg.clone();
        let tab_cw = tab_cell.clone();
        let cl_cw = compact_left.clone();
        let cr_cw = compact_right.clone();
        main.on_clear_watchlist(move || {
            wl_cw.borrow_mut().clear_all();
            wl_cw.borrow().save();
            if let Some(m) = w.upgrade() {
                let c = cfg_cw.borrow();
                let pw = fetch_players(&enc_cw, tab_cw.get());
                let main_uids: Vec<i64> = pw.player_rows.iter().map(|p| p.uid as i64).collect();
                let pin_uids = timer_roster(
                    &wl_cw.borrow(),
                    false,
                    c.sync_timer_with_main,
                    c.sync_order_follow,
                    &main_uids,
                    &[],
                    pw.local_player_uid as i64,
                );
                apply_player_rows(
                    &m,
                    &rows_cw,
                    &cl_cw,
                    &cr_cw,
                    build_rows(
                        &pw,
                        &c.name_template,
                        c.abbreviate_scores,
                        c.privacy_mask_names,
                        &pin_uids,
                        c.graph_player_count as i32,
                        c.graph_for_local_player,
                        &dps_bar_config(&c),
                    ),
                );
            }
        });
    }
    // 設定パネルの開閉。開く瞬間にテンプレ入力欄・自キャラUID欄・ショートカット一覧へ最新値を push。
    {
        let w = main.as_weak();
        let cfg_ts = cfg.clone();
        let enc_ts = enc.clone();
        let cands_ts = uid_candidates.clone();
        #[cfg(windows)]
        let shortcuts_ts = shortcuts_model.clone();
        #[cfg(windows)]
        let hotkeys_ts = hotkeys_holder.clone();
        main.on_toggle_settings(move || {
            if let Some(m) = w.upgrade() {
                let opening = !m.get_settings_open();
                if opening {
                    refresh_settings_inputs(&m, &cfg_ts.borrow());
                    refresh_selected_uid(&m, &enc_ts, &cands_ts);
                    #[cfg(windows)]
                    push_shortcuts_to_ui(&shortcuts_ts, &hotkeys_ts.borrow(), &cfg_ts.borrow());
                }
                m.set_settings_open(opening);
            }
        });
    }
    // 「別ウィンドウで開く」ボタン→ イマジン名メンテ窓の絞り込み値/一覧を最新化して表示する。
    {
        let iw = imagine_window.as_weak();
        let imagine_rows_ow = imagine_rows_model.clone();
        let imagine_expanded_ow = imagine_expanded.clone();
        let imagine_filter_ow = imagine_filter.clone();
        let imagine_window_sized_ow = imagine_window_sized.clone();
        main.on_imagine_open_window(move || {
            if let Some(w) = iw.upgrade() {
                w.set_imagine_filter_value(imagine_filter_ow.borrow().clone().into());
                // 前回の反映結果を再開時まで残すと紛らわしいためクリアする。
                w.set_imagine_push_status("".into());
                rebuild_imagine_rows(
                    &imagine_rows_ow,
                    &imagine_expanded_ow.borrow(),
                    &imagine_filter_ow.borrow(),
                );
                let _ = w.show();
                if !imagine_window_sized_ow.get() {
                    window_state::enforce_size(
                        w.window(),
                        &window_state::WinRect {
                            x: 0,
                            y: 0,
                            w: 620,
                            h: 520,
                        },
                    );
                    imagine_window_sized_ow.set(true);
                }
            }
        });
    }
    // イマジン名一覧（イマジン名メンテ窓）: 展開/表示名/IGNORE/リセット/フィルタ（誰でも）
    // ＋devリネーム/DB反映（devのみ）。
    {
        let imagine_rows_ie = imagine_rows_model.clone();
        let imagine_expanded_ie = imagine_expanded.clone();
        let imagine_filter_ie = imagine_filter.clone();
        imagine_window.on_imagine_toggle_expand(move |canonical| {
            {
                let mut set = imagine_expanded_ie.borrow_mut();
                if !set.remove(canonical.as_str()) {
                    set.insert(canonical.to_string());
                }
            }
            rebuild_imagine_rows(
                &imagine_rows_ie,
                &imagine_expanded_ie.borrow(),
                &imagine_filter_ie.borrow(),
            );
        });
    }
    {
        // 表示名の編集は永続化のみ行いモデルを rebuild しない（TemplateField と同じ方針。
        // TextInput の text: バインドを編集の都度再送するとカーソル位置がクロバーされるため）。
        imagine_window.on_imagine_set_display(move |canonical, val| {
            let display = (!val.is_empty()).then(|| val.to_string());
            engine::imagine_overrides::set_display(canonical.as_str(), display);
        });
    }
    {
        let imagine_rows_ii = imagine_rows_model.clone();
        let imagine_expanded_ii = imagine_expanded.clone();
        let imagine_filter_ii = imagine_filter.clone();
        imagine_window.on_imagine_toggle_ignore(move |canonical| {
            let current =
                engine::imagine_overrides::get(canonical.as_str()).is_some_and(|o| o.ignored);
            engine::imagine_overrides::set_ignored(canonical.as_str(), !current);
            rebuild_imagine_rows(
                &imagine_rows_ii,
                &imagine_expanded_ii.borrow(),
                &imagine_filter_ii.borrow(),
            );
        });
    }
    {
        let imagine_rows_ir = imagine_rows_model.clone();
        let imagine_expanded_ir = imagine_expanded.clone();
        let imagine_filter_ir = imagine_filter.clone();
        imagine_window.on_imagine_reset(move |canonical| {
            engine::imagine_overrides::clear(canonical.as_str());
            rebuild_imagine_rows(
                &imagine_rows_ir,
                &imagine_expanded_ir.borrow(),
                &imagine_filter_ir.borrow(),
            );
        });
    }
    {
        // devのみ有効。イマジン名(ja)の書き換えは canonical(=グループの識別キー)を変えるため、
        // rebuild で一覧・展開状態キーを追随させる（旧canonicalの展開状態は新canonicalへ引き継ぐ）。
        let imagine_rows_in = imagine_rows_model.clone();
        let imagine_expanded_in = imagine_expanded.clone();
        let imagine_filter_in = imagine_filter.clone();
        imagine_window.on_imagine_set_name(move |canonical, val| {
            if !imagine_dev_mode {
                return;
            }
            let new_name_ja = (!val.is_empty()).then(|| val.to_string());
            engine::imagine_skills::dev_rename_entry(canonical.as_str(), new_name_ja.clone(), None);
            if let Some(new_canonical) = new_name_ja {
                let mut set = imagine_expanded_in.borrow_mut();
                if set.remove(canonical.as_str()) {
                    set.insert(new_canonical);
                }
            }
            rebuild_imagine_rows(
                &imagine_rows_in,
                &imagine_expanded_in.borrow(),
                &imagine_filter_in.borrow(),
            );
        });
    }
    {
        // devのみ有効。git push は資格情報プロンプト/低速回線/pre-pushフックでブロックし得る
        // ネットワークI/O（他のローカルgit操作と違いUIスレッドで同期実行すると凍結し得る）ため、
        // 別スレッドで実行し、結果は invoke_from_event_loop 経由でUIスレッドへ反映する。
        // Weak<ImagineNamesWindow> は Send のためスレッドへ move できる。
        let w = imagine_window.as_weak();
        imagine_window.on_imagine_push_db(move || {
            if !imagine_dev_mode {
                return;
            }
            if let Some(m) = w.upgrade() {
                m.set_imagine_push_status(if is_ja() { "反映中…" } else { "Pushing…" }.into());
            }
            let w_bg = w.clone();
            std::thread::spawn(move || {
                let status = push_imagine_db();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(m) = w_bg.upgrade() {
                        m.set_imagine_push_status(status.into());
                    }
                });
            });
        });
    }
    {
        let imagine_rows_if = imagine_rows_model.clone();
        let imagine_expanded_if = imagine_expanded.clone();
        let imagine_filter_if = imagine_filter.clone();
        imagine_window.on_imagine_set_filter(move |val| {
            *imagine_filter_if.borrow_mut() = val.to_string();
            rebuild_imagine_rows(
                &imagine_rows_if,
                &imagine_expanded_if.borrow(),
                &imagine_filter_if.borrow(),
            );
        });
    }
    // グローバルショートカット（issue #3）: ダイアログの開閉・キャプチャ・解除・キー入力。
    // 設定パネルのボタンから開くモーダル（別ウィンドウではない）。
    // ダイアログを開いている間は割当済みキーが RegisterHotKey にシステム側で吸収され
    // FocusScope へ届かない（＝再割当できない・代わりに本番アクションが誤発火する）ため、
    // 開いている間は全ホットキーの登録を一時解除し、閉じたら復帰させる（C1）。
    #[cfg(windows)]
    {
        let w = main.as_weak();
        let hotkeys_od = hotkeys_holder.clone();
        main.on_shortcut_open_dialog(move || {
            if let Some(m) = w.upgrade() {
                m.set_shortcut_dialog_open(true);
            }
            if let Some(hk) = hotkeys_od.borrow_mut().as_mut() {
                hk.suspend();
            }
        });
    }
    #[cfg(windows)]
    {
        let w = main.as_weak();
        let cfg_cd = cfg.clone();
        let hotkeys_cd = hotkeys_holder.clone();
        let shortcuts_cd = shortcuts_model.clone();
        main.on_shortcut_close_dialog(move || {
            if let Some(m) = w.upgrade() {
                m.set_shortcut_dialog_open(false);
                // ダイアログを閉じたら進行中のキャプチャも打ち切る（元の割当は変更されない）。
                m.set_shortcut_capturing(-1);
            }
            // open 側で suspend() した分をここで復帰させる。
            if let Some(hk) = hotkeys_cd.borrow_mut().as_mut() {
                hk.apply(&cfg_cd.borrow());
            }
            push_shortcuts_to_ui(&shortcuts_cd, &hotkeys_cd.borrow(), &cfg_cd.borrow());
        });
    }
    // 「変更」押下でキャプチャ開始。前回の一時エラー表示が残っていればクリアする。
    #[cfg(windows)]
    {
        let w = main.as_weak();
        let shortcuts_sc = shortcuts_model.clone();
        main.on_shortcut_capture(move |idx| {
            if let Some(m) = w.upgrade() {
                m.set_shortcut_capturing(idx);
            }
            // 前回キャプチャの一時エラー（修飾キー無し/重複等）が対象を切り替えても
            // 別行に残ったままにならないよう、対象行だけでなく全行を掃除する。
            for i in 0..shortcuts_sc.row_count() {
                if let Some(mut row) = shortcuts_sc.row_data(i) {
                    if !row.error.is_empty() {
                        row.error = "".into();
                        shortcuts_sc.set_row_data(i, row);
                    }
                }
            }
        });
    }
    // 「解除」押下で当該アクションのキーを空にして保存・エラー再計算・UI反映。
    // ダイアログを開いた時点で suspend 済みなので、ここでは revalidate() のみ（apply() は
    // 呼ばない。呼ぶとダイアログ表示中に全ホットキーが再登録されてしまう。C1）。
    // 実際の再登録はダイアログを閉じる（shortcut-close-dialog）まで行わない。
    #[cfg(windows)]
    {
        let cfg_scl = cfg.clone();
        let hotkeys_scl = hotkeys_holder.clone();
        let shortcuts_scl = shortcuts_model.clone();
        main.on_shortcut_clear(move |idx| {
            let Some(action) = hotkey::ShortcutAction::from_index(idx as usize) else {
                return;
            };
            {
                let mut c = cfg_scl.borrow_mut();
                action.set_key_text(&mut c, String::new());
            }
            settings::save(&cfg_scl.borrow());
            if let Some(hk) = hotkeys_scl.borrow_mut().as_mut() {
                hk.revalidate(&cfg_scl.borrow());
            }
            push_shortcuts_to_ui(&shortcuts_scl, &hotkeys_scl.borrow(), &cfg_scl.borrow());
        });
    }
    // 「修飾キーなしでも割り当てを許可」の切替（issue #8）。ON にする前の確認モーダルは
    // .slint 側（solo-confirm-open）が担当し、ここへは同意後の値だけが来る。
    // ダイアログを開いた時点で suspend 済みなので revalidate() のみ（apply() は閉じる時。C1）。
    // OFF に戻したときは、許可中に保存された単独キーが MissingModifier になり登録されない
    // （設定値は消さないので、再度 ON にすればそのまま復帰する）。
    #[cfg(windows)]
    {
        let w = main.as_weak();
        let cfg_as = cfg.clone();
        let hotkeys_as = hotkeys_holder.clone();
        let shortcuts_as = shortcuts_model.clone();
        main.on_shortcut_set_allow_solo(move |val| {
            cfg_as.borrow_mut().allow_solo_hotkeys = val;
            settings::save(&cfg_as.borrow());
            if let Some(hk) = hotkeys_as.borrow_mut().as_mut() {
                hk.revalidate(&cfg_as.borrow());
            }
            if let Some(m) = w.upgrade() {
                // トグルの表示状態（cfg.allow-solo-hotkeys）と説明文を更新する。
                apply_settings(&m, &cfg_as.borrow());
            }
            push_shortcuts_to_ui(&shortcuts_as, &hotkeys_as.borrow(), &cfg_as.borrow());
        });
    }
    // FocusScope の key-pressed から渡される1キー。キャプチャ中のみ判定する
    // （shortcut-capturing が -1 なら待機していないので無視）。
    #[cfg(windows)]
    {
        let w = main.as_weak();
        let cfg_sk = cfg.clone();
        let hotkeys_sk = hotkeys_holder.clone();
        let shortcuts_sk = shortcuts_model.clone();
        main.on_shortcut_key(move |text, ctrl, shift, alt, meta, repeat| {
            let Some(m) = w.upgrade() else {
                return;
            };
            let idx = m.get_shortcut_capturing();
            if idx < 0 {
                return;
            }
            let Some(action) = hotkey::ShortcutAction::from_index(idx as usize) else {
                return;
            };
            let allow_solo = cfg_sk.borrow().allow_solo_hotkeys;
            match hotkey::capture_key(text.as_str(), ctrl, shift, alt, meta, repeat, allow_solo) {
                hotkey::CaptureOutcome::Continue(err) => {
                    // None(無視するキー)は何もしない。Some はエラー文言を出しつつ待機継続。
                    if let Some(msg) = err {
                        if let Some(mut row) = shortcuts_sk.row_data(idx as usize) {
                            row.error = msg.into();
                            shortcuts_sk.set_row_data(idx as usize, row);
                        }
                    }
                }
                hotkey::CaptureOutcome::Cancelled => {
                    m.set_shortcut_capturing(-1);
                    // 直前の一時エラー（修飾キー無し/重複等）が残ったままにならないようクリアする。
                    if let Some(mut row) = shortcuts_sk.row_data(idx as usize) {
                        row.error = "".into();
                        shortcuts_sk.set_row_data(idx as usize, row);
                    }
                }
                hotkey::CaptureOutcome::Captured(key_text) => {
                    // 自プロセス内でも RegisterHotKey は同一キーの重複登録に失敗するため、
                    // 保存前にアプリ内重複を検出して分かりやすく伝える（採用しない＝待機継続）。
                    if hotkey::is_duplicate(&cfg_sk.borrow(), action, &key_text) {
                        if let Some(mut row) = shortcuts_sk.row_data(idx as usize) {
                            row.error = hotkey::msg_duplicate().into();
                            shortcuts_sk.set_row_data(idx as usize, row);
                        }
                        return;
                    }
                    {
                        let mut c = cfg_sk.borrow_mut();
                        action.set_key_text(&mut c, key_text);
                    }
                    settings::save(&cfg_sk.borrow());
                    // suspend 状態のまま静的エラーだけ再計算する（apply() は呼ばない。
                    // ダイアログ表示中に全ホットキーが再登録されるのを防ぐ。C1）。
                    if let Some(hk) = hotkeys_sk.borrow_mut().as_mut() {
                        hk.revalidate(&cfg_sk.borrow());
                    }
                    m.set_shortcut_capturing(-1);
                    push_shortcuts_to_ui(&shortcuts_sk, &hotkeys_sk.borrow(), &cfg_sk.borrow());
                }
            }
        });
    }
    // 設定トグル変更 → cfg 更新・即適用・保存
    {
        let w = main.as_weak();
        let cfg_b = cfg.clone();
        let enc_sb = enc.clone();
        let self_ov = self_overlay.as_weak();
        let buff_ov = buff_overlay.as_weak();
        let stats_ov = stats_overlay.as_weak();
        let stat_catalog_left_sb = stat_catalog_left.clone();
        let stat_catalog_right_sb = stat_catalog_right.clone();
        let taskbar_flag_cb = taskbar_flag.clone();
        main.on_set_bool(move |key, val| {
            {
                let mut c = cfg_b.borrow_mut();
                match key.as_str() {
                    "self-status-overlay" => c.show_self_status_overlay = val,
                    "stats-overlay" => c.show_stats_overlay = val,
                    // ステータス窓の表示項目トグル（カタログ順で並べ直して安定化）
                    k if k.starts_with("stat.") => {
                        let key = k.trim_start_matches("stat.").to_string();
                        let mut set: std::collections::HashSet<String> =
                            c.stats_enabled.iter().cloned().collect();
                        if val {
                            set.insert(key);
                        } else {
                            set.remove(&key);
                        }
                        c.stats_enabled = settings::STAT_CATALOG
                            .iter()
                            .map(|d| d.key.to_string())
                            .filter(|k| set.contains(k))
                            .collect();
                    }
                    // ステータス表示項目の一括ON/OFF（カタログ全項目を1クリックで切替）
                    "stats-all-on" => {
                        c.stats_enabled = settings::STAT_CATALOG
                            .iter()
                            .map(|d| d.key.to_string())
                            .collect();
                    }
                    "stats-all-off" => c.stats_enabled.clear(),
                    "buff-overlay" => c.show_buff_overlay = val,
                    // 専用モードON時はイマジンタイマーを強制表示（旧UIと同挙動）。
                    // 専用モードは集計を早期returnし軽量化する仕組みのため、導出元(メインDPS一覧)
                    // が空になる「メインDPSと同期」とは相互排他（ONにしたら他方を自動OFF）。
                    "imagine-only" => {
                        c.imagine_only_mode = val;
                        if val {
                            c.show_buff_overlay = true;
                            c.sync_timer_with_main = false;
                        }
                    }
                    "show-crit" => c.show_crit = val,
                    "show-crit-value" => c.show_crit_value = val,
                    "show-lucky" => c.show_lucky = val,
                    "show-lucky-value" => c.show_lucky_value = val,
                    "show-hits" => c.show_hits = val,
                    "show-hpm" => c.show_hpm = val,
                    "show-score" => c.show_score = val,
                    "show-eff-dps" => c.show_eff_dps = val,
                    "highlight-local" => c.highlight_local_player = val,
                    "abbreviate-scores" => c.abbreviate_scores = val,
                    "privacy-mask" => c.privacy_mask_names = val,
                    "aot" => c.always_on_top = val,
                    "three-min-auto-open" => c.three_min_auto_open = val,
                    "compact-split" => c.compact_split_mode = val,
                    "graph-for-local" => c.graph_for_local_player = val,
                    // imagine_only_mode と相互排他（ONにしたら専用モードを自動OFF）。
                    "sync-timer-with-main" => {
                        c.sync_timer_with_main = val;
                        if val {
                            c.imagine_only_mode = false;
                        }
                    }
                    "sync-order-follow" => c.sync_order_follow = val,
                    "imagine-col-tina" => c.show_imagine_tina = val,
                    "imagine-col-aluna" => c.show_imagine_aluna = val,
                    "imagine-col-tarta" => c.show_imagine_tarta = val,
                    "imagine-col-basilisk" => c.show_imagine_basilisk = val,
                    "imagine-col-kartgriff" => c.show_imagine_kartgriff = val,
                    "imagine-compact-rows" => c.imagine_compact_rows = val,
                    "show-consumable" => c.show_consumable = val,
                    "party-only-consumables" => c.party_only_consumables = val,
                    "measure-self-only" => c.measure_self_only = val,
                    "measure-first-target-only" => c.measure_first_target_only = val,
                    "show-in-taskbar" => c.show_in_taskbar = val,
                    "main-font-bold" => c.main_font_bold = val,
                    "stats-overlay-font-bold" => c.stats_overlay_font_bold = val,
                    "imagine-overlay-font-bold" => c.imagine_overlay_font_bold = val,
                    "buff-overlay-font-bold" => c.buff_overlay_font_bold = val,
                    "overlay-outline" => c.overlay_outline = val,
                    "overlay-shadow" => c.overlay_shadow = val,
                    "show-footer" => c.show_footer = val,
                    "show-total-row" => c.show_total_row = val,
                    "dps-bar-animate" => c.dps_bar_animate = val,
                    "check-update-on-startup" => c.check_update_on_startup = val,
                    other => log::warn!("unknown setting key: {other}"),
                }
            }
            let c = cfg_b.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
            }
            // タスクバー常駐⇔トレイ格納を全ウィンドウへ即時反映（再起動不要）。
            // 共有フラグも更新し、以降に再生成されるウィンドウへも引き継ぐ。
            if key.as_str() == "show-in-taskbar" {
                taskbar_flag_cb.set(c.show_in_taskbar);
                // オーバーレイの最小化ボタン表示を切替（トレイ格納時はOS最小化で復帰口が無いため隠す）
                if let Some(o) = self_ov.upgrade() {
                    o.set_show_minimize(c.show_in_taskbar);
                }
                if let Some(o) = buff_ov.upgrade() {
                    o.set_show_minimize(c.show_in_taskbar);
                }
                if let Some(o) = stats_ov.upgrade() {
                    o.set_show_minimize(c.show_in_taskbar);
                }
                #[cfg(windows)]
                {
                    let show = c.show_in_taskbar;
                    if let Some(m) = w.upgrade() {
                        overlay::apply_taskbar_mode(m.window(), show);
                    }
                    if let Some(o) = self_ov.upgrade() {
                        overlay::apply_taskbar_mode(o.window(), show);
                    }
                    if let Some(o) = buff_ov.upgrade() {
                        overlay::apply_taskbar_mode(o.window(), show);
                    }
                    if let Some(o) = stats_ov.upgrade() {
                        overlay::apply_taskbar_mode(o.window(), show);
                    }
                }
            }
            // ステータス表示項目のトグル・一括切替はカタログモデルを再生成してチェック状態へ反映。
            if key.as_str().starts_with("stat.")
                || key.as_str() == "stats-all-on"
                || key.as_str() == "stats-all-off"
            {
                let (l, r) = split_stat_catalog(&build_stat_catalog(&c.stats_enabled));
                stat_catalog_left_sb.set_vec(l);
                stat_catalog_right_sb.set_vec(r);
            }
            // フォント太字はオーバーレイ外観へ即反映。
            if matches!(
                key.as_str(),
                "main-font-bold"
                    | "stats-overlay-font-bold"
                    | "imagine-overlay-font-bold"
                    | "buff-overlay-font-bold"
                    | "overlay-outline"
                    | "overlay-shadow"
            ) {
                apply_overlay_appearance(&c, &self_ov, &buff_ov, &stats_ov);
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
            if key.as_str() == "stats-overlay" {
                if let Some(o) = stats_ov.upgrade() {
                    if c.show_stats_overlay {
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
            if key.as_str() == "party-only-consumables" {
                engine::runtime_settings::set_party_only_consumables(c.party_only_consumables);
            }
            // sync-timer-with-main 側からの排他連動でも imagine_only_mode が変わるため、
            // どちらのキー経由でも compute 側の状態を実際の値へ整合させる。
            if matches!(key.as_str(), "imagine-only" | "sync-timer-with-main") {
                compute::set_imagine_only_mode(&enc_sb, c.imagine_only_mode);
                // 専用モードON時は強制表示にした buff overlay を実際に出す
                if c.imagine_only_mode && c.show_buff_overlay {
                    if let Some(o) = buff_ov.upgrade() {
                        let _ = o.show();
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
    // オーバーレイ共通の不透明度（メインとは独立）。スライダー操作で全オーバーレイへ即反映。
    {
        let w = main.as_weak();
        let cfg_o = cfg.clone();
        let self_o = self_overlay.as_weak();
        let buff_o = buff_overlay.as_weak();
        let stats_o = stats_overlay.as_weak();
        main.on_set_overlay_opacity(move |v| {
            // オーバーレイのみ完全透明(0)を許可。スライダー左端付近(<0.04)は0へスナップして
            // 「完全透明＝クリック透過」をはっきり選べるようにする。それ以外は下限0.04。
            let clamped: f64 = if v < 0.04 { 0.0 } else { v.clamp(0.04, 1.0) as f64 };
            cfg_o.borrow_mut().overlay_opacity = clamped;
            if let Some(m) = w.upgrade() {
                m.set_overlay_opacity(clamped as f32);
            }
            apply_overlay_appearance(&cfg_o.borrow(), &self_o, &buff_o, &stats_o);
            settings::save(&cfg_o.borrow());
        });
    }
    // 文字色 HSV ピッカー操作。h/s/v(各0..1)を hex 化して保存し、全オーバーレイへ即反映。
    {
        let w = main.as_weak();
        let cfg_p = cfg.clone();
        let self_o = self_overlay.as_weak();
        let buff_o = buff_overlay.as_weak();
        let stats_o = stats_overlay.as_weak();
        main.on_pick_overlay_text(move |h, s, v| {
            cfg_p.borrow_mut().overlay_text_color = hsv_to_hex(h, s, v);
            let c = cfg_p.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
                // ドラッグ追従を滑らかに: h/s/v は入力値そのままで上書き（hex 量子化の戻りで跳ねさせない）
                m.set_overlay_text_h(h);
                m.set_overlay_text_s(s);
                m.set_overlay_text_v(v);
            }
            apply_overlay_appearance(&c, &self_o, &buff_o, &stats_o);
            settings::save(&c);
        });
    }
    // アクセント色 HSV ピッカー操作。h/s/v(各0..1)を hex 化して保存し、Theme へ即反映。
    {
        let w = main.as_weak();
        let cfg_a = cfg.clone();
        main.on_pick_accent(move |h, s, v| {
            cfg_a.borrow_mut().accent_theme = hsv_to_hex(h, s, v);
            let c = cfg_a.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
                // ドラッグ追従を滑らかに: h/s/v は入力値そのままで上書き（hex 量子化の戻りで跳ねさせない）
                m.set_accent_h(h);
                m.set_accent_s(s);
                m.set_accent_v(v);
            }
            settings::save(&c);
        });
    }
    // メインpollタイマーの実体。生成をここへ前出しし、下の on_bump_num の poll-interval
    // 分岐から set_interval で即時反映できるようにする（実際の start() は後段のポーリング
    // ループ構築時）。Timer は Clone 不可のため Rc で共有する（result_countup_timer と同様）。
    let poll_timer: Rc<Timer> = Rc::new(Timer::default());
    // 数値設定ステッパー（key と方向 dir=±1）。キー毎に step/範囲を持ち、必要なら即適用。
    {
        let w = main.as_weak();
        let cfg_n = cfg.clone();
        let self_o = self_overlay.as_weak();
        let buff_o = buff_overlay.as_weak();
        let stats_o = stats_overlay.as_weak();
        let poll_timer_n = poll_timer.clone();
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
                        // タイマー周期へ即時反映（再起動不要。set_interval は Repeated タイマーの
                        // 周期そのものを変える。次回発火は「今から」新周期後に再計算される）。
                        poll_timer_n.set_interval(Duration::from_millis(c.poll_interval_ms.max(50.0) as u64));
                    }
                    "three-min-dur" => {
                        c.three_min_duration_sec = (c.three_min_duration_sec + d * 30.0).clamp(30.0, 1800.0);
                    }
                    "history-limit" => {
                        c.history_limit = (c.history_limit + d * 5.0).clamp(0.0, 100.0);
                        compute::set_history_limit(c.history_limit);
                    }
                    // 保持期間(サンプル数×間隔)を変えると固定基準モードの平均窓秒の上限
                    // (dps_bar::max_window_secs)も変わるため、既存の指定値を都度クランプし直す
                    // （表示欄は下の apply_settings 後に push_dps_bar_inputs で追従させる）。
                    "ts-samples" => {
                        c.time_series_samples = (c.time_series_samples + d * 10.0).clamp(10.0, 200.0);
                        compute::set_time_series_config(c.time_series_samples, c.time_series_interval_ms);
                        c.dps_bar_window_secs = dps_bar::clamp_window_secs(
                            c.dps_bar_window_secs,
                            c.time_series_samples,
                            c.time_series_interval_ms,
                        );
                    }
                    "ts-interval" => {
                        c.time_series_interval_ms = (c.time_series_interval_ms + d * 250.0).clamp(250.0, 5000.0);
                        compute::set_time_series_config(c.time_series_samples, c.time_series_interval_ms);
                        c.dps_bar_window_secs = dps_bar::clamp_window_secs(
                            c.dps_bar_window_secs,
                            c.time_series_samples,
                            c.time_series_interval_ms,
                        );
                    }
                    "graph-count" => {
                        c.graph_player_count = (c.graph_player_count + d).clamp(0.0, 10.0);
                    }
                    "font-size" => {
                        c.font_size = (c.font_size + d).clamp(10.0, 18.0);
                    }
                    "stats-overlay-font-size" => {
                        c.stats_overlay_font_size = (c.stats_overlay_font_size + d).clamp(8.0, 100.0);
                    }
                    "imagine-overlay-font-size" => {
                        c.imagine_overlay_font_size =
                            (c.imagine_overlay_font_size + d).clamp(8.0, 24.0);
                    }
                    "buff-overlay-font-size" => {
                        c.buff_overlay_font_size = (c.buff_overlay_font_size + d).clamp(8.0, 100.0);
                    }
                    other => log::warn!("unknown num key: {other}"),
                }
            }
            let c = cfg_n.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
                // ts-samples/ts-interval は保持期間経由で dps_bar_window_secs を再クランプしうる
                // ため、表示欄も実効値へ同期する（他キーは無関係な上書きになるため対象外）。
                if key.as_str() == "ts-samples" || key.as_str() == "ts-interval" {
                    push_dps_bar_inputs(&m, &c);
                }
            }
            apply_overlay_appearance(&c, &self_o, &buff_o, &stats_o);
            settings::save(&c);
        });
    }
    // テンプレ編集（edited）。cfg と preview のみ更新（value は push しない＝入力中クロバー防止）。
    {
        let w = main.as_weak();
        let cfg_s = cfg.clone();
        let self_o = self_overlay.as_weak();
        let buff_o = buff_overlay.as_weak();
        let stats_o = stats_overlay.as_weak();
        main.on_set_str(move |key, val| {
            {
                let mut c = cfg_s.borrow_mut();
                match key.as_str() {
                    "name-template" => c.name_template = val.to_string(),
                    "copy-template" => c.copy_template = val.to_string(),
                    "startup-tab" => c.startup_tab = val.to_string(),
                    // Slint 1.16 は @tr リテラルを定数畳み込みするため select_bundled_translation を
                    // ランタイムで呼んでも既存 UI は再翻訳されない。永続化のみ行い、反映は次回起動時
                    // （main.rs 起動時の select_bundled_translation(&cfg.language)）。UI 側で再起動要を明示。
                    "language" => c.language = val.to_string(),
                    "accent-theme" => c.accent_theme = val.to_string(),
                    "main-font" => c.main_font = val.to_string(),
                    "stats-overlay-font" => c.stats_overlay_font = val.to_string(),
                    "imagine-overlay-font" => c.imagine_overlay_font = val.to_string(),
                    "buff-overlay-font" => c.buff_overlay_font = val.to_string(),
                    "overlay-text-color" => c.overlay_text_color = val.to_string(),
                    "dps-bar-mode" => c.dps_bar_mode = val.to_string(),
                    "dps-bar-intensity" => c.dps_bar_intensity = val.to_string(),
                    // 不正入力（非数・0以下）は保存せず直前の値を維持する（SegButton の値送出とは
                    // 異なりユーザーの自由入力のため、ここで検証する）。検証・クランプは
                    // crate::dps_bar に一本化し、settings::load() の正規化処理と共用する。
                    "dps-bar-fixed-max" => {
                        if let Some(v) =
                            val.trim().parse::<f64>().ok().filter(|v| dps_bar::is_positive_finite(*v))
                        {
                            c.dps_bar_fixed_max = v;
                        }
                    }
                    "dps-bar-window-secs" => {
                        if let Some(v) =
                            val.trim().parse::<f64>().ok().filter(|v| dps_bar::is_positive_finite(*v))
                        {
                            c.dps_bar_window_secs = dps_bar::clamp_window_secs(
                                v,
                                c.time_series_samples,
                                c.time_series_interval_ms,
                            );
                        }
                    }
                    other => log::warn!("unknown str key: {other}"),
                }
            }
            let c = cfg_s.borrow();
            if let Some(m) = w.upgrade() {
                // cfg-ui(起動タブ強調)・nums・font-scale を反映。テンプレ value は
                // push されない（apply_settings は触らない）ため入力中もクロバーしない。
                apply_settings(&m, &c);
                let (np, cp) = template_previews(&c);
                m.set_name_preview(np);
                m.set_copy_preview(cp);
            }
            apply_overlay_appearance(&c, &self_o, &buff_o, &stats_o);
            settings::save(&c);
        });
    }
    // バー表示方式の数値入力欄の確定（Enter／フォーカスアウト）。範囲外入力がクランプ・拒否
    // された場合に表示だけ入力値のまま残る（実効値と乖離する）のを防ぐため、実効値へ再同期する。
    {
        let w = main.as_weak();
        let cfg_c = cfg.clone();
        main.on_commit_str(move |key| {
            if let Some(m) = w.upgrade() {
                match key.as_str() {
                    "dps-bar-fixed-max" | "dps-bar-window-secs" => {
                        push_dps_bar_inputs(&m, &cfg_c.borrow());
                    }
                    other => log::warn!("unknown commit key: {other}"),
                }
            }
        });
    }
    // テンプレ リセット。既定値へ戻し、value を push して入力欄も更新する。
    {
        let w = main.as_weak();
        let cfg_r = cfg.clone();
        main.on_reset_str(move |key| {
            {
                let mut c = cfg_r.borrow_mut();
                match key.as_str() {
                    "name-template" => c.name_template = settings::DEFAULT_NAME_TEMPLATE.to_string(),
                    "copy-template" => c.copy_template = settings::DEFAULT_COPY_TEMPLATE.to_string(),
                    other => log::warn!("unknown reset key: {other}"),
                }
            }
            let c = cfg_r.borrow();
            if let Some(m) = w.upgrade() {
                refresh_settings_inputs(&m, &c);
            }
            settings::save(&c);
        });
    }
    // 自キャラUID 確定（空文字=クリア）。set_selected_uid は集計をリセットするため
    // Enter / 候補クリック / クリア の明示操作時のみ呼ぶ。
    {
        let w = main.as_weak();
        let enc_su = enc.clone();
        let cands = uid_candidates.clone();
        main.on_set_selected_uid(move |s| {
            let t = s.as_str().trim();
            let uid: Option<f64> = if t.is_empty() {
                None
            } else {
                t.parse::<f64>().ok().filter(|v| *v > 0.0)
            };
            compute::set_selected_uid(&enc_su, uid);
            if let Some(m) = w.upgrade() {
                refresh_selected_uid(&m, &enc_su, &cands);
            }
        });
    }
    // 一覧コピー（現在タブの行を copy_template で整形して \n 連結→クリップボード）。
    {
        let w = main.as_weak();
        let enc_c = enc.clone();
        let cfg_c = cfg.clone();
        let tab_c = tab_cell.clone();
        main.on_copy_list(move || {
            let pw = fetch_players(&enc_c, tab_c.get());
            if pw.player_rows.is_empty() {
                return;
            }
            let text = {
                let c = cfg_c.borrow();
                pw.player_rows
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        format::format_row_template(
                            &copy_row_data(p, (i + 1) as i32),
                            &c.copy_template,
                            c.abbreviate_scores,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
                Ok(()) => {
                    if let Some(m) = w.upgrade() {
                        m.set_copied(true);
                        let wk = m.as_weak();
                        Timer::single_shot(Duration::from_millis(800), move || {
                            if let Some(m) = wk.upgrade() {
                                m.set_copied(false);
                            }
                        });
                    }
                }
                Err(e) => log::warn!("clipboard copy failed: {e}"),
            }
        });
    }
    // 履歴: 見出し/プレイヤー行クリックで展開トグル（各単一展開）。
    {
        let w = main.as_weak();
        let hr = history_rows.clone();
        let he = history_expanded.clone();
        let hpe = history_player_expanded.clone();
        let cfg_h = cfg.clone();
        main.on_toggle_history(move |key| {
            let key = key.as_str();
            if let Some(id_text) = key.strip_prefix("h:") {
                let id: i64 = id_text.parse().unwrap_or(0);
                if id != 0 {
                    let next = if he.get() == Some(id) { None } else { Some(id) };
                    he.set(next);
                    // エンカウンタを切り替えた/閉じたら、前のプレイヤー内訳も閉じる。
                    if next.is_none() {
                        hpe.set(None);
                    } else if hpe.get().is_some_and(|(snap_id, _)| snap_id != id) {
                        hpe.set(None);
                    }
                }
            } else if let Some(rest) = key.strip_prefix("p:") {
                let mut parts = rest.split(':');
                let id = parts.next().and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                let uid = parts.next().and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                if id != 0 && uid != 0 && he.get() == Some(id) {
                    hpe.set(if hpe.get() == Some((id, uid)) {
                        None
                    } else {
                        Some((id, uid))
                    });
                }
            }
            if w.upgrade().is_some() {
                let hist = compute::get_history();
                hr.apply(build_history_rows(
                    &hist,
                    he.get(),
                    hpe.get(),
                    &cfg_h.borrow(),
                ));
            }
        });
    }
    // 履歴クリア。食事/シロップの表示も一緒に消す。
    {
        let hr = history_rows.clone();
        let he = history_expanded.clone();
        let hpe = history_player_expanded.clone();
        let enc_ch = enc.clone();
        main.on_clear_history(move || {
            compute::clear_history();
            compute::clear_consumables(&enc_ch);
            he.set(None);
            hpe.set(None);
            hr.apply(Vec::new());
        });
    }
    // 3分計測: 通常→開始 / 待機・計測中→キャンセル（確認ダイアログは省略）。
    {
        let enc_m = enc.clone();
        let cfg_m = cfg.clone();
        main.on_toggle_measure(move || {
            let status = compute::get_measure_mode_status(&enc_m);
            if status.kind == "normal" {
                let scope = measure_scope(&cfg_m.borrow());
                let secs = cfg_m.borrow().three_min_duration_sec;
                compute::start_3min_measure_mode(&enc_m, secs, scope);
            } else {
                compute::cancel_3min_measure_mode(&enc_m);
            }
        });
    }
    // 集計の一時停止トグル / 手動リセット
    {
        let enc_p = enc.clone();
        main.on_toggle_pause(move || compute::toggle_pause(&enc_p));
    }
    {
        let enc_r = enc.clone();
        let wl_r = wl.clone();
        main.on_reset_encounter(move || {
            compute::reset_encounter(&enc_r);
            // 旧版同様リセットでウォッチ対象をクリア（excludedは維持）。
            // 自動追加ONなら次tickで再充填される。
            let mut wl = wl_r.borrow_mut();
            wl.clear_watched();
            wl.save();
        });
    }
    // 3分計測 結果パネル: 閉じる
    {
        let w = main.as_weak();
        main.on_close_result(move || {
            if let Some(m) = w.upgrade() {
                m.set_result_open(false);
            }
        });
    }
    // 3分計測 結果パネル: 行クリックでスキル内訳の対象プレイヤーを切替
    {
        let w = main.as_weak();
        let lr = last_result.clone();
        let cs = captured_skills.clone();
        let rr = result_rows.clone();
        let rsr = result_skill_rows.clone();
        let rp = result_pie.clone();
        let rl = result_legend.clone();
        let sel_p = selected_result_player.clone();
        let sel_s = selected_result_skill.clone();
        let cfg_sp = cfg.clone();
        main.on_select_result_player(move |uid_str| {
            let uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            let snap = lr.borrow();
            let Some(snap) = snap.as_ref() else {
                return;
            };
            if let Some(m) = w.upgrade() {
                apply_result_selection(
                    &m,
                    uid,
                    snap,
                    &cs.borrow(),
                    &rr,
                    &rsr,
                    &rp,
                    &rl,
                    &sel_p,
                    &sel_s,
                    cfg_sp.borrow().privacy_mask_names,
                );
            }
        });
    }
    // 3分計測 結果パネル: スキル行クリックでエリア2 折れ線の対象スキルを切替
    {
        let w = main.as_weak();
        let cs = captured_skills.clone();
        let rsr = result_skill_rows.clone();
        let sel_p = selected_result_player.clone();
        let sel_s = selected_result_skill.clone();
        main.on_select_result_skill(move |uid_str| {
            let skill_uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            sel_s.set(skill_uid);
            let captured = cs.borrow();
            let empty = Vec::new();
            let skills = captured.get(&sel_p.get()).unwrap_or(&empty);
            if let Some(m) = w.upgrade() {
                let dur = m.get_result_duration_ms() as f64;
                apply_result_skill_selection(&m, skills, skill_uid, &rsr, dur);
            }
        });
    }
    // 3分計測 結果パネル: 上位10行を copy_template でコピー
    {
        let lr = last_result.clone();
        let cfg_cr = cfg.clone();
        main.on_copy_result(move || {
            let snap = lr.borrow();
            let Some(snap) = snap.as_ref() else {
                return;
            };
            let text = {
                let c = cfg_cr.borrow();
                snap.player_rows
                    .iter()
                    .take(10)
                    .enumerate()
                    .map(|(i, p)| {
                        format::format_row_template(
                            &copy_row_data(p, (i + 1) as i32),
                            &c.copy_template,
                            c.abbreviate_scores,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            if let Err(e) = arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
                log::warn!("clipboard copy (result) failed: {e}");
            }
        });
    }
    // 3分計測 結果パネル: 閉じて再計測を開始
    {
        let w = main.as_weak();
        let enc_rm = enc.clone();
        let cfg_rm = cfg.clone();
        main.on_restart_measure(move || {
            if let Some(m) = w.upgrade() {
                m.set_result_open(false);
            }
            let scope = measure_scope(&cfg_rm.borrow());
            let secs = cfg_rm.borrow().three_min_duration_sec;
            compute::start_3min_measure_mode(&enc_rm, secs, scope);
        });
    }
    // 結果画面の画像コピー（ウィンドウのスナップショット→モーダル矩形へクロップ→クリップボード）。
    // ウィンドウは半透明合成のため α=255 を強制しないと貼り付け先で透ける。
    {
        let w = main.as_weak();
        let result_countup_timer_ci = result_countup_timer.clone();
        let result_countup_final_ci = result_countup_final.clone();
        main.on_copy_result_image(move || {
            let Some(m) = w.upgrade() else { return };
            // カウントアップ演出の途中(計測確定から500ms以内)にコピーされると小さい値が
            // 画像に焼き込まれてしまうため、snapshot 前に必ず最終値へ即完了させる。
            finish_result_countup(&m, &result_countup_timer_ci, &result_countup_final_ci);
            let win = m.window();
            let snap = match win.take_snapshot() {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("result image: snapshot failed: {e}");
                    return;
                }
            };
            // モーダルは論理 12px マージン（app.slint の結果モーダル矩形と一致させる）
            let margin = (12.0 * win.scale_factor()).round() as usize;
            let (full_w, full_h) = (snap.width() as usize, snap.height() as usize);
            let crop_w = full_w.saturating_sub(margin * 2);
            let crop_h = full_h.saturating_sub(margin * 2);
            if crop_w == 0 || crop_h == 0 {
                log::warn!("result image: window too small to crop ({full_w}x{full_h})");
                return;
            }
            let src = snap.as_slice();
            let mut bytes = Vec::with_capacity(crop_w * crop_h * 4);
            for row in margin..margin + crop_h {
                let line = &src[row * full_w + margin..row * full_w + margin + crop_w];
                for px in line {
                    bytes.extend_from_slice(&[px.r, px.g, px.b, 255]);
                }
            }
            let img = arboard::ImageData {
                width: crop_w,
                height: crop_h,
                bytes: bytes.into(),
            };
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_image(img)) {
                Ok(()) => {
                    m.set_result_img_copied(true);
                    let wk = m.as_weak();
                    Timer::single_shot(Duration::from_millis(800), move || {
                        if let Some(m) = wk.upgrade() {
                            m.set_result_img_copied(false);
                        }
                    });
                }
                Err(e) => log::warn!("clipboard image copy failed: {e}"),
            }
        });
    }
    // フッターのリンク（既定ブラウザで開く。URL は固定＋バージョン埋め込みのみで状態非依存）。
    main.on_open_contact(move || {
        open_url(&format!(
            "https://rererr-portfolio.pages.dev/?from=bpsr-checker&v={}#contact",
            env!("CARGO_PKG_VERSION")
        ));
    });
    main.on_open_github_issue(move || {
        open_url("https://github.com/Rererr/bpsr-checker/issues/new");
    });

    // ── アプリ内更新（GitHub Releases）──
    // 確認で得たリリース一覧（新しい順）を保持し、「更新する」とバージョン選択が同じ内容を使う
    // （再取得しない）。通信スレッドから書くため Arc<Mutex>（UI スレッド専有の Rc では跨げない）。
    let update_releases: Arc<std::sync::Mutex<Vec<update::Release>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    // 通知カードの世代。カードを開くたびに上がり、古い自動クローズを無効化する。
    let toast_gen: ToastGen = Arc::new(AtomicU64::new(0));
    {
        let w = main.as_weak();
        let releases = update_releases.clone();
        let generation = toast_gen.clone();
        main.on_update_check(move || {
            if let Some(m) = w.upgrade() {
                start_update_check(&m, &releases, &generation, false);
            }
        });
    }
    {
        let w = main.as_weak();
        let releases = update_releases.clone();
        let enc_up = enc.clone();
        let generation = toast_gen.clone();
        main.on_update_install(move || {
            if let Some(m) = w.upgrade() {
                start_update_install(&m, &releases, &enc_up, &generation, 0); // 先頭＝最新版
            }
        });
    }
    {
        let w = main.as_weak();
        let releases = update_releases.clone();
        let enc_up = enc.clone();
        let generation = toast_gen.clone();
        main.on_update_install_index(move |i| {
            if let Some(m) = w.upgrade() {
                if let Ok(i) = usize::try_from(i) {
                    start_update_install(&m, &releases, &enc_up, &generation, i);
                }
            }
        });
    }
    {
        let releases = update_releases.clone();
        main.on_update_open_page(move || {
            // 確認済みならそのリリースのページ、未確認なら latest リリースへ飛ばす。
            let url = releases
                .lock()
                .ok()
                .and_then(|list| list.first().map(|r| r.page_url.clone()))
                .unwrap_or_else(|| update::RELEASES_PAGE.to_string());
            open_url(&url);
        });
    }
    {
        let releases = update_releases.clone();
        main.on_update_open_page_index(move |i| {
            let url = releases
                .lock()
                .ok()
                .and_then(|list| usize::try_from(i).ok().and_then(|i| list.get(i).map(|r| r.page_url.clone())))
                .unwrap_or_else(|| update::RELEASES_PAGE.to_string());
            open_url(&url);
        });
    }
    {
        let w = main.as_weak();
        main.on_update_toast_dismiss(move || {
            if let Some(m) = w.upgrade() {
                m.set_update_toast_open(false);
            }
        });
    }

    main.show()?;

    // 起動時の更新確認（設定でOFFにできる唯一の自動外部通信）。起動直後の描画・
    // キャプチャ開始と競合させないよう数秒遅らせてから走らせる。
    if cfg.borrow().check_update_on_startup {
        let w = main.as_weak();
        let releases = update_releases.clone();
        let generation = toast_gen.clone();
        Timer::single_shot(Duration::from_secs(5), move || {
            if let Some(m) = w.upgrade() {
                start_update_check(&m, &releases, &generation, true);
            }
        });
    }
    if cfg.borrow().show_self_status_overlay {
        let _ = self_overlay.show();
    }
    if cfg.borrow().show_buff_overlay {
        let _ = buff_overlay.show();
    }
    if cfg.borrow().show_stats_overlay {
        let _ = stats_overlay.show();
    }

    // 周期ポーリング＋初回セットアップ（位置復元）＋自動保存
    let main_w = main.as_weak();
    let enc_poll = enc.clone();
    let saved = window_state::load();
    let last_saved = Rc::new(RefCell::new(saved.clone()));
    let mut st = PollState::default();
    let tab_cell_poll = tab_cell.clone();
    let drill_poll = drill.clone();
    let skill_rows_poll = skill_rows.clone();
    // 位置/サイズ復元・トレイ連携用の Weak（オーバーレイの表示内容そのものは専用タイマー
    // overlay_timer が別に持つ Weak 経由で更新する。ウィンドウ幾何情報の管理はここで維持）。
    let self_overlay_w = self_overlay.as_weak();
    let buff_overlay_w = buff_overlay.as_weak();
    let stats_overlay_w = stats_overlay.as_weak();
    // メイン一覧の並び順(uid列)＋自キャラuid。バフオーバーレイ(専用タイマー overlay_timer)が
    // 名簿順の算出に読む共有スナップショット。poll側で毎tick書き込む（最大 poll_ms 分だけ
    // 古い可能性があるが、名簿順はサブtickの鮮度を要求しないため許容）。
    let main_order_shared: Rc<RefCell<(Vec<i64>, i64)>> = Rc::new(RefCell::new((Vec::new(), 0)));
    let main_order_poll = main_order_shared.clone();
    let cfg_poll = cfg.clone();
    let wl_poll = wl.clone();
    let history_rows_poll = history_rows.clone();
    let history_expanded_poll = history_expanded.clone();
    let history_player_expanded_poll = history_player_expanded.clone();
    let result_rows_poll = result_rows.clone();
    let result_skill_rows_poll = result_skill_rows.clone();
    let result_pie_poll = result_pie.clone();
    let result_legend_poll = result_legend.clone();
    let selected_result_player_poll = selected_result_player.clone();
    let selected_result_skill_poll = selected_result_skill.clone();
    let captured_skills_poll = captured_skills.clone();
    let last_result_poll = last_result.clone();
    let compact_left_poll = compact_left.clone();
    let compact_right_poll = compact_right.clone();
    let uid_candidates_poll = uid_candidates.clone();
    let best_records_poll = best_records.clone();
    let result_countup_timer_poll = result_countup_timer.clone();
    let result_countup_final_poll = result_countup_final.clone();
    // タスクトレイ／クリックスルー状態（poll closure が move で保持）
    let click_through = Rc::new(Cell::new(false));
    #[cfg(windows)]
    let tray_holder: Rc<RefCell<Option<tray::Tray>>> = Rc::new(RefCell::new(None));
    // グローバルショートカット（poll closure が move で保持。生成はイベントループ稼働後）。
    #[cfg(windows)]
    let hotkeys_holder_poll = hotkeys_holder.clone();
    #[cfg(windows)]
    let shortcuts_poll = shortcuts_model.clone();
    let poll_ms = cfg.borrow().poll_interval_ms.max(50.0) as u64;
    // オーバーレイ(バフ/ステータス/イマジン)の更新は、このメインpollタイマーには相乗りさせず
    // 専用タイマー overlay_timer（このブロックの少し下）で独立に行う。W1: poll に相乗りさせると
    // 発火機会が poll グリッド（既定200ms）に縛られ、かつ poll タイマー自身は EncounterMutex
    // 競合等で処理が数ms遅れるだけで tick を1回飛ばして実効2倍(400ms)に落ちるため、
    // 200/400msが不規則に交替する＝ユーザーが訴えた「ガクッ」を再生産していた（診断済み・
    // レビュー指摘）。詳細な設計意図は overlay_timer のコメントおよび
    // `overlay_next_delay_ms`／`OVERLAY_FALLBACK_MS`／`OVERLAY_DUE_MARGIN_MS`／
    // `OVERLAY_MAX_DELAY_MS`（モジュール直下、テスト容易性のため main() の外に定義）参照。

    poll_timer.start(TimerMode::Repeated, Duration::from_millis(poll_ms), move || {
        st.tick += 1;
        let Some(m) = main_w.upgrade() else {
            return;
        };

        // 初回: winit 実体化後に位置復元 → 完了 tick でトレイ生成。
        let just_setup = poll_setup_once(&m, &mut st, &saved);
        #[cfg(windows)]
        if just_setup {
            *tray_holder.borrow_mut() = tray::create();
            log::info!("tray created: {}", tray_holder.borrow().is_some());
            // 起動時のタスクバー常駐モードをメインへ適用（実体化後）。
            overlay::apply_taskbar_mode(m.window(), cfg_poll.borrow().show_in_taskbar);

            // グローバルショートカット: winit イベントループ稼働後・トレイ生成と同じタイミングで
            // GlobalHotKeyManager を生成する（Windows の RegisterHotKey はメッセージループの
            // あるスレッドでのみ有効なため）。
            let mut hk = hotkey::Hotkeys::new();
            hk.apply(&cfg_poll.borrow());
            *hotkeys_holder_poll.borrow_mut() = Some(hk);
            push_shortcuts_to_ui(&shortcuts_poll, &hotkeys_holder_poll.borrow(), &cfg_poll.borrow());
        }
        #[cfg(not(windows))]
        let _ = just_setup;

        // オーバーレイの位置/サイズ復元（表示された最初のtickで一度）。非表示で None に戻す。
        poll_overlay_restore(
            &mut st,
            &cfg_poll,
            &self_overlay_w,
            &buff_overlay_w,
            &stats_overlay_w,
            &last_saved,
        );

        // トレイメニューのイベント処理（クリックスルー切替・表示/非表示・終了）
        #[cfg(windows)]
        poll_tray_events(
            &m,
            &cfg_poll,
            &self_overlay_w,
            &buff_overlay_w,
            &stats_overlay_w,
            &tray_holder,
            &main_visible,
            &click_through,
        );

        // グローバルショートカットの発火チェックは専用Timer（S4: 固定100ms）に分離済み
        // （このTimerはユーザー設定で最大2000msまで間引かれるため相乗りすると検知が遅れる）。

        // 食事/シロップ残時間ストアを更新（戦闘終了をまたいで保持・失効除去）
        compute::refresh_consumables(&enc_poll);

        // ライブ集計を反映（共有セルの現在タブに応じて取得）
        refresh_header(&m, &enc_poll, tab_cell_poll.get());

        // 観測ステータス（0=起動中 1=待機 2=受信中 3=失敗）。
        // 「受信中」はゲームサーバのパケットを直近10秒以内に処理した場合のみ。
        let cs = compute::get_capture_status();
        m.set_capture_state(match cs.state {
            2 => 3,
            1 if (0.0..10_000.0).contains(&cs.ms_since_last_game_packet) => 2,
            1 => 1,
            _ => 0,
        });

        // イマジン専用モード案内の拡大率。窓が広いほど大きくする（可読性優先。専用モード中は
        // 一覧領域に他に何も出ない）。.slint 側で窓サイズから計算すると
        // 「文字サイズ→preferred-height→レイアウト→窓サイズ」の束縛ループになるため Rust で持つ。
        // Slint の set は同値でも依存を dirty にするため、変化時のみ書いて毎ポーリングの
        // 不要な再レイアウトを避ける（このタイマーは既定 200ms で回り続ける）。
        let scale = notice_scale(&m);
        if (m.get_notice_scale() - scale).abs() > f32::EPSILON {
            m.set_notice_scale(scale);
        }

        // 3分計測 結果モーダルの文字倍率。設定の「フォントサイズ」には依存させず、窓が広い
        // ほど大きくする（notice_scale と同じ理由で .slint 側に計算を持たせると束縛ループに
        // なるため Rust 側で持つ）。結果パネルが閉じていても軽い計算なので毎回更新して構わない。
        let rscale = result_scale(&m);
        if (m.get_result_scale() - rscale).abs() > f32::EPSILON {
            m.set_result_scale(rscale);
        }

        // 一時停止状態をボタンへ反映
        m.set_paused(compute::is_paused(&enc_poll));

        // 設定パネルを開いている間は自キャラUID候補を生きたまま更新（入力欄は触らない）
        if m.get_settings_open() {
            refresh_uid_candidates(&enc_poll, &uid_candidates_poll);
        }

        // 3分計測の状態反映＋残0で自動確定（→履歴。結果パネルは後続増分）
        let ms = compute::get_measure_mode_status(&enc_poll);
        let mkind = match ms.kind.as_str() {
            "pending" => 1,
            "active" => 2,
            _ => 0,
        };
        m.set_measure_kind(mkind);
        if mkind == 2 {
            let rem = ms.remaining_ms.unwrap_or(0.0).max(0.0);
            m.set_measure_text(format::format_elapsed(rem).into());
            if rem <= 0.0 {
                // 最終着弾時刻まで系列を届かせるため、捕捉・確定の前に終端サンプルを足す
                // （X軸自体は固定窓のため、早期に攻撃が止まった場合は右端までは届かない。
                // 詳細は seal_3min_series のコメント参照）。
                // （スキル内訳は下の capture_3min_result_skills で確定前に取得されるため順序が重要）
                compute::seal_3min_series(&enc_poll);
                // finalize で集計が消えるため、直前に自分uidとスキル内訳を捕捉。
                // スキル内訳は finalize（build_encounter_snapshot）と同じ分母
                // （combat_elapsed_ms＝3分計測は実測スパンでなく armed_at 基準の固定窓）で
                // core 側が算出する。ライブの get_skills を別途呼ぶとヘッダ/プレイヤー行の
                // DPSと食い違うため使わない。
                let local_uid = compute::get_dps_players(&enc_poll).local_player_uid as i64;
                let skills = compute::capture_3min_result_skills(&enc_poll);
                if let Some(snap) = compute::finalize_3min_measure_mode(&enc_poll) {
                    // 自己ベスト判定・更新は auto-open 設定に依らず finalize の都度必ず行う
                    // （auto-open OFF でも記録は静かに積み上がり、次にモーダルを見た時に
                    // 正しい自己ベストが出るようにする）。自キャラ不在なら None（記録もしない）。
                    // デモモードでは保存しない(persist=false)がメモリ上の比較は行う。
                    let best_outcome = record_best(&best_records_poll, &snap, local_uid, !demo_mode);
                    let c = cfg_poll.borrow();
                    if c.three_min_auto_open && !c.imagine_only_mode {
                        // 既定の選択=自分(スキル有り)・無ければ最上位プレイヤー
                        let default_uid = if local_uid != 0 && skills.contains_key(&local_uid) {
                            local_uid
                        } else {
                            snap.player_rows.first().map(|p| p.uid as i64).unwrap_or(0)
                        };
                        *captured_skills_poll.borrow_mut() = skills;
                        *last_result_poll.borrow_mut() = Some(snap.clone());
                        show_result(
                            &m,
                            &snap,
                            &captured_skills_poll.borrow(),
                            default_uid,
                            &result_rows_poll,
                            &result_skill_rows_poll,
                            &result_pie_poll,
                            &result_legend_poll,
                            &selected_result_player_poll,
                            &selected_result_skill_poll,
                            c.privacy_mask_names,
                        );
                        // 新記録バッジ/自己ベスト併記の表示プロパティへ反映（モーダルを開く時のみ）。
                        apply_result_best_record_ui(&m, best_outcome);
                        // シェア画像の透かし（バージョン・計測終了時刻・計測時間）。
                        m.set_result_watermark(
                            build_result_watermark(&m.get_app_version(), &snap).into(),
                        );
                        // 新規計測確定時のみ DPS/総ダメをカウントアップ演出する（履歴等からの
                        // 再表示経路は show_result の即時表示のままにする）。
                        start_result_countup(
                            &m,
                            &result_countup_timer_poll,
                            &result_countup_final_poll,
                            snap.total_dps,
                            snap.total_dmg,
                        );
                    }
                }
            }
        }

        // メイン表示中タブの並び順(uid列)。イマジンタイマーの行順をこれに追従させる。
        // 履歴タブ等で算出できない場合は空＝従来の watched 順へフォールバック。
        let mut main_ordered_uids: Vec<i64> = Vec::new();
        let mut main_local_uid: i64 = 0;
        let cur_tab = tab_cell_poll.get();
        if cur_tab == 3 {
            // 履歴タブ: 確定済みエンカウンタ一覧を反映（展開状態は維持）。
            // このタブでは一覧を組み立てないため、自キャラ名の未取得ヒント
            // (local-name-unresolved) は直近の一覧タブでの値を据え置く。名前は入場時にしか
            // 変わらないので陳腐化の実害が小さく、この判定のためだけに毎 tick 集計を回さない。
            let hist = compute::get_history();
            history_rows_poll.apply(build_history_rows(
                &hist,
                history_expanded_poll.get(),
                history_player_expanded_poll.get(),
                &cfg_poll.borrow(),
            ));
        } else {
            let pw = fetch_players(&enc_poll, cur_tab);
            // 名簿は一覧の表示行ではなく core の非射影リストから採る。自分のみ計測中は一覧が
            // 自分1行に絞られるが、timer_roster はこの列を所属の決定にも使うため、そのまま
            // 渡すと PT メンバーのバフ/イマジンタイマーが黙って消える。
            main_ordered_uids = compute::get_roster_uids(&enc_poll, tab_stat(cur_tab))
                .into_iter()
                .map(|uid| uid as i64)
                .collect();
            main_local_uid = pw.local_player_uid as i64;
            let c = cfg_poll.borrow();
            m.set_show_graph_col(graph_col_active(&c, cur_tab));
            // メイン行のピン点灯集合（専用モードは導出元が異なるため常に非専用扱い）。
            let pin_uids = timer_roster(
                &wl_poll.borrow(),
                false,
                c.sync_timer_with_main,
                c.sync_order_follow,
                &main_ordered_uids,
                &[],
                main_local_uid,
            );
            apply_player_rows(
                &m,
                &rows,
                &compact_left_poll,
                &compact_right_poll,
                build_rows(
                    &pw,
                    &c.name_template,
                    c.abbreviate_scores,
                    c.privacy_mask_names,
                    &pin_uids,
                    c.graph_player_count as i32,
                    c.graph_for_local_player,
                    &dps_bar_config(&c),
                ),
            );
        }
        // バフオーバーレイ(専用タイマー overlay_timer)の名簿順算出用に共有する
        // （cur_tab==3 の履歴タブでは空/0のまま＝従来の watched 順フォールバックに委ねる）。
        *main_order_poll.borrow_mut() = (main_ordered_uids, main_local_uid);

        // ドリルダウン中はライブ更新
        // 自分のみ計測が始まると他プレイヤーの内訳は core 側で断られる。開いたままにすると
        // 直前の行が画面に残り続けるため、表示可否を core の判定（compute::breakdown_visible）
        // に問い合わせて一覧へ戻す。
        let drilled_uid = match drill_poll.get() {
            Drill::Skills(uid) | Drill::TakenAttackers(uid) | Drill::TakenSkills(uid, _) => Some(uid),
            Drill::None => None,
        };
        if let Some(uid) = drilled_uid {
            if !compute::breakdown_visible(&enc_poll, uid) {
                drill_poll.set(Drill::None);
                m.set_view(0);
            }
        }

        match drill_poll.get() {
            Drill::Skills(uid) => {
                // タブ切替でdrillはNoneへ戻るため(on_select_tab)、Skills中はcur_tabが
                // 開いた時のタブのまま＝そのタブ基準で内訳を取り続けてよい。
                if let Ok(sw) = compute::get_skills(&enc_poll, uid, tab_stat(cur_tab)) {
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

        // オーバーレイ(バフ/ステータス/イマジン)のモデル更新は専用タイマー overlay_timer が
        // 独立したスケジュールで行う（このブロックの少し下・W1参照）。ここでは行わない。

        // 起動/表示直後の preferred サイズ再アサートを settle 期間中の再適用で打ち消す
        // （自動保存ガードより手前で実施）。
        poll_window_settle(&m, &st, &self_overlay_w, &buff_overlay_w, &stats_overlay_w);

        // settle 完了後、窓幅が最小幅を下回っていれば底上げする。主目的は settle が Slint の
        // 制約を経由せず適用した保存幅（最小幅未満）の補正。実行中の設定変更で最小幅が広がった
        // 場合も拾う（settle 中は poll_window_settle と競合するため対象外）。
        if main_settled(&st) {
            let sf = m.window().scale_factor();
            let min_w = (m.get_layout_min_width() * sf).round() as u32;
            let min_h = (m.get_layout_min_height() * sf).round() as u32;
            window_state::grow_to_min(m.window(), min_w, min_h);
        }

        // レイアウト自動保存（復元確定後・差分時のみ）。
        poll_auto_save(
            &m,
            &st,
            &cfg_poll,
            &self_overlay_w,
            &buff_overlay_w,
            &stats_overlay_w,
            &last_saved,
        );
    });

    // オーバーレイ(バフ/ステータス/イマジンタイマー)専用タイマー（W1）。
    // メインpollタイマーに相乗りさせていた旧実装は、発火機会が poll グリッド（既定200ms）に
    // 縛られる上、poll タイマー自身が Repeated（コールバック実行"前"に再武装する非補償型・
    // slint 1.16.1 `timers.rs`）なので、EncounterMutex 競合等で処理が数ms遅れるだけで tick を
    // 1回飛ばして実効2倍(400ms)に落ちてしまい、200/400msが不規則に交替する＝ユーザーが訴えた
    // 「ガクッ」そのものを再生産していた（レビュー指摘・診断済み）。専用タイマーへ分離することで
    // 発火グリッドを poll から独立させ、`overlay_next_delay_ms` が返す 31〜199ms の同期を
    // 実際に効かせる。
    //
    // TimerMode::Repeated + 毎回コールバック末尾で set_interval を選ぶ理由:
    // SingleShot は再武装（次の single_shot 予約）を呼び忘れると「更新が永久停止」する事故に
    // 直結するが、Repeated は set_interval の呼び忘れがあっても直前の周期で回り続ける
    // （フォールバック要件を構造的に満たす。overlay_next_delay_ms 自身の
    // OVERLAY_FALLBACK_MS フォールバックと合わせた二重の安全策）。
    //
    // poll_interval_ms の設定変更はメインpollタイマー(poll_timer)にのみ影響し、この
    // overlay_timer の周期はそれとは独立に自身が算出した値で回り続ける
    // （＝オーバーレイの滑らかさは poll_interval_ms 設定から独立して保たれる）。
    // 例外はオーバーレイを1つも表示していないときで、この間は更新対象が無く滑らかさの
    // 対象も存在しないため poll_interval_ms を下限として長く休む（OVERLAY_IDLE_MIN_MS）。
    let overlay_timer: Rc<Timer> = Rc::new(Timer::default());
    {
        let overlay_timer_self = overlay_timer.clone();
        let enc_ov = enc.clone();
        let cfg_ov = cfg.clone();
        let wl_ov = wl.clone();
        let self_overlay_w = self_overlay.as_weak();
        let self_buffs_ov = self_buffs.clone();
        let self_debuffs_ov = self_debuffs.clone();
        let stats_overlay_w = stats_overlay.as_weak();
        let stats_rows_ov = stats_rows.clone();
        let buff_overlay_w = buff_overlay.as_weak();
        let buff_players_ov = buff_players.clone();
        let main_order_ov = main_order_shared.clone();
        let mut ov = OverlayState::default();
        overlay_timer.start(
            TimerMode::Repeated,
            Duration::from_millis(OVERLAY_INITIAL_DELAY_MS),
            move || {
                // このtickで実際に更新するセルの remaining_ms/duration_ms から、次に表示が
                // 変わるまでの時間を集計する（複数セルがあれば最小値＝最初に変わるセルに同期）。
                // 1つも無ければ None のままとなり、下で OVERLAY_FALLBACK_MS へフォールバックする。
                let mut next_change_ms: Option<u64> = None;

                // オーバーレイの文字サイズは窓ごとに独立した専用設定。
                // 表示可否も同時に1回だけ読む。各オーバーレイの更新分岐と、末尾の
                // 「1つも表示していないなら休む」判定の両方がこの値を使う
                // （同じ対象を判定する式を2箇所に書かない）。
                let (self_scale, stats_scale, imagine_scale, show_self, show_stats, show_buff) = {
                    let c = cfg_ov.borrow();
                    (
                        (c.buff_overlay_font_size / 12.0) as f32,
                        (c.stats_overlay_font_size / 12.0) as f32,
                        (c.imagine_overlay_font_size / 12.0) as f32,
                        c.show_self_status_overlay,
                        c.show_stats_overlay,
                        c.show_buff_overlay,
                    )
                };

                // 自キャラ オーバーレイ更新（表示中のみ）
                if show_self {
                    if let Some(o) = self_overlay_w.upgrade() {
                        o.set_font_scale(self_scale);
                        let s = compute::get_self_buff_status(&enc_ov);
                        o.set_waiting(s.local_player_uid == 0.0);
                        // 次回発火予定の算出用に、表示中のバフ/デバフセルの残量を集計する
                        // （W2: このオーバーレイは format::format_remaining で描画するため、
                        // 「次に表示が変わる時刻」も同じ10秒閾値・丸め方式から導出する
                        // format::next_text_change_ms を使う＝重複した判定式を作らない）。
                        for e in s.buffs.iter().chain(s.debuffs.iter()) {
                            next_change_ms = merge_next_change_ms(
                                next_change_ms,
                                format::next_text_change_ms(e.remaining_ms, e.duration_ms),
                            );
                        }
                        sync_model_if_changed(&self_buffs_ov, &mut ov.last_self_buffs, build_status_entries(&s.buffs));
                        sync_model_if_changed(&self_debuffs_ov, &mut ov.last_self_debuffs, build_status_entries(&s.debuffs));
                    }
                }

                // 自キャラ ステータス オーバーレイ更新（表示中のみ。数値ステータスは秒刻みの
                // 表示が無いため、この窓自体は発火予定の算出に寄与しない）
                if show_stats {
                    if let Some(o) = stats_overlay_w.upgrade() {
                        o.set_font_scale(stats_scale);
                        let s = compute::get_self_stats(&enc_ov);
                        o.set_waiting(s.local_player_uid == 0.0);
                        let enabled = cfg_ov.borrow().stats_enabled.clone();
                        sync_model_if_changed(&stats_rows_ov, &mut ov.last_stats_rows, build_stat_entries(&s, &enabled));
                    }
                }

                // バフタイマー オーバーレイ更新（表示中のみ）
                if show_buff {
                    if let Some(o) = buff_overlay_w.upgrade() {
                        o.set_font_scale(imagine_scale);
                        let imagine_only = cfg_ov.borrow().imagine_only_mode;
                        {
                            // 表示するイマジン列・レイアウトを設定から反映（極小コスト・即時反映）
                            let c = cfg_ov.borrow();
                            o.set_show_tina(c.show_imagine_tina);
                            o.set_show_aluna(c.show_imagine_aluna);
                            o.set_show_tarta(c.show_imagine_tarta);
                            o.set_show_basilisk(c.show_imagine_basilisk);
                            o.set_show_kartgriff(c.show_imagine_kartgriff);
                            o.set_compact(c.imagine_compact_rows);
                        }
                        // 名簿源は3分岐（timer_roster 参照）。専用モードはバフ追跡から自動
                        // （メイン一覧が空集計のため使えない）。専用OFFはメイン順
                        // （main_order_shared・メインpollタイマーが毎tick書き込む共有スナップ
                        // ショット。最大 poll_ms 分だけ古い可能性があるが名簿順に鮮度は不要）、
                        // 無い(履歴タブ等)場合は live DPS 順を代用。
                        let (main_ordered_uids, main_local_uid) = main_order_ov.borrow().clone();
                        let (order_src, local_uid): (Vec<i64>, i64) = if imagine_only {
                            (Vec::new(), main_local_uid)
                        } else if main_ordered_uids.is_empty() {
                            let pw = compute::get_dps_players(&enc_ov);
                            let uids = pw.player_rows.iter().map(|p| p.uid as i64).collect();
                            (uids, pw.local_player_uid as i64)
                        } else {
                            (main_ordered_uids, main_local_uid)
                        };
                        let buff_tracked_uids = if imagine_only {
                            compute::get_buff_tracked_uids(&enc_ov)
                        } else {
                            Vec::new()
                        };
                        let (sync, order_follow) = {
                            let c = cfg_ov.borrow();
                            (c.sync_timer_with_main, c.sync_order_follow)
                        };
                        let display_uids = timer_roster(
                            &wl_ov.borrow(),
                            imagine_only,
                            sync,
                            order_follow,
                            &order_src,
                            &buff_tracked_uids,
                            local_uid,
                        );
                        o.set_empty(display_uids.is_empty());
                        // 表示集合が空なら空行で更新（古い行が残って名前が消えない不具合を防ぐ）。
                        // いずれも sync_model_if_changed で変化行のみ再描画する。
                        let privacy_mask = cfg_ov.borrow().privacy_mask_names;
                        let next_buff_rows = if !display_uids.is_empty() {
                            let uids: Vec<f64> = display_uids.iter().map(|&u| u as f64).collect();
                            let t = compute::get_tracked_buffs(&enc_ov, uids);
                            // S3: 発火予定の算出は実際に描画したセル（build_buff_rows が返す
                            // 側）から集める。tracked.players[].buffs[] を別途全走査すると、
                            // 表示していないkind/uidまで拾って表示セル集合と食い違うため。
                            let (rows, buff_next_change_ms) = build_buff_rows(&t, &display_uids, privacy_mask);
                            next_change_ms = merge_next_change_ms(next_change_ms, buff_next_change_ms);
                            rows
                        } else {
                            Vec::new()
                        };
                        sync_model_if_changed(&buff_players_ov, &mut ov.last_buff_players, next_buff_rows);
                    }
                }

                // 次回発火の周期を張り直す（秒境界同期＋アーク/バー滑らかさ上限クランプ＋
                // フォールバック。詳細は overlay_next_delay_ms 参照）。Repeated タイマーのため、
                // 万一ここへ到達できなくても直前の周期で回り続け、更新の永久停止は起きない。
                // オーバーレイを1つも表示していない間だけは更新対象が無いので長く休む
                // （OVERLAY_IDLE_MIN_MS 参照。表示中の周期は従来どおり poll から独立）。
                let delay_ms = if show_self || show_stats || show_buff {
                    overlay_next_delay_ms(next_change_ms)
                } else {
                    let poll_ms = cfg_ov.borrow().poll_interval_ms.max(50.0) as u64;
                    OVERLAY_IDLE_MIN_MS.max(poll_ms)
                };
                overlay_timer_self.set_interval(Duration::from_millis(delay_ms));
            },
        );
    }

    // グローバルショートカット専用ポーリング（issue #3 S4）。メインpollタイマーは設定で
    // 最大2000msまで間引かれる（poll_interval_ms、上のスライダーで調整可）ため、相乗りすると
    // 押下から検知まで最大2秒遅れうる。固定100msの専用Timerで受信する
    // （try_recvは空チャネルへの非ブロッキング1回のみでコストは無視できる）。
    #[cfg(windows)]
    let hotkey_timer = Timer::default();
    #[cfg(windows)]
    {
        let hotkey_w = main.as_weak();
        let hotkeys_hk = hotkeys_holder.clone();
        hotkey_timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
            let Some(m) = hotkey_w.upgrade() else {
                return;
            };
            poll_hotkey_events(&m, &hotkeys_hk);
        });
    }

    // トレイ格納（全ウィンドウ hide）でアプリが終了しないよう、最後のウィンドウが
    // 閉じてもループを止めない。終了はトレイ「終了」/ ×ボタンの quit_event_loop のみ。
    slint::run_event_loop_until_quit()?;

    persist_state(&enc); // 終了時に最終状態を永続化
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist_row(name_text: &str, dmg: &str) -> HistoryRowUi {
        let part = format::NamePart {
            text: name_text.to_string(),
            class_icon: false,
            shrink_rank: 0,
            has_name: true,
        };
        HistoryRowUi {
            row: Row {
                name_parts: ui_name_parts(vec![part]),
                dmg_text: dmg.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    // 名前パーツの ModelRc は行ごとに別インスタンスだが、中身が同じなら「変化なし」と判定する。
    #[test]
    fn history_row_same_compares_name_parts_by_content() {
        assert!(history_row_same(&hist_row("ソラ", "1.0K"), &hist_row("ソラ", "1.0K")));
        assert!(!history_row_same(&hist_row("ソラ", "1.0K"), &hist_row("ハヤテ", "1.0K")));
        assert!(!history_row_same(&hist_row("ソラ", "1.0K"), &hist_row("ソラ", "2.0K")));
    }

    // 行数が同じでも、内容が変わった行はモデルへ反映される（テンプレート変更など）。
    #[test]
    fn history_rows_apply_reflects_changed_and_resized_rows() {
        let rows = HistoryRows::new();
        rows.apply(vec![hist_row("ソラ", "1.0K"), hist_row("ハヤテ", "2.0K")]);
        rows.apply(vec![hist_row("ソラ", "1.0K"), hist_row("ハヤテ (x)", "2.0K")]);
        let part_text = |i: usize| rows.model.row_data(i).unwrap().row.name_parts.row_data(0).unwrap().text;
        assert_eq!(part_text(1).as_str(), "ハヤテ (x)");
        assert_eq!(part_text(0).as_str(), "ソラ");
        rows.apply(vec![hist_row("ソラ", "1.0K")]);
        assert_eq!(rows.model.row_count(), 1);
        rows.apply(Vec::new());
        assert_eq!(rows.model.row_count(), 0);
    }

    fn wl_with(watched: &[i64], excluded: &[i64]) -> watchlist::Watchlist {
        watchlist::Watchlist {
            watched: watched.to_vec(),
            excluded: excluded.to_vec(),
        }
    }

    // 専用モードON: 名簿源は buff_tracked_uids（first-seen順）。excluded を除き、
    // 自分(local_uid)を先頭固定＋以降は元の順（first-seen）をそのまま使う。
    #[test]
    fn test_timer_roster_imagine_only_orders_local_first_then_first_seen() {
        let wl = wl_with(&[], &[300]); // 300 は手動で隠している
        let buff_tracked = vec![200, 100, 300]; // first-seen 順（200が最初に検出）
        let roster = timer_roster(&wl, true, false, true, &[], &buff_tracked, 100);
        // 100(自分)が先頭、300はexcludedで除外、残りはfirst-seen順(200)
        assert_eq!(roster, vec![100, 200]);
    }

    // 専用OFF・同期ON・追従ON: メイン順そのまま（excluded のみ除外）。
    #[test]
    fn test_timer_roster_sync_on_follow_on_uses_main_order() {
        let wl = wl_with(&[], &[20]);
        let main_ordered = vec![30, 20, 10];
        let roster = timer_roster(&wl, false, true, true, &main_ordered, &[], 10);
        assert_eq!(roster, vec![30, 10]);
    }

    // 専用OFF・同期ON・追従OFF: 顔ぶれはメイン−excludedと同じだが、並びは自分が先頭固定＋
    // 残りは uid 昇順の安定順（live DPS 順を無視）。
    #[test]
    fn test_timer_roster_sync_on_follow_off_uses_stable_local_first_order() {
        let wl = wl_with(&[], &[]);
        let main_ordered = vec![30, 10, 20]; // 仮にDPS順でシャッフルされていても無視される
        let roster = timer_roster(&wl, false, true, false, &main_ordered, &[], 10);
        // 自分(10)が先頭、残り(20,30)はuid昇順
        assert_eq!(roster, vec![10, 20, 30]);
    }

    // 専用OFF・同期OFF: 手動ウォッチ(watched)のみ。メイン順があれば追従させる。
    #[test]
    fn test_timer_roster_manual_uses_watched_ordered_by_main() {
        let wl = wl_with(&[10, 20], &[]);
        let main_ordered = vec![20, 10, 99];
        let roster = timer_roster(&wl, false, false, true, &main_ordered, &[], 10);
        assert_eq!(roster, vec![20, 10]);
    }

    // 専用モードは上限(watchlist::MAX)を超えない。
    #[test]
    fn test_timer_roster_imagine_only_respects_max_limit() {
        let wl = wl_with(&[], &[]);
        let buff_tracked: Vec<i64> = (1..=(watchlist::MAX as i64 + 10)).collect();
        let roster = timer_roster(&wl, true, false, true, &[], &buff_tracked, 0);
        assert_eq!(roster.len(), watchlist::MAX);
    }

    // --- 履歴見出しタイトル（issue #9 PR2b）関連のユニットテスト ---

    // 期待値は build_history_title と同じ変換（chrono::Local.timestamp_millis_opt(..).single()）
    // から組み立てる。実行環境のローカルタイムゾーンに依存する値なので固定文字列と比較しない。
    fn expected_date_str(start_ms: f64) -> String {
        use chrono::TimeZone;
        chrono::Local
            .timestamp_millis_opt(start_ms as i64)
            .single()
            .expect("valid timestamp")
            .format(DATETIME_DISPLAY_FORMAT)
            .to_string()
    }

    #[test]
    fn build_history_title_combines_date_and_content_name() {
        let start_ms = 1_704_164_645_000.0;
        let title = build_history_title(start_ms, "霧海の猟場 マスター難易度1");
        assert_eq!(title, format!("{} 霧海の猟場 マスター難易度1", expected_date_str(start_ms)));
    }

    #[test]
    fn build_history_title_start_ms_zero_omits_date() {
        assert_eq!(build_history_title(0.0, "霧海の猟場 マスター難易度1"), "霧海の猟場 マスター難易度1");
    }

    #[test]
    fn build_history_title_empty_content_name_keeps_date_only() {
        let start_ms = 1_704_164_645_000.0;
        assert_eq!(build_history_title(start_ms, ""), expected_date_str(start_ms));
    }

    #[test]
    fn build_history_title_both_missing_is_empty() {
        assert_eq!(build_history_title(0.0, ""), "");
    }

    // --- オーバーレイ秒境界同期（S1）関連のユニットテスト ---

    // 1000ms境界を跨ぐまでの残余が発火間隔になる（跨ぐ瞬間の text 変化に一致させる。
    // バトルイマジンタイマー用＝常に ceil・1000ms格子）。
    #[test]
    fn test_imagine_cell_next_change_ms_returns_remainder_within_second() {
        assert_eq!(imagine_cell_next_change_ms(60_000, 4_400), Some(400));
    }

    // remaining_ms がちょうど1000の倍数のときは、次の境界まで丸々1000ms（跨いだ直後の想定）。
    #[test]
    fn test_imagine_cell_next_change_ms_exact_multiple_of_1000_returns_full_second() {
        assert_eq!(imagine_cell_next_change_ms(60_000, 4_000), Some(1000));
    }

    // 無期限(duration<=0)・表示上ゼロ以下(remaining<=0)は秒が動かないため対象外。
    #[test]
    fn test_imagine_cell_next_change_ms_none_for_infinite_or_expired() {
        assert_eq!(imagine_cell_next_change_ms(0, 4_400), None);
        assert_eq!(imagine_cell_next_change_ms(60_000, 0), None);
        assert_eq!(imagine_cell_next_change_ms(60_000, -1), None);
    }

    // 複数セルの最小値（＝最初に表示が変わるセル）へ畳み込む。片方 None は無視する。
    #[test]
    fn test_merge_next_change_ms_takes_minimum_and_ignores_none() {
        assert_eq!(merge_next_change_ms(None, None), None);
        assert_eq!(merge_next_change_ms(Some(500), None), Some(500));
        assert_eq!(merge_next_change_ms(None, Some(300)), Some(300));
        assert_eq!(merge_next_change_ms(Some(500), Some(300)), Some(300));
    }

    // 無期限(duration<=0)は量子化せずそのまま返す（クランプのみ）。
    #[test]
    fn test_quantize_ratio_infinite_duration_returns_clamped_ratio_unchanged() {
        assert_eq!(quantize_ratio(0.42, 0), 0.42);
        assert_eq!(quantize_ratio(1.5, 0), 1.0);
        assert_eq!(quantize_ratio(-0.5, 0), 0.0);
    }

    // 時間軸100ms相当の段へ丸める（duration=1000msなら1段=10%刻み）。
    #[test]
    fn test_quantize_ratio_rounds_to_100ms_step_for_1s_duration() {
        let q = quantize_ratio(0.83, 1000);
        assert!((q - 0.8).abs() < 1e-4, "expected ~0.8, got {q}");
    }

    // duration が短いほど段が粗くなる（100ms未満のdurationは実質2値: 0か1）が、範囲は超えない。
    #[test]
    fn test_quantize_ratio_short_duration_clamps_step_to_one() {
        assert_eq!(quantize_ratio(0.3, 50), 0.0);
        assert_eq!(quantize_ratio(0.9, 50), 1.0);
    }

    // 長時間バフ(256秒)でも1段が約100ms相当まで細かくなる（旧1/128固定の粗さの解消を検証）。
    #[test]
    fn test_quantize_ratio_long_duration_stays_close_to_original_ratio() {
        let duration_ms = 256_000;
        let ratio = 0.6173;
        let q = quantize_ratio(ratio, duration_ms);
        // 段幅 = 100ms/256000ms ≒ 0.00039。誤差はその半分程度に収まるはず。
        assert!((q - ratio).abs() < 0.0004, "expected close to {ratio}, got {q}");
    }

    // 上限クランプが効くケース: 秒境界までの残りが長い（=400ms）と、マージンを足した430msは
    // OVERLAY_MAX_DELAY_MS(200ms) を超えるためクランプされる（アーク/バーを最大でも200ms
    // 間隔で動かし続けるため。秒境界に同期させつつ滑らかさも確保する要件の核心）。
    #[test]
    fn test_overlay_next_delay_ms_clamps_to_max_when_boundary_is_far() {
        // 残り4400ms→境界まで400ms。
        let next_change = imagine_cell_next_change_ms(60_000, 4_400);
        assert_eq!(next_change, Some(400));
        assert_eq!(overlay_next_delay_ms(next_change), OVERLAY_MAX_DELAY_MS);
    }

    // 上限クランプが効かないケース: 秒境界までの残りが短い（=100ms）と、マージン込みの130msは
    // 上限を超えないためそのまま採用され、秒境界ちょうど（＋マージン）に同期する。
    #[test]
    fn test_overlay_next_delay_ms_uses_margin_when_boundary_is_near() {
        // 残り4100ms→境界まで100ms。
        let next_change = imagine_cell_next_change_ms(60_000, 4_100);
        assert_eq!(next_change, Some(100));
        assert_eq!(overlay_next_delay_ms(next_change), 130);
    }

    // S5: マージン加算前にクランプすることで、境界までの残りが 171〜200ms のときでも
    // マージン30msが必ず維持される（クランプ後にマージンを足すと 200ms 丁度に潰れて
    // マージンが目減りし、境界の手前で発火しうる旧実装のバグを再発させないための回帰テスト）。
    #[test]
    fn test_overlay_next_delay_ms_preserves_full_margin_near_the_clamp_boundary() {
        assert_eq!(overlay_next_delay_ms(Some(180)), 200);
        assert_eq!(overlay_next_delay_ms(Some(170)), 200);
        assert_eq!(overlay_next_delay_ms(Some(169)), 199);
    }

    // 表示中セルが無い/算出できない場合は固定フォールバック間隔へ必ず落ちる
    // （再武装漏れで更新が永久停止する事故を防ぐための必須要件）。
    #[test]
    fn test_overlay_next_delay_ms_falls_back_when_no_cell() {
        assert_eq!(overlay_next_delay_ms(None), OVERLAY_FALLBACK_MS);
    }

    // --- 食事/シロップ アイコン(consumable_display)関連のユニットテスト ---

    // 残量に余裕があるうちは基準色（警告なし）のまま。
    #[test]
    fn test_consumable_display_uses_base_tint_when_remaining_is_plenty() {
        let (active, ratio, _time, _label, tint) =
            consumable_display(600_000.0, 1_800_000.0, 0, FOOD_TINT_RGB);
        assert!(active);
        assert!((ratio - (1.0 / 3.0)).abs() < 1e-6);
        assert_eq!(tint, slint::Color::from_rgb_u8(0x66, 0xbb, 0x6a));
    }

    // 残り5分未満は注意色（既存の警告バナーと同色）へ切り替わる。
    #[test]
    fn test_consumable_display_switches_to_caution_tint_under_five_minutes() {
        let (_active, _ratio, _time, _label, tint) =
            consumable_display(299_999.0, 1_800_000.0, 0, FOOD_TINT_RGB);
        assert_eq!(tint, slint::Color::from_rgb_u8(0xff, 0xb4, 0x54));
    }

    // 残り1分未満は危険色（build_status_entries の is_low と同色）へ切り替わる。
    #[test]
    fn test_consumable_display_switches_to_critical_tint_under_one_minute() {
        let (_active, _ratio, _time, _label, tint) =
            consumable_display(59_999.0, 1_800_000.0, 0, SYRUP_TINT_RGB);
        assert_eq!(tint, slint::Color::from_rgb_u8(0xff, 0x70, 0x43));
    }

    // 未使用(duration<=0 または remaining<=0)は非アクティブ扱いで基準色を返す（表示上は使われない）。
    #[test]
    fn test_consumable_display_inactive_when_unused() {
        let (active, ratio, time, label, tint) = consumable_display(0.0, 1_800_000.0, 0, SYRUP_TINT_RGB);
        assert!(!active);
        assert_eq!(ratio, 0.0);
        assert_eq!(time, "");
        assert_eq!(label, "");
        assert_eq!(tint, slint::Color::from_rgb_u8(0xb0, 0x7c, 0xff));
    }

    fn ts_point(t_ms: f64, dps: f64) -> bpsr_core::models::TimeSeriesPoint {
        bpsr_core::models::TimeSeriesPoint { t_ms, total_dmg: 0.0, total_dps: dps }
    }

    /// パス文字列から数値トークンだけを取り出す（M/L と座標が空白区切りで並ぶ前提）。
    fn spark_coords(cmds: &str) -> Vec<f32> {
        cmds.split_whitespace().filter_map(|t| t.parse::<f32>().ok()).collect()
    }

    // issue #7 回帰防止: 最終サンプルが計測末尾なら折れ線は右端(SPARK_VB)まで届く。
    // 旧実装はプロット実寸(px)を引数で受けており、実寸が渡らないと途中で切れていた
    // （現在は実寸を受ける引数自体が無く、引き伸ばしは .slint の fit:fill が行う）。
    #[test]
    fn spark_path_reaches_right_edge_when_last_sample_is_at_duration() {
        let pts = vec![ts_point(0.0, 10.0), ts_point(90_000.0, 20.0), ts_point(180_000.0, 5.0)];
        let cmds = build_spark_dps_time(&pts, 180_000.0);
        let coords = spark_coords(&cmds);
        let last_x = coords[coords.len() - 2]; // 末尾は (x, y) の並び
        assert!((last_x - SPARK_VB).abs() < 1e-4, "右端まで届いていない: {cmds}");
    }

    // 座標が正規化範囲(0..SPARK_VB)を出ないこと。範囲外の点は fit:fill 後に要素外へ出て
    // クリップされ、DPSが低い区間の線が消える（issue #7 で実際に起きた症状）。
    #[test]
    fn spark_path_coords_stay_inside_normalized_viewbox() {
        let pts = vec![ts_point(0.0, 0.0), ts_point(60_000.0, 12_345.0), ts_point(180_000.0, 1.0)];
        let cmds = build_spark_dps_time(&pts, 180_000.0);
        for v in spark_coords(&cmds) {
            assert!((0.0..=SPARK_VB).contains(&v), "正規化座標の範囲外: {v} / {cmds}");
        }
        // 最大DPSの点は上端(y=0)、最小は下端(y=SPARK_VB)＝Y方向も全高を使う。
        let ys: Vec<f32> = spark_coords(&cmds).iter().skip(1).step_by(2).copied().collect();
        let ymin = ys.iter().copied().fold(f32::INFINITY, f32::min);
        let ymax = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(ymin.abs() < 1e-4, "最大DPSが上端に接していない: {cmds}");
        assert!((ymax - SPARK_VB).abs() < 1e-4, "DPS=0 が下端に接していない: {cmds}");
    }
}
