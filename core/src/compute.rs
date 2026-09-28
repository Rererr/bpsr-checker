use crate::engine::buff_source::BuffSourceKind;
use crate::engine::class::{Class, ClassSpec};
use crate::engine::combat_stats::CombatStats;
use crate::engine::entity::{EntityKey, MAX_IMAGINE_NAMES, MAX_ROLE_SKILL_IMAGINES};
use crate::engine::runtime_settings::{self, Lang};
use crate::engine::encounter::{Encounter, EncounterMutex};
use crate::engine::name_cache;
use crate::engine::selected_uid;
use crate::engine::skill_names::get_skill_name;
use crate::models::{
    EncounterSnapshot, HeaderInfo, MeasureModeStatus, PlayerBuffSnapshot, PlayerRow,
    PlayerSkillSnapshot, PlayersWindow, SelfBuffSnapshot, SelfStatsData, SelfStatusData,
    SelfStatusEntry, SkillRow, SkillsWindow, TimeSeriesPoint, TrackedBuffsData,
};
use log::info;
use std::collections::VecDeque;

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CachedPlayerDto {
    pub name: String,
    pub class_id: Option<i32>,
    pub ability_score: Option<i32>,
}

#[inline]
fn ratio_pct(num: i64, denom: i64) -> f64 {
    if denom == 0 {
        0.0
    } else {
        num as f64 / denom as f64 * 100.0
    }
}

#[inline]
fn ratio_count_pct(num: u32, denom: u32) -> f64 {
    if denom == 0 {
        0.0
    } else {
        num as f64 / denom as f64 * 100.0
    }
}

#[inline]
fn rate_per_sec(total: i64, elapsed_secs: f64) -> f64 {
    if elapsed_secs <= 0.0 {
        0.0
    } else {
        total as f64 / elapsed_secs
    }
}

#[inline]
fn rate_per_minute(count: u32, elapsed_secs: f64) -> f64 {
    if elapsed_secs <= 0.0 {
        0.0
    } else {
        count as f64 / elapsed_secs * 60.0
    }
}

fn sort_skill_rows_desc(rows: &mut [SkillRow]) {
    rows.sort_by(|a, b| {
        b.total_value
            .partial_cmp(&a.total_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn sort_player_rows_desc(rows: &mut [PlayerRow]) {
    rows.sort_by(|a, b| {
        b.total_value
            .partial_cmp(&a.total_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// `enc` をロックしてクロージャを実行する。ロックが poison していた場合は
/// `ctx` 付きでエラーログを出し `default` を返す（各所に散在していたロック定型を集約）。
fn with_lock_or<T>(
    enc: &EncounterMutex,
    ctx: &str,
    default: T,
    f: impl FnOnce(&mut Encounter) -> T,
) -> T {
    match enc.lock() {
        Ok(mut encounter) => f(&mut encounter),
        Err(e) => {
            log::error!("Lock poisoned in {ctx}: {e}");
            default
        }
    }
}

/// DPS等の分母(ms)の単一定義。ライブ表示・確定(finalize)・3分計測のいずれもこの関数を通す
/// （同じ対象を判定する条件式を2つ書かない規約。分岐はここの2本だけに集約する）。
/// - 未戦闘（`time_fight_start_ms == 0`）: 0
/// - 3分計測中/確定（`MeasureMode::Active3Min`）: `armed_at_ms` からの実経過を `duration_ms`
///   で頭打ちにする。`armed_at_ms` は最初の攻撃で `time_fight_start_ms` と同時刻にセットされる
///   ため、ライブ中はそのまま伸び、窓を過ぎたら確定時と同じ `duration_ms` に収束する
///   （3:00到達で確定した瞬間に値が下へ跳ぶ不連続が構造的に無い）。
/// - それ以外（通常モード。ライブ・確定とも常にこちら）: `time_last_combat_packet_ms -
///   time_fight_start_ms`（実測スパン）。「戦闘中は現在時刻を分母にする」設計は他ツール3種の
///   いずれも採用していないため撤回済み（設定「戦闘終了(秒)=0」で分母が無限に伸びる／
///   一時停止中も分母が動く／時計巻き戻りで分母が実測スパンを下回る、の3件が同時に解消する）。
fn combat_elapsed_ms(encounter: &Encounter, now: u128) -> u128 {
    if encounter.time_fight_start_ms == 0 {
        return 0;
    }
    if let crate::engine::encounter::MeasureMode::Active3Min {
        armed_at_ms,
        duration_ms,
        ..
    } = encounter.measure_mode
    {
        return now.saturating_sub(armed_at_ms).min(duration_ms);
    }
    encounter
        .time_last_combat_packet_ms
        .saturating_sub(encounter.time_fight_start_ms)
}

fn skill_row_for(
    uid: f64,
    name: String,
    element: u8,
    damage_mode: u8,
    stats: &CombatStats,
    elapsed_secs: f64,
    denominator: i64,
) -> SkillRow {
    SkillRow {
        uid,
        name,
        element,
        damage_mode,
        total_value: stats.total as f64,
        value_per_sec: rate_per_sec(stats.total, elapsed_secs),
        value_pct: ratio_pct(stats.total, denominator),
        crit_rate: ratio_count_pct(stats.crit_count, stats.hit_count),
        crit_value_rate: ratio_pct(stats.crit_value, stats.total),
        lucky_rate: ratio_count_pct(stats.lucky_count, stats.hit_count),
        lucky_value_rate: ratio_pct(stats.lucky_value, stats.total),
        hits: stats.hit_count as f64,
        hits_per_minute: rate_per_minute(stats.hit_count, elapsed_secs),
        time_series: Vec::new(),
    }
}

/// プレイヤーのスキル内訳を指標別に構築する。履歴保存では時系列を持たせず、
/// ライブ/計測結果では既存のスキル別時系列も含める。
fn build_skill_rows_for_player(
    player: &crate::engine::entity::Entity,
    elapsed_secs: f64,
    is_heal: bool,
    include_time_series: bool,
) -> Vec<SkillRow> {
    let player_stats = if is_heal {
        &player.heal_stats
    } else {
        &player.dmg_stats
    };
    let skill_stats_map = if is_heal {
        &player.skill_uid_to_heal_stats
    } else {
        &player.skill_uid_to_dps_stats
    };

    let mut skill_rows: Vec<SkillRow> = skill_stats_map
        .iter()
        .map(|(&skill_uid, skill_stat)| {
            let meta = player.skill_meta.get(&skill_uid).copied().unwrap_or_default();
            let mut row = skill_row_for(
                f64::from(skill_uid),
                get_skill_name(skill_uid),
                meta.property,
                meta.damage_mode,
                skill_stat,
                elapsed_secs,
                player_stats.total,
            );
            if include_time_series && !is_heal {
                row.time_series = player
                    .skill_time_series
                    .get(&skill_uid)
                    .map(|d| d.iter().cloned().collect())
                    .unwrap_or_default();
            }
            row
        })
        .collect();
    sort_skill_rows_desc(&mut skill_rows);
    skill_rows
}

/// 集計指標の種別。`get_header_info` / `get_skills` の共通引数で、どの統計を集計対象にするかを
/// 表す（旧実装はどちらも生の `tab: i32` を受けタブ番号→意味のマッピングを別々に持っていたため、
/// タブ追加時に片方だけ直しても動いてしまう乖離があった）。slint-app 側の `tab_stat` がタブ番号
/// からこの型へ変換する唯一の入口。
#[derive(Debug, Clone, Copy)]
pub enum StatType {
    Dmg,
    DmgBossOnly,
    Heal,
    DmgTaken,
}

/// 指標(タブ)に対応する `Entity` 側の集計。
fn entity_stats_for(entity: &crate::engine::entity::Entity, stat: StatType) -> &CombatStats {
    match stat {
        StatType::Dmg => &entity.dmg_stats,
        StatType::DmgBossOnly => &entity.dmg_stats_boss_only,
        StatType::Heal => &entity.heal_stats,
        StatType::DmgTaken => &entity.dmg_taken_stats,
    }
}

/// 自分のみ計測が効いているときの自分のプレイヤー UID。
///
/// 通常モードでは常に `None`（計測ボタンで始めた計測のあいだだけ効く）。自キャラが未確定なら
/// `None` を返し、呼び出し側は絞り込み無しで従来どおり全員を扱う。黙って空の画面になるより、
/// 絞り込みが効いていないほうがまだ読める。
fn self_only_uid(encounter: &Encounter) -> Option<i64> {
    if !encounter.measure_scope().self_only {
        return None;
    }
    // 「自分」の導出規則は Encounter::self_player_uid が唯一の定義。取り込み側のゲート
    // （processor）と同じ人を指す必要があるため、ここで別に書かない。
    encounter.self_player_uid()
}

/// `self_only_uid` の結果に対して、そのプレイヤーを表示してよいか。
/// 行フィルタ・内訳ガード・結果モーダルの3箇所が同じ述語を共有する（極性を都度書き分けると、
/// 一箇所の反転ミスに気付けない）。
fn visible_under(only_uid: Option<i64>, player_uid: i64) -> bool {
    only_uid.is_none_or(|uid| uid == player_uid)
}

/// 自分のみ計測中に、指定プレイヤーの内訳ビューを表示してよいか。
/// 内訳を返す3本の関数と、開いたままのドリルを畳む UI 側が共有する唯一の判定。
fn breakdown_visible_locked(encounter: &Encounter, player_uid: i64) -> bool {
    visible_under(self_only_uid(encounter), player_uid)
}

/// [`breakdown_visible_locked`] のロック付き版。UI が false を見たらドリルを一覧へ戻す。
/// 一覧から行が消えても、消える前に開いていたドリルは UI 側に残り、放っておくと他プレイヤーの
/// 内訳がライブ更新され続ける（Err で握ると直前の行が画面に残る）。
pub fn breakdown_visible(enc: &EncounterMutex, player_uid: i64) -> bool {
    with_lock_or(enc, "breakdown_visible", true, |e| {
        breakdown_visible_locked(e, player_uid)
    })
}

/// [`breakdown_visible_locked`] の Result 版。内訳を返す3本の関数が先頭で通す。
fn reject_other_player_while_self_only(encounter: &Encounter, player_uid: i64) -> Result<(), String> {
    if breakdown_visible_locked(encounter, player_uid) {
        return Ok(());
    }
    Err(format!(
        "self-only measurement in progress; breakdown for uid {player_uid} is hidden"
    ))
}

/// 表示の母集団となる「全体の集計」。シェア率の分母とヘッダの合計値がここから出る。
///
/// 自分のみ計測が効いているあいだは自分の `Entity` の集計へ差し替える。集計そのものは全員ぶん
/// 取り続けているので、計測を抜ければ元へ戻る（取り込み時に他人を捨てると復元できない）。
/// `get_header_info` と `build_players_window_unsorted` が同じ選択規則を共有するための単一定義。
fn total_stats_for(encounter: &Encounter, stat: StatType) -> &CombatStats {
    if let Some(uid) = self_only_uid(encounter) {
        // 自分の Entity がまだ無いあいだは 0 を返す。全体の合計へフォールバックすると、
        // 行は自分だけに絞られているのにヘッダだけ全員の合計、という食い違いが出る
        // （行フィルタと母集団は同じ only_uid から導く、という約束が破れる）。
        static EMPTY: CombatStats = CombatStats::ZERO;
        return encounter
            .entities
            .get(&EntityKey::player(uid))
            .map_or(&EMPTY, |me| entity_stats_for(me, stat));
    }
    match stat {
        StatType::Dmg => &encounter.dmg_stats,
        StatType::DmgBossOnly => &encounter.dmg_stats_boss_only,
        StatType::Heal => &encounter.heal_stats,
        StatType::DmgTaken => &encounter.dmg_taken_stats,
    }
}

// ─── Header ──────────────────────────────────────────────────────────────────

/// `stat` に応じて合計DPS/合計値を切り替える。
pub fn get_header_info(enc: &EncounterMutex, stat: StatType) -> HeaderInfo {
    with_lock_or(enc, "get_header_info", HeaderInfo::default(), |encounter| {
        let selected = selected_uid::get();
        if selected.is_some() && !encounter.has_selected_participant {
            return HeaderInfo::default();
        }

        let now = crate::engine::processor::now_ms();
        let elapsed_ms = combat_elapsed_ms(encounter, now);
        let elapsed_secs = elapsed_ms as f64 / 1000.0;

        let stats = total_stats_for(encounter, stat);

        HeaderInfo {
            total_dps: rate_per_sec(stats.total, elapsed_secs),
            total_dmg: stats.total as f64,
            elapsed_ms: elapsed_ms as f64,
            time_last_combat_packet_ms: encounter.time_last_combat_packet_ms as f64,
        }
    })
}

// ─── Players windows ─────────────────────────────────────────────────────────

/// 現在の分母（秒）を算出する（`combat_elapsed_ms` を `now` 込みで呼ぶ定型の集約）。
fn live_elapsed_secs(encounter: &Encounter) -> f64 {
    let now = crate::engine::processor::now_ms();
    combat_elapsed_ms(encounter, now) as f64 / 1000.0
}

/// バフ/イマジンタイマーのオーバーレイが使う名簿順（プレイヤー UID の降順）。
///
/// **自分のみ計測の射影を通さない**。オーバーレイの名簿は「誰の残時間を並べるか」であって
/// DPS 一覧の表示対象とは別の関心事で、`timer_roster` はこの列を順序ではなく所属の決定にも
/// 使う（main.rs 参照）。射影を通すと、自分のみ計測のあいだ PT メンバーのイマジンタイマーが
/// 黙って消える。行を組まずキーと合計だけ見るので、一覧の再構築より安い。
pub fn get_roster_uids(enc: &EncounterMutex, stat: StatType) -> Vec<f64> {
    with_lock_or(enc, "get_roster_uids", Vec::new(), |encounter| {
        let mut rows: Vec<(i64, i64)> = encounter
            .entities
            .iter()
            .filter(|(key, _)| key.is_player())
            .map(|(key, entity)| (key.player_uid(), entity_stats_for(entity, stat).total))
            .filter(|(_, total)| *total > 0)
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        rows.into_iter().map(|(uid, _)| uid as f64).collect()
    })
}

pub fn get_dps_players(enc: &EncounterMutex) -> PlayersWindow {
    let mut window = with_lock_or(enc, "get_dps_players", PlayersWindow::default(), |e| {
        let elapsed_secs = live_elapsed_secs(e);
        build_players_window_unsorted(&*e, StatType::Dmg, true, elapsed_secs)
    });
    sort_player_rows_desc(&mut window.player_rows);
    window
}

pub fn get_dps_boss_players(enc: &EncounterMutex) -> PlayersWindow {
    let mut window = with_lock_or(enc, "get_dps_boss_players", PlayersWindow::default(), |e| {
        let elapsed_secs = live_elapsed_secs(e);
        build_players_window_unsorted(&*e, StatType::DmgBossOnly, true, elapsed_secs)
    });
    sort_player_rows_desc(&mut window.player_rows);
    window
}

pub fn get_heal_players(enc: &EncounterMutex) -> PlayersWindow {
    let mut window = with_lock_or(enc, "get_heal_players", PlayersWindow::default(), |e| {
        let elapsed_secs = live_elapsed_secs(e);
        build_players_window_unsorted(&*e, StatType::Heal, true, elapsed_secs)
    });
    sort_player_rows_desc(&mut window.player_rows);
    window
}

pub fn get_dmg_taken_players(enc: &EncounterMutex) -> PlayersWindow {
    let mut window = with_lock_or(enc, "get_dmg_taken_players", PlayersWindow::default(), |e| {
        let elapsed_secs = live_elapsed_secs(e);
        build_players_window_unsorted(&*e, StatType::DmgTaken, true, elapsed_secs)
    });
    sort_player_rows_desc(&mut window.player_rows);
    window
}

pub fn get_dmg_taken_attackers(
    enc: &EncounterMutex,
    player_uid: i64,
) -> Result<SkillsWindow, String> {
    let encounter = enc.lock().map_err(|e| format!("Lock poisoned: {e}"))?;

    reject_other_player_while_self_only(&encounter, player_uid)?;

    let Some(player) = encounter.entities.get(&EntityKey::player(player_uid)) else {
        return Err(format!("Could not find player with uid {player_uid}"));
    };

    let elapsed_secs = live_elapsed_secs(&encounter);

    let player_stats = &player.dmg_taken_stats;
    let encounter_stats = total_stats_for(&encounter, StatType::DmgTaken);

    let inspected_player = make_player_row(
        player_uid,
        player.name.as_deref().unwrap_or(""),
        player.class,
        player.class_spec,
        player.ability_score,
        player.season_level,
        player.season_strength,
        player_stats,
        encounter_stats,
        elapsed_secs,
        // 被ダメタブ: active_dmg_time は与ダメ専用の実働時間なので「被ダメ量÷自分が
        // 殴っていた時間」という定義の無い値になる。None にして0扱いにする。
        None,
        &player.dmg_taken_time_series,
        ConsumableTimes::default(), // inspected_player 見出しは食事/シロップ非表示
        // 見出しは使用イマジンを強制表示
        format_imagine_suffix(&player.imagine_display_labels()),
        format_role_skill_suffix(&player.role_skill_imagine_labels()),
    );

    let mut top_value = 0.0_f64;
    let mut skill_rows: Vec<SkillRow> = player
        .attacker_uid_to_dmg_taken_stats
        .iter()
        .map(|(&attacker_key, stats)| {
            top_value = top_value.max(stats.total as f64);
            skill_row_for(
                // 行 id は UUID（種別コード込み）。`get_dmg_taken_skills` へそのまま往復させる
                // ため、`uuid >> 16` に潰さない（潰すとモンスターと召喚体の行が同じ id になる）。
                attacker_key.uuid() as f64,
                attacker_display_name(&encounter, attacker_key),
                0,
                0,
                stats,
                elapsed_secs,
                player_stats.total,
            )
        })
        .collect();

    sort_skill_rows_desc(&mut skill_rows);

    Ok(SkillsWindow {
        inspected_player,
        skill_rows,
        local_player_uid: encounter.local_player_uid as f64,
        top_value,
    })
}

/// `attacker_uuid` は [`get_dmg_taken_attackers`] が行 id として返した UUID
/// （種別コード込み）をそのまま受け取る。プレイヤー UID ではない。
pub fn get_dmg_taken_skills(
    enc: &EncounterMutex,
    player_uid: i64,
    attacker_uuid: i64,
) -> Result<SkillsWindow, String> {
    let attacker_key = EntityKey::from_uuid(attacker_uuid);
    let encounter = enc.lock().map_err(|e| format!("Lock poisoned: {e}"))?;

    reject_other_player_while_self_only(&encounter, player_uid)?;

    let Some(player) = encounter.entities.get(&EntityKey::player(player_uid)) else {
        return Err(format!("Could not find player with uid {player_uid}"));
    };

    let elapsed_secs = live_elapsed_secs(&encounter);

    let attacker_total = player
        .attacker_uid_to_dmg_taken_stats
        .get(&attacker_key)
        .map(|s| s.total as f64)
        .unwrap_or(0.0);
    let encounter_stats = total_stats_for(&encounter, StatType::DmgTaken);

    let player_stats = &player.dmg_taken_stats;

    let inspected_player = make_player_row(
        player_uid,
        player.name.as_deref().unwrap_or(""),
        player.class,
        player.class_spec,
        player.ability_score,
        player.season_level,
        player.season_strength,
        player_stats,
        encounter_stats,
        elapsed_secs,
        // 被ダメタブ: get_dmg_taken_attackers と同じ理由で None（定義の無い値を渡さない）。
        None,
        &player.dmg_taken_time_series,
        ConsumableTimes::default(), // inspected_player 見出しは食事/シロップ非表示
        // 見出しは使用イマジンを強制表示
        format_imagine_suffix(&player.imagine_display_labels()),
        format_role_skill_suffix(&player.role_skill_imagine_labels()),
    );

    let attacker_total_i64 = attacker_total as i64;
    let mut top_value = 0.0_f64;
    let mut skill_rows: Vec<SkillRow> = player
        .attacker_skill_to_dmg_taken_stats
        .iter()
        .filter(|((key, _), _)| *key == attacker_key)
        .map(|((_, skill_uid), stats)| {
            top_value = top_value.max(stats.total as f64);
            let meta = player.skill_meta.get(skill_uid).copied().unwrap_or_default();
            skill_row_for(
                f64::from(*skill_uid),
                crate::engine::skill_names::get_skill_name(*skill_uid),
                meta.property,
                meta.damage_mode,
                stats,
                elapsed_secs,
                attacker_total_i64,
            )
        })
        .collect();

    sort_skill_rows_desc(&mut skill_rows);

    Ok(SkillsWindow {
        inspected_player,
        skill_rows,
        local_player_uid: encounter.local_player_uid as f64,
        top_value,
    })
}

fn attacker_display_name(encounter: &Encounter, attacker_key: EntityKey) -> String {
    // 名前が引けないときの短縮表示。3通りの分岐で同じ式を書き直さないよう1箇所で導出する。
    let short = attacker_key.player_uid() & 0xFFFF;
    let Some(e) = encounter.entities.get(&attacker_key) else {
        return format!("#{short}");
    };
    if attacker_key.is_player() {
        return e
            .name
            .clone()
            .unwrap_or_else(|| format!("プレイヤー#{short}"));
    }
    if let Some(mid) = e.monster_id {
        if let Some(name) = crate::engine::monster_names::get_boss_name(mid) {
            return name;
        }
        return format!("モンスター#{mid}");
    }
    format!("#{short}")
}

/// ロック保持中に呼ぶ。ソートはロック解放後に呼び出し元で行う。
/// `include_idle_consumable` が true なら、ダメージ0でも食事/シロップを持つ
/// プレイヤー行を含める（ライブ表示用。戦闘前/非ダメージの使用者を表示）。
/// 履歴スナップショットでは false にしてダメージ実績行のみ残す。
/// `elapsed_secs` は呼び出し元が算出した分母（秒）。内部で再計算しない＝ヘッダ等と
/// 必ず同じ値を使わせる（build_encounter_snapshot の3分計測固定窓オーバーライドを
/// ここでも反映させるため）。
/// ダメージ0＋食事/シロップ持ちの特例行は、`runtime_settings::party_only_consumables()`
/// が true のとき PT メンバー（＋自分）限定にする（IMAGINE_ONLY_MODE と同じ流儀で
/// atomic を直接読む。消費箇所がここ1つのため呼び出し元へは通さない）。
/// `include_idle_consumable=false` のときは特例行自体が出ないため参照されない。
/// ダメージ>0の行は常に無条件表示（この設定と無関係）。
fn build_players_window_unsorted(
    encounter: &Encounter,
    stat_type: StatType,
    include_idle_consumable: bool,
    elapsed_secs: f64,
) -> PlayersWindow {
    let party_only_idle_consumable = runtime_settings::party_only_consumables();
    let selected = selected_uid::get();
    if selected.is_some() && !encounter.has_selected_participant {
        return PlayersWindow::default();
    }

    let encounter_stats = total_stats_for(encounter, stat_type);
    // 自分のみ計測中は自分の行だけを出す。母集団(encounter_stats)も同じ判定から導いているので、
    // シェア率の分子と分母がずれない。
    let only_uid = self_only_uid(encounter);

    let mut window = PlayersWindow {
        player_rows: Vec::new(),
        local_player_uid: selected.unwrap_or(encounter.local_player_uid) as f64,
        top_value: 0.0,
    };

    for (&entity_key, entity) in &encounter.entities {
        let entity_stats = entity_stats_for(entity, stat_type);
        // 推移グラフ・固定基準バーが指標(タブ)と一致した系列を見るように、stat_type に応じて
        // 時系列も切り替える（boss-onlyは専用系列を持たず通常の与ダメ系列を流用する）。
        let entity_time_series = match stat_type {
            StatType::Dmg | StatType::DmgBossOnly => &entity.time_series,
            StatType::Heal => &entity.heal_time_series,
            StatType::DmgTaken => &entity.dmg_taken_time_series,
        };
        // 有効DPSの分母(active_dmg_time)は与ダメイベント専用に積算されているため、与ダメ系
        // (Dmg/DmgBossOnly)以外では意味を持たない（回復量÷与ダメ実働時間、のような定義の
        // 無い値になってしまう）。Heal/DmgTaken では None を渡す(make_player_rowが0扱いにする)。
        let active_dmg_ms = matches!(stat_type, StatType::Dmg | StatType::DmgBossOnly)
            .then_some(entity.active_dmg_time.active_ms);

        if !entity_key.is_player() {
            continue;
        }
        // ここから先はプレイヤー確定なので、キーの上位ビットはプレイヤー UID として一意。
        let entity_uid = entity_key.player_uid();

        if !visible_under(only_uid, entity_uid) {
            continue;
        }

        let pc = encounter.consumables.get(&entity_uid);
        let has_consumable = pc.is_some_and(|c| c.food.is_some() || c.syrup.is_some());
        // ダメージ0の行は通常除外するが、食事/シロップ使用者はライブ表示で残す。
        // party_only_idle_consumable が true のときは、この特例行を自分/PTメンバーに限る
        // （AOI appear 同期で街中の無関係プレイヤー全員が並ぶのを防ぐ）。
        // 表示可否の判定はここ1箇所（entity_stats.total>0 の行はこの条件と無関係に無条件表示）。
        let is_party_visible =
            entity_uid == encounter.local_player_uid || encounter.team.is_member(entity_uid);
        let idle_consumable_visible = include_idle_consumable
            && has_consumable
            && (!party_only_idle_consumable || is_party_visible);
        // 自分のみ計測中は、与ダメージ0でも自分の行を残す。回復専業や被ダメージ計測では
        // dmg_stats が0のまま確定しうるが、ここで落とすと player_rows が空になり、履歴 push も
        // 自己ベスト判定も飛んだうえ結果モーダルだけが 0 DPS で開く。
        let keep_self_row = only_uid == Some(entity_uid);
        if entity_stats.total == 0 && !idle_consumable_visible && !keep_self_row {
            continue;
        }

        window.top_value = window.top_value.max(entity_stats.total as f64);

        let now = crate::engine::processor::now_ms();
        let consumable = ConsumableTimes {
            food_remaining_ms: pc
                .and_then(|c| c.food)
                .map(|t| t.remaining_ms(now).max(0) as f64)
                .unwrap_or(0.0),
            food_duration_ms: pc.and_then(|c| c.food).map(|t| t.duration_ms as f64).unwrap_or(0.0),
            food_base_id: pc.and_then(|c| c.food).map(|t| t.base_id).unwrap_or(0),
            syrup_remaining_ms: pc
                .and_then(|c| c.syrup)
                .map(|t| t.remaining_ms(now).max(0) as f64)
                .unwrap_or(0.0),
            syrup_duration_ms: pc
                .and_then(|c| c.syrup)
                .map(|t| t.duration_ms as f64)
                .unwrap_or(0.0),
            syrup_base_id: pc.and_then(|c| c.syrup).map(|t| t.base_id).unwrap_or(0),
        };
        let row = make_player_row(
            entity_uid,
            entity.name.as_deref().unwrap_or(""),
            entity.class,
            entity.class_spec,
            entity.ability_score,
            entity.season_level,
            entity.season_strength,
            entity_stats,
            encounter_stats,
            elapsed_secs,
            active_dmg_ms,
            entity_time_series,
            consumable,
            format_imagine_suffix(&entity.imagine_display_labels()),
            format_role_skill_suffix(&entity.role_skill_imagine_labels()),
        );
        window.player_rows.push(row);
    }

    window
}

#[derive(Clone, Copy, Default)]
struct ConsumableTimes {
    food_remaining_ms: f64,
    food_duration_ms: f64,
    food_base_id: i32,
    syrup_remaining_ms: f64,
    syrup_duration_ms: f64,
    syrup_base_id: i32,
}

/// 装備中バトルイマジンの表示ラベル（`imagine_display_labels`＝凸数判明時は「名前(N)」）から
/// "-ティナ(5)/アルーナ" 形式の表示用サフィックスを作る。
/// 未装備なら空文字（テンプレート展開・見出し強制表示のいずれも自然に何も付かない）。
/// 装備枠は2つ（[`MAX_IMAGINE_NAMES`]）なので、万一それ以上溜まっていても表示は先頭 MAX 件に丸める
/// （検知/キャッシュ側で既に丸めているが、表示層でも保険をかけて「3つ以上」を出さない）。
fn format_imagine_suffix(imagine_names: &[String]) -> String {
    if imagine_names.is_empty() {
        return String::new();
    }
    let shown = &imagine_names[..imagine_names.len().min(MAX_IMAGINE_NAMES)];
    format!("-{}", shown.join("/"))
}

/// ロールスキル(簡易版バトルイマジン、最大 [`MAX_ROLE_SKILL_IMAGINES`] 件)の表示ラベルから
/// " (R:アルーナ(3)/ファルファラ)" 形式のサフィックスを作る（無ければ空文字）。
/// 先頭のスペースは実イマジン部と直結したときの区切り（既定テンプレートは `{imagine}{roleSkill}`）。
/// 実イマジン側と別トークンなので、装備中の実イマジン2枠とロールスキルは "(R:" 表記に加えて
/// テンプレート上でも独立に扱える（例: "-ゴーストカニクモ/ティナ (R:アルーナ(3)/ファルファラ)"）。
/// 実イマジン側と同様、万一それ以上溜まっていても表示は先頭 MAX 件に丸める（検知/キャッシュ側で
/// 既に丸めているが、表示層でも保険をかける）。
fn format_role_skill_suffix(role_skill_labels: &[String]) -> String {
    if role_skill_labels.is_empty() {
        return String::new();
    }
    let shown = &role_skill_labels[..role_skill_labels.len().min(MAX_ROLE_SKILL_IMAGINES)];
    format!(" (R:{})", shown.join("/"))
}

fn make_player_row(
    uid: i64,
    name: &str,
    class: Option<Class>,
    class_spec: Option<ClassSpec>,
    ability_score: Option<i32>,
    season_level: Option<i32>,
    season_strength: Option<i32>,
    entity_stats: &CombatStats,
    encounter_stats: &CombatStats,
    elapsed_secs: f64,
    // 有効DPS（実働時間ベース）の分母。`entity_stats` が与ダメ系（Dmg/DmgBossOnly）以外の
    // 指標を指しているとき（回復・被ダメ集計）は None を渡すこと。`Entity::active_dmg_time`は
    // 与ダメイベント専用に積算されており、回復量や被ダメ量と組み合わせても定義のない値になる
    // （回復量÷与ダメ実働時間、被ダメ量÷自分が殴っていた時間、等）。None は0扱いにする。
    active_dmg_ms: Option<u128>,
    time_series: &VecDeque<TimeSeriesPoint>,
    consumable: ConsumableTimes,
    imagine_suffix: String,
    role_skill_suffix: String,
) -> PlayerRow {
    // 表示言語に応じた名前（ja 以外は en。zh/ko は保留中のため en にフォールバック）。
    let lang = runtime_settings::display_lang();
    let name_resolved = !name.is_empty();
    let display_name = if name_resolved {
        name.to_string()
    } else if lang == Lang::Ja {
        format!("プレイヤー#{}", uid & 0xFFFF)
    } else {
        format!("Player#{}", uid & 0xFFFF)
    };
    let class = class.unwrap_or(Class::Unknown);
    let class_name = match lang {
        Lang::Ja => class.name_ja(),
        _ => class.name_en(),
    }
    .to_string();
    // 特化が未判明（None/Unknown）のときは空文字にして表示側で省略させる。
    let class_spec_name = match class_spec {
        Some(s) if s != ClassSpec::Unknown => match lang {
            Lang::Ja => s.name_ja(),
            _ => s.name(),
        }
        .to_string(),
        _ => String::new(),
    };

    PlayerRow {
        uid: uid as f64,
        name: display_name,
        name_resolved,
        class_name,
        class_spec_name,
        ability_score: f64::from(ability_score.unwrap_or(-1)),
        season_level: f64::from(season_level.unwrap_or(-1)),
        season_strength: f64::from(season_strength.unwrap_or(-1)),
        total_value: entity_stats.total as f64,
        value_per_sec: rate_per_sec(entity_stats.total, elapsed_secs),
        value_pct: ratio_pct(entity_stats.total, encounter_stats.total),
        crit_rate: ratio_count_pct(entity_stats.crit_count, entity_stats.hit_count),
        crit_value_rate: ratio_pct(entity_stats.crit_value, entity_stats.total),
        lucky_rate: ratio_count_pct(entity_stats.lucky_count, entity_stats.hit_count),
        lucky_value_rate: ratio_pct(entity_stats.lucky_value, entity_stats.total),
        hits: entity_stats.hit_count as f64,
        hits_per_minute: rate_per_minute(entity_stats.hit_count, elapsed_secs),
        // 分母は実測スパン(elapsed_secs)ではなく実働時間(active_dmg_ms)。ただし
        // ActiveTime::record_event は初回イベントにも猶予(500ms)を積むため、実測スパンが
        // それより短い極端なケースでは実働時間が実測スパンを上回り得る→elapsed_secs で
        // 上限クランプする（「有効DPSが通常DPSを下回らない」性質を保つ）。None（回復/被ダメ等
        // 定義の無い指標）は0.0のまま（rate_per_sec が active_secs<=0.0 を0.0へ丸める）。
        active_value_per_sec: active_dmg_ms
            .map(|ms| {
                let bound_ms = (elapsed_secs.max(0.0) * 1000.0) as u128;
                rate_per_sec(entity_stats.total, ms.min(bound_ms) as f64 / 1000.0)
            })
            .unwrap_or(0.0),
        food_remaining_ms: consumable.food_remaining_ms,
        food_duration_ms: consumable.food_duration_ms,
        food_base_id: consumable.food_base_id,
        syrup_remaining_ms: consumable.syrup_remaining_ms,
        syrup_duration_ms: consumable.syrup_duration_ms,
        syrup_base_id: consumable.syrup_base_id,
        imagine_suffix,
        role_skill_suffix,
        time_series: time_series.iter().cloned().collect(),
    }
}

// ─── Skills window ───────────────────────────────────────────────────────────

/// `stat`(Heal のみ回復基準。DmgTaken/DmgBossOnly を渡された場合も含め、それ以外はすべて
/// 与ダメ基準へフォールバックする。被ダメの技別内訳は `get_dmg_taken_attackers` へ振り分ける
/// 想定で、本関数が DmgTaken を受け取ることは無い)に応じてスキル別内訳を取得する。
///
/// 総計・内訳(SkillRow)は指標別に既に集計済みの `skill_uid_to_heal_stats` を使えるため
/// heal タブでも正しい値を返せるが、**スキル別の推移グラフ(row.time_series)は現状
/// 与ダメ専用の `skill_time_series` しか採取していない**ため、heal タブでは空にする
/// （dmg 専用マップを heal の skill_uid で誤って引くと無関係なスキルの系列が出かねないため）。
/// heal 版のスキル別時系列が要るなら `skill_heal_time_series: HashMap<i32, VecDeque<..>>` の
/// 新設とサンプリング側の対応が要る（新規データ構造の追加＝別スコープ）。
pub fn get_skills(
    enc: &EncounterMutex,
    player_uid: i64,
    stat: StatType,
) -> Result<SkillsWindow, String> {
    let encounter = enc.lock().map_err(|e| format!("Lock poisoned: {e}"))?;

    reject_other_player_while_self_only(&encounter, player_uid)?;

    let Some(player) = encounter.entities.get(&EntityKey::player(player_uid)) else {
        return Err(format!("Could not find player with uid {player_uid}"));
    };

    let elapsed_secs = live_elapsed_secs(&encounter);

    let is_heal = matches!(stat, StatType::Heal);
    let player_stats = if is_heal { &player.heal_stats } else { &player.dmg_stats };
    // 分母は player_stats と同じ軸から採る（DmgTaken タブでも player 側は dmg_stats を見るため、
    // stat をそのまま渡すと分子=与ダメ・分母=被ダメになってしまう）。
    let encounter_stats =
        total_stats_for(&encounter, if is_heal { StatType::Heal } else { StatType::Dmg });
    let player_time_series = if is_heal { &player.heal_time_series } else { &player.time_series };
    // heal タブは回復量÷与ダメ実働時間という定義の無い値になるため None（0扱い）。
    // それ以外(与ダメ基準へフォールバックする各種)は player.active_dmg_time が対応する。
    let active_dmg_ms = if is_heal { None } else { Some(player.active_dmg_time.active_ms) };

    let inspected_player = make_player_row(
        player_uid,
        player.name.as_deref().unwrap_or(""),
        player.class,
        player.class_spec,
        player.ability_score,
        player.season_level,
        player.season_strength,
        player_stats,
        encounter_stats,
        elapsed_secs,
        active_dmg_ms,
        player_time_series,
        ConsumableTimes::default(), // inspected_player 見出しは食事/シロップ非表示
        // 見出しは使用イマジンを強制表示
        format_imagine_suffix(&player.imagine_display_labels()),
        format_role_skill_suffix(&player.role_skill_imagine_labels()),
    );

    let skill_rows = build_skill_rows_for_player(player, elapsed_secs, is_heal, true);
    let top_value = skill_rows
        .iter()
        .map(|row| row.total_value)
        .fold(0.0_f64, f64::max);

    let skill_window = SkillsWindow {
        inspected_player,
        skill_rows,
        local_player_uid: encounter.local_player_uid as f64,
        top_value,
    };
    drop(encounter);

    Ok(skill_window)
}

// ─── Control commands ─────────────────────────────────────────────────────────

pub fn reset_encounter(enc: &EncounterMutex) {
    with_lock_or(enc, "reset_encounter", (), |encounter| {
        // M2/M3/M5計測: 手動リセットでもロールオーバー時と同じサマリーを出す
        // （通常モードのタイムアウト待ちに限定せず、ここで確実に読めるようにする）。
        crate::probe::log_and_reset_encounter_summary();
        encounter.clear_combat_stats();
        // 食事/シロップはゲーム内効果が継続するため手動リセットでは消さない
        // （消えるのは自然失効・履歴クリアのみ）。
        // 3分計測中に初期化された場合は計測そのものを破棄する。
        // measure_mode を残すと armed_at_ms が古いまま固定され、
        // 次の計測クリックが開始ではなくキャンセルとして処理されたり、
        // 締切が早まり計測時間が3分未満になる不整合が起きる。
        encounter.measure_mode = crate::engine::encounter::MeasureMode::Normal;
        info!("Encounter reset");
    });
}

/// 起動時に永続化された食事/シロップ状態を Encounter へ復元する（1回だけ呼ぶ）。
pub fn load_consumables(enc: &EncounterMutex) {
    let now = crate::engine::processor::now_ms();
    let loaded = crate::engine::consumables::load(now);
    if let Ok(mut e) = enc.lock() {
        e.consumables = loaded;
    }
}

/// 現在の食事/シロップ状態をディスクへ保存（変化時のみ書き込み）。終了時に呼ぶ。
pub fn save_consumables(enc: &EncounterMutex) {
    let snapshot = match enc.lock() {
        Ok(e) => e.consumables.clone(),
        Err(_) => return,
    };
    crate::engine::consumables::save_if_changed(&snapshot);
}

/// 食事/シロップ残時間ストアを buff_tracker の観測で更新（毎poll呼ぶ）。
/// clear_combat_stats で消えても保持し続け、自然失効分はここで除去する。
/// 更新後の状態は変化時のみディスク永続化する（I/O はロック外で実施）。
pub fn refresh_consumables(enc: &EncounterMutex) {
    let snapshot = {
        let Ok(mut e) = enc.lock() else {
            return;
        };
        let now = crate::engine::processor::now_ms();
        let e = &mut *e;
        crate::engine::consumables::refresh(&mut e.consumables, &e.buff_tracker, now);
        e.consumables.clone()
    };
    crate::engine::consumables::save_if_changed(&snapshot);
}

/// 食事/シロップ残時間ストアを全消去（履歴クリア時など）。
pub fn clear_consumables(enc: &EncounterMutex) {
    if let Ok(mut e) = enc.lock() {
        e.consumables.clear();
    }
}

pub fn toggle_pause(enc: &EncounterMutex) {
    with_lock_or(enc, "toggle_pause", (), |encounter| {
        encounter.is_paused = !encounter.is_paused;
        info!("Encounter paused: {}", encounter.is_paused);
    });
}

/// 現在の一時停止状態（UI のボタン表示用）。
pub fn is_paused(enc: &EncounterMutex) -> bool {
    enc.lock().map(|e| e.is_paused).unwrap_or(false)
}

// ─── Encounter snapshot ───────────────────────────────────────────────────────

/// `now` はロールオーバー確定・3分計測確定いずれの呼び出し元も既に持っている `ts`/`now_ms()`
/// をそのまま渡す（テストでも実時刻を経由せず注入できる）。分母は `combat_elapsed_ms` に
/// 集約済み＝ヘッダ(total_dps)・プレイヤー行(player_rows)・スキル内訳(player_skill_rows)は
/// すべてここで一度だけ算出した `elapsed_secs` から導出され、互いに食い違わない。
pub fn build_encounter_snapshot(encounter: &Encounter, now: u128) -> EncounterSnapshot {
    let elapsed_ms = combat_elapsed_ms(encounter, now);
    let elapsed_secs = elapsed_ms as f64 / 1000.0;
    // ヘッダ・プレイヤー行と同じ射影を通す（自分のみ計測中は自分の集計が合計になる）。
    let total_dmg = total_stats_for(encounter, StatType::Dmg).total as f64;
    // 折れ線も合計に対応した系列へ揃える。エンティティ別系列は entity.dmg_stats の差分から
    // 採られているので、自分の Entity のものがそのまま自分だけの推移になる。
    let self_series = self_only_uid(encounter)
        .and_then(|uid| encounter.entities.get(&EntityKey::player(uid)))
        .map(|me| &me.time_series);
    let total_dps = if elapsed_secs > 0.0 {
        total_dmg / elapsed_secs
    } else {
        0.0
    };

    let mut window = build_players_window_unsorted(encounter, StatType::Dmg, false, elapsed_secs);
    window.player_rows.sort_by(|a, b| {
        b.total_value
            .partial_cmp(&a.total_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let player_skill_rows = window
        .player_rows
        .iter()
        .filter_map(|player_row| {
            let player_uid = player_row.uid as i64;
            let player = encounter.entities.get(&EntityKey::player(player_uid))?;
            let skill_rows = build_skill_rows_for_player(player, elapsed_secs, false, false);
            if skill_rows.is_empty() {
                return None;
            }
            Some(PlayerSkillSnapshot {
                player_uid: player_row.uid,
                skill_rows,
            })
        })
        .collect();

    EncounterSnapshot {
        id: 0.0,
        start_ms: encounter.time_fight_start_ms as f64,
        end_ms: encounter.time_last_combat_packet_ms as f64,
        duration_ms: elapsed_ms as f64,
        total_dmg,
        total_dps,
        player_rows: window.player_rows,
        player_skill_rows,
        time_series: self_series
            .unwrap_or(&encounter.time_series)
            .iter()
            .cloned()
            .collect(),
        measure_scope: encounter.measure_scope(),
        participant_player_uids: encounter
            .participant_player_uids
            .iter()
            .map(|&v| v as f64)
            .collect(),
        level_map_id: encounter.fight_level_map_id,
    }
}

// ─── History commands ─────────────────────────────────────────────────────────

pub fn set_combat_exit_timeout(secs: f64) {
    let ms = (secs * 1000.0).max(0.0) as u64;
    crate::engine::runtime_settings::COMBAT_EXIT_TIMEOUT_MS
        .store(ms, std::sync::atomic::Ordering::Relaxed);
}

pub fn set_history_limit(limit: f64) {
    let n = limit.max(0.0) as usize;
    crate::engine::runtime_settings::HISTORY_LIMIT.store(n, std::sync::atomic::Ordering::Relaxed);
    crate::engine::history::trim_to_limit();
}

pub fn get_history() -> Vec<crate::models::EncounterSnapshot> {
    let all = crate::engine::history::snapshot_list();
    let Some(sel) = selected_uid::get() else {
        return all;
    };
    let sel_f64 = sel as f64;
    all.into_iter()
        .filter(|snap| {
            snap.participant_player_uids.is_empty()
                || snap.participant_player_uids.contains(&sel_f64)
        })
        .collect()
}

pub fn set_time_series_config(samples: f64, interval_ms: f64) {
    let n = samples.max(1.0) as usize;
    let i = interval_ms.max(50.0) as u64;
    crate::engine::runtime_settings::TS_SAMPLES.store(n, std::sync::atomic::Ordering::Relaxed);
    crate::engine::runtime_settings::TS_INTERVAL_MS.store(i, std::sync::atomic::Ordering::Relaxed);
}

/// バフ追跡中の player_uid を first-seen 順（各 player が最初にバフ検出された順）で返す。
/// イマジン専用モード（DPS等の集計を回さずバフ追跡だけ動く軽量モード）では、メインDPS一覧が
/// 空でタイマー名簿の導出元にできないため、これを名簿源として使う。
pub fn get_buff_tracked_uids(enc: &EncounterMutex) -> Vec<i64> {
    with_lock_or(enc, "get_buff_tracked_uids", Vec::new(), |e| {
        e.buff_tracker.tracked_player_uids_by_first_seen()
    })
}

pub fn set_imagine_only_mode(enc: &EncounterMutex, enabled: bool) {
    let was_enabled = crate::engine::runtime_settings::IMAGINE_ONLY_MODE
        .swap(enabled, std::sync::atomic::Ordering::Relaxed);
    // 切替時は古い集計結果を残さないようにエンカウンターをクリア
    if was_enabled != enabled {
        if let Ok(mut enc) = enc.lock() {
            enc.clear_combat_stats();
        }
        info!("Imagine-only mode: {enabled}");
    }
}

pub fn clear_history() {
    crate::engine::history::clear();
}

// ─── capture status ──────────────────────────────────────────────────────────

#[derive(serde::Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatusDto {
    /// 0=初期化中 1=観測中 2=開始失敗（capture::status::STATE_*）
    pub state: u8,
    pub packets_total: f64,
    /// 最後に TCP パケットを観測してからの経過 ms（-1.0=未観測）
    pub ms_since_last_packet: f64,
    /// 最後にゲームサーバのパケットを処理してからの経過 ms（-1.0=未観測）
    pub ms_since_last_game_packet: f64,
    /// MAX_SUBNET_CONNECTIONS 到達で新規接続の追跡を諦めた回数（累計）
    pub subnet_cap_hits: f64,
    /// TCP 再組立でギャップ検知→再同期が起きた回数（累計。戦闘データ一部欠落を伴う）
    pub reassembly_gaps: f64,
    /// ディスパッチチャネル満杯で破棄したフレーム数（累計）
    pub dropped_frames: f64,
}

pub fn get_capture_status() -> CaptureStatusDto {
    use crate::capture::status;
    use std::sync::atomic::Ordering;
    let since = |v: u64| status::ms_since(v).map(|ms| ms as f64).unwrap_or(-1.0);
    CaptureStatusDto {
        state: status::state(),
        packets_total: status::PACKETS_TOTAL.load(Ordering::Relaxed) as f64,
        ms_since_last_packet: since(status::LAST_PACKET_UNIX_MS.load(Ordering::Relaxed)),
        ms_since_last_game_packet: since(status::LAST_GAME_PACKET_UNIX_MS.load(Ordering::Relaxed)),
        subnet_cap_hits: status::SUBNET_CAP_HITS.load(Ordering::Relaxed) as f64,
        reassembly_gaps: status::REASSEMBLY_GAPS.load(Ordering::Relaxed) as f64,
        dropped_frames: status::DROPPED_FRAMES.load(Ordering::Relaxed) as f64,
    }
}

/// 自キャラのバフ観測を表示用の (バフ, デバフ) へ整形する。
/// 同一 base_id のインスタンスは1件に束ねる。AddBuff 経路は同じバフの再付与ごとに新しい
/// buff_uuid を振るため、束ねないと同名の行が並ぶ（実測: 自バフ窓に同じバフが2件ずつ）。
/// 代表は残り時間が最長のインスタンス（イマジンタイマー `aggregate_player_buffs` と同規約）。
fn split_self_status(
    snapshots: Vec<crate::engine::buff_tracker::BuffStateSnapshot>,
) -> (Vec<SelfStatusEntry>, Vec<SelfStatusEntry>) {
    use crate::engine::buff_dictionary::{self, DisplayPriority};
    use std::collections::HashMap;

    let mut by_base: HashMap<i32, SelfStatusEntry> = HashMap::new();

    for snap in snapshots {
        if !buff_dictionary::is_visible(snap.base_id) {
            continue;
        }
        let meta = match buff_dictionary::lookup(snap.base_id) {
            Some(m) => *m,
            None => continue,
        };
        let priority_str = match meta.priority {
            DisplayPriority::Hidden => "hidden",
            DisplayPriority::Low => "low",
            DisplayPriority::Normal => "normal",
            DisplayPriority::High => "high",
            DisplayPriority::Alert => "alert",
        };

        let entry = SelfStatusEntry {
            instance_id: snap.buff_uuid as i64,
            base_id: snap.base_id,
            category: meta.category.as_str().to_string(),
            priority: priority_str.to_string(),
            remaining_ms: snap.remaining_ms.max(0),
            duration_ms: snap.duration_ms,
            layer: snap.layer,
            source_config_id: 0,
        };

        match by_base.get(&entry.base_id) {
            Some(kept) if kept.remaining_ms >= entry.remaining_ms => {}
            _ => {
                by_base.insert(entry.base_id, entry);
            }
        }
    }

    let mut buffs = Vec::new();
    let mut debuffs = Vec::new();
    for entry in by_base.into_values() {
        if entry.category == "debuff" {
            debuffs.push(entry);
        } else {
            buffs.push(entry);
        }
    }

    // 残り時間の降順で並べる（残り多い順）。同値は base_id 昇順で確定させる
    // （HashMap 由来の不定順で毎tick 行が入れ替わるのを防ぐ）。
    let by_remaining = |a: &SelfStatusEntry, b: &SelfStatusEntry| {
        b.remaining_ms.cmp(&a.remaining_ms).then(a.base_id.cmp(&b.base_id))
    };
    buffs.sort_by(by_remaining);
    debuffs.sort_by(by_remaining);
    (buffs, debuffs)
}

pub fn get_self_buff_status(enc: &EncounterMutex) -> SelfStatusData {
    use crate::engine::processor::now_ms;

    let (snapshots, now_ms, local_uid) = {
        let mut enc = match enc.lock() {
            Ok(e) => e,
            Err(e) => {
                log::error!("Lock poisoned in get_self_buff_status: {e}");
                return SelfStatusData::default();
            }
        };
        let now = now_ms();
        let uid = enc.local_player_uid;
        if uid == 0 {
            return SelfStatusData::default();
        }
        enc.buff_tracker.gc(now);
        let snaps = enc.buff_tracker.snapshot_for(uid, now);
        (snaps, now, uid)
    };

    let (buffs, debuffs) = split_self_status(snapshots);

    SelfStatusData {
        buffs,
        debuffs,
        now_ms: now_ms as f64,
        local_player_uid: local_uid as f64,
    }
}

/// 自キャラの戦闘ステータスを取得（リアルタイム表示用）。
/// ステータス値はパケット attr 由来（Entity に追従保持）、実測率は当該エンカウンタの
/// 命中データ由来。自キャラ未確定（uid==0）や Entity 不在なら空（uid のみ）を返す。
pub fn get_self_stats(enc: &EncounterMutex) -> SelfStatsData {
    let encounter = match enc.lock() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Lock poisoned in get_self_stats: {e}");
            return SelfStatsData::default();
        }
    };
    let uid = encounter.local_player_uid;
    if uid == 0 {
        return SelfStatsData::default();
    }
    let Some(ent) = encounter.entities.get(&EntityKey::player(uid)) else {
        return SelfStatsData {
            local_player_uid: uid as f64,
            ..Default::default()
        };
    };
    let s = &ent.dmg_stats;
    SelfStatsData {
        local_player_uid: uid as f64,
        has_combat: s.hit_count > 0,
        curr_hp: ent.curr_hp.map(|v| v as f64),
        max_hp: ent.max_hp.map(|v| v as f64),
        attack_power: ent.attack_power,
        magic_attack: ent.magic_attack,
        defense_power: ent.defense_power,
        magic_defense: ent.magic_defense,
        endurance: ent.endurance,
        strength: ent.strength,
        intelligence: ent.intelligence,
        agility: ent.agility,
        ability_score: ent.ability_score,
        season_strength: ent.season_strength,
        attack_speed: ent.attack_speed,
        cast_speed: ent.cast_speed,
        haste: ent.haste,
        lucky: ent.lucky,
        crit_stat: ent.crit_stat,
        versatility: ent.versatility,
        resist: ent.resist,
        dexterity: ent.dexterity,
        crit_dmg: ent.crit_dmg,
        lucky_dmg: ent.lucky_dmg,
        crit_rate_measured: ratio_count_pct(s.crit_count, s.hit_count),
        lucky_rate_measured: ratio_count_pct(s.lucky_count, s.hit_count),
    }
}

// ─── selected_uid コマンド ────────────────────────────────────────────────────

pub fn get_selected_uid() -> Option<f64> {
    selected_uid::get().map(|v| v as f64)
}

/// 指定 UID に対応するゲームクライアントの接続を特定できているか。
/// UID 未指定なら常に true（フィルタ待ちが存在しない）。UID 指定中に false の間は
/// 対象クライアントのパケットを1件も採用していない＝表示が空になるため、UI で区別する。
pub fn selected_conn_resolved(enc: &EncounterMutex) -> bool {
    if selected_uid::get().is_none() {
        return true;
    }
    with_lock_or(enc, "selected_conn_resolved", false, |encounter| {
        encounter.active_connection.is_some()
    })
}

pub fn set_selected_uid(enc: &EncounterMutex, uid: Option<f64>) {
    let uid_i64 = uid.map(|v| v as i64);
    selected_uid::set(uid_i64);
    with_lock_or(enc, "set_selected_uid", (), |encounter| {
        let new_uid = uid_i64.unwrap_or(0);
        // シーン(current_level_map_id)はキャラ単位。前キャラのシーンを新キャラの最初の
        // 戦闘記録に引き継がないよう、自キャラが実際に切り替わるときだけクリアする
        // （同じ UID を再指定しただけなら、既に持っている値を失わせない）。
        if encounter.local_player_uid != new_uid {
            encounter.current_level_map_id = 0;
        }
        encounter.clear_combat_stats();
        encounter.active_connection = None;
        encounter.local_player_uid = new_uid;
        // PT構成はキャラ単位。別キャラへの切替・再ログインで前キャラの PT を引き継がない
        // （processor.rs 側の自動検出は Encounter::set_local_player_uid が条件付きでクリア
        // するが、ここは明示的な手動切替のため無条件でクリアする）。
        encounter.team = crate::engine::team::TeamState::default();
        encounter.measure_mode = crate::engine::encounter::MeasureMode::Normal;
    });
}

pub fn lookup_name_cache(uid: f64) -> Option<CachedPlayerDto> {
    let cached = name_cache::lookup(uid as i64)?;
    Some(CachedPlayerDto {
        name: cached.name,
        class_id: cached.class_id,
        ability_score: cached.ability_score,
    })
}

// ─── 3min measure mode ───────────────────────────────────────────────────────

/// 3分計測の確定。スナップショットを履歴へ push し集計をリセットして返す。
/// UI 通知（モーダル表示）は呼び出し側の責務。core は Tauri/emit に依存しない。
pub fn finalize_3min_locked(encounter: &mut Encounter) -> EncounterSnapshot {
    let now = crate::engine::processor::now_ms();
    let snapshot = build_encounter_snapshot(encounter, now);
    if !snapshot.player_rows.is_empty() {
        crate::engine::history::push(snapshot.clone());
    }
    // M2/M3/M5計測: 3分計測の確定時にもサマリーを出す（3分計測モードは processor.rs の
    // ロールオーバー分岐が MeasureMode::Normal 限定で発火しないため、ここが唯一の出口）。
    crate::probe::log_and_reset_encounter_summary();
    encounter.clear_combat_stats();
    encounter.measure_mode = crate::engine::encounter::MeasureMode::Normal;
    snapshot
}

/// 3分計測の確定直前に、全系列（global/entity/skill）へ終端サンプルを1点足し、
/// 末尾を実測の最終着弾時刻（`time_last_combat_packet_ms`）へ揃える。
/// 確定値のX軸最大は固定窓（`duration_ms`。`combat_elapsed_ms` 参照）のため、計測窓の
/// 途中で攻撃が止まった（早期終了）場合はこのサンプルを足しても折れ線は窓の右端までは
/// 届かない（＝X軸を固定窓にした副作用。以前の「折れ線を右端まで届かせる」という記述は
/// 分母が実測スパンだった頃のもので、現在は最終着弾時刻までしか保証しない）。
/// スキル内訳は finalize 前に `capture_3min_result_skills` で取得されるため、取得・確定の
/// **前** に呼ぶ。measure_mode が Active3Min のうちに採取すること（clear_combat_stats 前）。
pub fn seal_3min_series(enc: &EncounterMutex) {
    with_lock_or(enc, "seal_3min_series", (), |e| {
        let end_ts = e.time_last_combat_packet_ms;
        crate::engine::processor::take_time_series_sample(e, end_ts, true);
    });
}

/// 3分計測の確定直前に、結果モーダル表示用のスキル内訳（時系列込み）を全プレイヤーぶん
/// 取得する。`finalize_3min_locked`（内部で `build_encounter_snapshot` → `combat_elapsed_ms`）
/// と**同じ分母**を使うことで、結果モーダルのスキル別DPSがヘッダ/プレイヤー行と食い違わない
/// ようにする（ライブの `get_skills` を別途呼ぶと実測スパン基準になり、3分計測の固定窓と
/// ズレていた）。`build_encounter_snapshot` の `player_skill_rows` は history 用に時系列を
/// 持たせない設計のため、時系列が要る結果モーダル向けにこちらを別経路として用意する。
/// スキル0件のプレイヤー（ダメージを与えていない）は結果から落とす（`filter_map`）。
/// 呼び出し側（main.rs）が `contains_key` だけで「自分のスキル内訳がある」既定選択を
/// 決めているため、空 Vec を残すとダメージ0のプレイヤーでも真になってしまう。
/// finalize（clear_combat_stats）より前、measure_mode が Active3Min のうちに呼ぶこと。
pub fn capture_3min_result_skills(
    enc: &EncounterMutex,
) -> std::collections::HashMap<i64, Vec<SkillRow>> {
    with_lock_or(
        enc,
        "capture_3min_result_skills",
        std::collections::HashMap::new(),
        |encounter| {
            // build_players_window_unsorted と同じゲート（他クライアント特定前は空を返す）。
            // ここは全プレイヤーを列挙する経路なので、特定uid向けの get_skills 系とは異なり
            // 明示チェックが要る（漏れると未確定クライアントのuidがキーとして紛れ込む）。
            let selected = selected_uid::get();
            if selected.is_some() && !encounter.has_selected_participant {
                return std::collections::HashMap::new();
            }
            let now = crate::engine::processor::now_ms();
            let elapsed_secs = combat_elapsed_ms(encounter, now) as f64 / 1000.0;
            // 結果モーダルのスキル内訳も、プレイヤー行と同じ母集団に揃える。
            let only_uid = self_only_uid(encounter);
            encounter
                .entities
                .iter()
                .filter(|(key, _)| key.is_player())
                .filter(|(key, _)| visible_under(only_uid, key.player_uid()))
                .filter_map(|(&key, player)| {
                    let rows = build_skill_rows_for_player(player, elapsed_secs, false, true);
                    if rows.is_empty() {
                        None
                    } else {
                        Some((key.player_uid(), rows))
                    }
                })
                .collect()
        },
    )
}

/// 3分計測を確定し snapshot を返す（履歴 push・mode=Normal は finalize_3min_locked 内）。
/// 旧 Tauri 版はイベント発火だったが、Slint 版はポーリングで残0を検知して本関数を呼ぶ。
pub fn finalize_3min_measure_mode(enc: &EncounterMutex) -> Option<EncounterSnapshot> {
    with_lock_or(enc, "finalize_3min_measure_mode", None, |enc| {
        Some(finalize_3min_locked(enc))
    })
}

/// 計測ボタンで3分計測を開始する。`scope` は開始時の設定値のスナップショットで、以降は
/// `MeasureMode` が運ぶ（走行中に設定を変えても計測結果がぶれない）。
pub fn start_3min_measure_mode(
    enc: &EncounterMutex,
    duration_secs: f64,
    scope: crate::engine::encounter::MeasureScope,
) {
    let duration_ms = (duration_secs * 1000.0).max(1000.0) as u128;
    with_lock_or(enc, "start_3min_measure_mode", (), |enc| {
        // 計測を始める前の戦闘ぶんをサマリーとして切り出す（reset_encounter と同じ扱い）。
        // clear_combat_stats は集計を消すが probe のカウンタは別管理で残るため、ここを飛ばすと
        // 直前の通常戦闘が計測ぶんのサマリーへ持ち越され、対象別内訳やロック対象が混ざって読める。
        crate::probe::log_and_reset_encounter_summary();
        enc.clear_combat_stats();
        enc.measure_mode =
            crate::engine::encounter::MeasureMode::Pending3Min { duration_ms, scope };
        info!("3min measure mode: pending (duration={duration_ms}ms, scope={scope:?})");
    });
}

pub fn cancel_3min_measure_mode(enc: &EncounterMutex) {
    with_lock_or(enc, "cancel_3min_measure_mode", (), |enc| {
        // 中止した計測ぶんも他のエンカウンターと同じ粒度で切り出す（開始側と対）。
        crate::probe::log_and_reset_encounter_summary();
        enc.clear_combat_stats();
        enc.measure_mode = crate::engine::encounter::MeasureMode::Normal;
        info!("3min measure mode: cancelled");
    });
}

fn aggregate_player_buffs(
    snapshots: Vec<crate::engine::buff_tracker::BuffStateSnapshot>,
    uid: f64,
    name: String,
) -> PlayerBuffSnapshot {
    use crate::engine::buff_source::classify_buff;
    use std::collections::HashMap;

    let mut by_kind: HashMap<String, SelfBuffSnapshot> = HashMap::new();
    for snap in &snapshots {
        let kind = classify_buff(snap.base_id as i64);
        if kind == BuffSourceKind::Other {
            continue;
        }
        let kind_str = kind.as_str().to_string();
        let candidate = SelfBuffSnapshot {
            kind: kind_str.clone(),
            base_id: snap.base_id,
            buff_uuid: snap.buff_uuid,
            layer: snap.layer,
            remaining_ms: snap.remaining_ms,
            duration_ms: snap.duration_ms,
            received_at_ms: snap.received_at_local_ms as f64,
        };
        match by_kind.get_mut(&kind_str) {
            None => {
                by_kind.insert(kind_str, candidate);
            }
            Some(entry) => {
                if snap.remaining_ms > entry.remaining_ms {
                    *entry = candidate;
                }
            }
        }
    }

    PlayerBuffSnapshot {
        uid,
        name,
        buffs: by_kind.into_values().collect(),
    }
}

pub fn get_tracked_buffs(
    enc: &EncounterMutex,
    uids: Vec<f64>,
) -> TrackedBuffsData {
    use crate::engine::processor::now_ms;

    // ロック内: gc と snapshot のみ実施
    let (raw_snapshots, now_ms, local_uid) = {
        let mut enc = match enc.lock() {
            Ok(e) => e,
            Err(e) => {
                log::error!("Lock poisoned in get_tracked_buffs: {e}");
                return TrackedBuffsData::default();
            }
        };
        let now_ms = now_ms();
        let local_uid = enc.local_player_uid;
        enc.buff_tracker.gc(now_ms);
        let raw: Vec<(f64, i64, _)> = uids
            .iter()
            .map(|&uid_f64| {
                let uid_i64 = uid_f64 as i64;
                let snapshots = enc.buff_tracker.snapshot_for(uid_i64, now_ms);
                (uid_f64, uid_i64, snapshots)
            })
            .collect();
        (raw, now_ms, local_uid)
    }; // ロック解放

    // ロック外: name_cache 参照・kind 分類・HashMap 構築
    let players = raw_snapshots
        .into_iter()
        .map(|(uid_f64, uid_i64, snapshots)| {
            let name = name_cache::lookup(uid_i64)
                .map(|c| c.name)
                .unwrap_or_default();
            aggregate_player_buffs(snapshots, uid_f64, name)
        })
        .collect();

    TrackedBuffsData {
        players,
        now_ms: now_ms as f64,
        local_player_uid: local_uid as f64,
    }
}

pub fn get_measure_mode_status(enc: &EncounterMutex) -> MeasureModeStatus {
    use crate::engine::encounter::MeasureMode;
    use crate::engine::processor::now_ms;

    match enc.lock() {
        Ok(enc) => match enc.measure_mode {
            MeasureMode::Normal => MeasureModeStatus {
                kind: "normal".to_string(),
                remaining_ms: None,
                duration_ms: None,
                armed_at_ms: None,
            },
            MeasureMode::Pending3Min { duration_ms, .. } => MeasureModeStatus {
                kind: "pending".to_string(),
                remaining_ms: None,
                duration_ms: Some(duration_ms as f64),
                armed_at_ms: None,
            },
            MeasureMode::Active3Min {
                armed_at_ms,
                duration_ms,
                ..
            } => {
                let elapsed = now_ms().saturating_sub(armed_at_ms);
                let remaining = duration_ms.saturating_sub(elapsed) as f64;
                MeasureModeStatus {
                    kind: "active".to_string(),
                    remaining_ms: Some(remaining),
                    duration_ms: Some(duration_ms as f64),
                    armed_at_ms: Some(armed_at_ms as f64),
                }
            }
        },
        Err(e) => {
            log::error!("Lock poisoned in get_measure_mode_status: {e}");
            MeasureModeStatus {
                kind: "normal".to_string(),
                remaining_ms: None,
                duration_ms: None,
                armed_at_ms: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::buff_tracker::BuffStateSnapshot;

    fn snap(base_id: i32, remaining_ms: i64) -> BuffStateSnapshot {
        BuffStateSnapshot {
            buff_uuid: base_id,
            base_id,
            fire_uuid: 0,
            received_at_local_ms: 0,
            duration_ms: remaining_ms,
            remaining_ms,
            layer: 1,
            count: 1,
            create_time_server: 0,
            expire_at_local_ms: None,
            server_clock_trusted: true,
        }
    }

    // リキャスト ID (392101 等) が混入していても無視され、免疫デバフのみが残る。
    #[test]
    fn recast_id_is_ignored_even_when_mixed_with_debuff() {
        for snaps in [
            vec![snap(392101, 150_000), snap(2110056, 60_000)],
            vec![snap(2110056, 60_000), snap(392101, 150_000)],
        ] {
            let result = aggregate_player_buffs(snaps, 1.0, "self".into());
            assert_eq!(result.buffs.len(), 1);
            let b = &result.buffs[0];
            assert_eq!(b.kind, "Tina");
            assert_eq!(b.base_id, 2110056);
            assert_eq!(b.remaining_ms, 60_000);
        }
    }

    // 免疫デバフが届いていない場合はリキャスト ID も無視して何も表示しない。
    #[test]
    fn recast_id_alone_shows_nothing() {
        let result = aggregate_player_buffs(vec![snap(392101, 150_000)], 1.0, "self".into());
        assert_eq!(result.buffs.len(), 0);
    }

    #[test]
    fn debuff_only_is_kept() {
        let result = aggregate_player_buffs(vec![snap(2110056, 45_000)], 1.0, "self".into());
        assert_eq!(result.buffs.len(), 1);
        assert_eq!(result.buffs[0].base_id, 2110056);
    }

    // 同じバフの再付与で buff_uuid が増えても、自バフ窓の行は base_id で1件に束ねられ、
    // 残り時間が最長のインスタンスが代表になる。
    #[test]
    fn self_status_dedupes_instances_of_same_base_id() {
        let mut a = snap(2208484, 5_000); // 表示対象の自バフ (Buff/Normal)
        a.buff_uuid = 11;
        let mut b = snap(2208484, 9_000);
        b.buff_uuid = 22;
        for snaps in [vec![a.clone(), b.clone()], vec![b, a]] {
            let (buffs, debuffs) = split_self_status(snaps);
            assert_eq!(buffs.len(), 1, "同一 base_id は1行に束ねる");
            assert!(debuffs.is_empty());
            assert_eq!(buffs[0].remaining_ms, 9_000);
            assert_eq!(buffs[0].instance_id, 22);
        }
    }

    // 残り時間が同値でも並びは base_id 昇順で確定する（毎tick の行入れ替わり防止）。
    #[test]
    fn self_status_order_is_deterministic_on_ties() {
        let (buffs, _) = split_self_status(vec![snap(2208651, 4_000), snap(2208484, 4_000)]);
        let ids: Vec<i32> = buffs.iter().map(|e| e.base_id).collect();
        assert_eq!(ids, vec![2208484, 2208651]);
    }

    // 同一系統(免疫デバフ同士)では従来どおり残時間が長い方を採用。
    #[test]
    fn longest_remaining_wins_within_same_source() {
        let result = aggregate_player_buffs(
            vec![snap(2110056, 30_000), snap(2110056, 50_000)],
            1.0,
            "self".into(),
        );
        assert_eq!(result.buffs.len(), 1);
        assert_eq!(result.buffs[0].remaining_ms, 50_000);
    }

    // bare varint(LEB128) 符号化（attr raw_data 形式）。
    fn enc_varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
        out
    }

    // オーナー(AttrTopSummonerId)＋召喚元スキル(AttrSkillId)を載せた召喚 spawn 相当の SceneDelta。
    fn summon_spawn_delta(owner_uid: i64, skill_id: i32) -> crate::protocol::pb::SceneDelta {
        use crate::protocol::constants::attr_type;
        use crate::protocol::pb;
        let owner_uuid = (owner_uid << 16) | 640; // Player 型コード
        let summon_uuid = (skill_id as i64) << 16 | 0x0100; // Unknown 型コード
        pb::SceneDelta {
            uuid: summon_uuid,
            attrs: Some(pb::EntityAttrs {
                uuid: summon_uuid,
                attrs: vec![
                    pb::RawAttr {
                        id: attr_type::ATTR_TOP_SUMMONER_ID,
                        raw_data: enc_varint(owner_uuid as u64),
                    },
                    pb::RawAttr { id: attr_type::ATTR_SKILL_ID, raw_data: enc_varint(skill_id as u64) },
                ],
            }),
            buff_list: None,
            skill_effects: None,
        }
    }

    // DPSランキング表示(get_dps_players)の imagine_suffix を実経路で検証する。pending 方式では
    // 3体目(新規)を検知しても即座には確定へ反映されない（confirmed は元の2件のまま＝新旧混在
    // ペアを一切表示しない）。確証（＝既存スロットの再検知）が得られて初めて pending の
    // ロローラが確定へ昇格し、実ゲーム版の召喚ID(2900840)で解決されたロローラが表示される。
    #[test]
    fn dps_ranking_imagine_suffix_confirmed_only_after_recheck_shows_rorora() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::entity::Entity;
        use crate::engine::processor::process_scene_delta;

        const SELF_UID: i64 = 42;
        let enc: EncounterMutex = std::sync::Mutex::new(Encounter::default());
        {
            let mut e = enc.lock().unwrap();
            let mut p = Entity::default();
            p.name = Some("ソラ".to_string());
            p.dmg_stats.total = 1000; // ランキングに載せるためダメージ実績を持たせる
            e.entities.insert(EntityKey::player(SELF_UID), p);

            // ヴェノミーンの巣 → アルーナ → ロローラ の順に召喚検知（3体・枠は2つ）。
            process_scene_delta(&mut e, summon_spawn_delta(SELF_UID, 1_007_740));
            process_scene_delta(&mut e, summon_spawn_delta(SELF_UID, 2_900_240));
            process_scene_delta(&mut e, summon_spawn_delta(SELF_UID, 2_900_840));
        }

        let window = get_dps_players(&enc);
        let row = window
            .player_rows
            .iter()
            .find(|r| r.uid as i64 == SELF_UID)
            .expect("SELF row present in DPS ranking");
        // 3体目(ロローラ)はまだ確証が無いので pending 止まり。表示は元の2件のまま。
        assert_eq!(row.imagine_suffix, "-ヴェノミーンの巣/アルーナ");

        {
            let mut e = enc.lock().unwrap();
            // ヴェノミーンの巣(現役)を再検知 → 確証が得られ、pending のロローラが確定へ昇格。
            process_scene_delta(&mut e, summon_spawn_delta(SELF_UID, 1_007_740));
        }

        let window = get_dps_players(&enc);
        let row = window
            .player_rows
            .iter()
            .find(|r| r.uid as i64 == SELF_UID)
            .expect("SELF row present in DPS ranking");
        // 昇格後は最新2件（ヴェノミーンの巣/ロローラ）に丸められ、ロローラが実ゲーム版の
        // 召喚ID(2900840)で解決されて表示される＝「3つ以上出さない」「ロローラが表示される」を満たす。
        assert_eq!(row.imagine_suffix, "-ヴェノミーンの巣/ロローラ");
    }

    // DPSランキング表示(get_dps_players)で、実イマジン2枠とは別枠のロールスキル(簡易版
    // バトルイマジン)が role_skill_suffix へ " (R:名前)" 形式で分離されること。
    // 別フィールドにすることで、名前列テンプレートが位置・有無を独立に指定できる。
    #[test]
    fn dps_ranking_role_skill_suffix_is_separate_from_imagine_suffix() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::entity::{Entity, ImagineSlot};

        const SELF_UID: i64 = 43;
        let enc: EncounterMutex = std::sync::Mutex::new(Encounter::default());
        {
            let mut e = enc.lock().unwrap();
            let mut p = Entity::default();
            p.name = Some("ソラ".to_string());
            p.dmg_stats.total = 1000; // ランキングに載せるためダメージ実績を持たせる
            p.imagines = vec![
                ImagineSlot { name: "ゴーストカニクモ".to_string(), last_seen: 0, tier: 0, pending_hits: 0 },
                ImagineSlot { name: "ティナ".to_string(), last_seen: 1, tier: 0, pending_hits: 0 },
            ];
            p.role_skill_imagines = vec![ImagineSlot {
                name: "アルーナ".to_string(),
                last_seen: 2,
                tier: 3,
                pending_hits: 0,
            }];
            e.entities.insert(EntityKey::player(SELF_UID), p);
        }

        let window = get_dps_players(&enc);
        let row = window
            .player_rows
            .iter()
            .find(|r| r.uid as i64 == SELF_UID)
            .expect("SELF row present in DPS ranking");
        assert_eq!(row.imagine_suffix, "-ゴーストカニクモ/ティナ");
        assert_eq!(row.role_skill_suffix, " (R:アルーナ(3))");
    }

    // ロールスキルは最大4枠(SlotPositionId 21-24)を同時装備できる。DPSランキング表示の
    // role_skill_suffix が3〜4件を欠落なく "/" 区切りで結合表示することを確認する
    // （ユーザー指摘の「1件目以降が黙って消える」ケースの直接の回帰テスト）。
    #[test]
    fn dps_ranking_role_skill_suffix_shows_all_simultaneous_role_skill_labels() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::entity::{Entity, ImagineSlot};

        const SELF_UID: i64 = 44;
        let enc: EncounterMutex = std::sync::Mutex::new(Encounter::default());
        {
            let mut e = enc.lock().unwrap();
            let mut p = Entity::default();
            p.name = Some("ソラ".to_string());
            p.dmg_stats.total = 1000; // ランキングに載せるためダメージ実績を持たせる
            p.imagines = vec![
                ImagineSlot { name: "ゴーストカニクモ".to_string(), last_seen: 0, tier: 0, pending_hits: 0 },
                ImagineSlot { name: "ティナ".to_string(), last_seen: 1, tier: 0, pending_hits: 0 },
            ];
            p.role_skill_imagines = vec![
                ImagineSlot { name: "アルーナ".to_string(), last_seen: 2, tier: 3, pending_hits: 0 },
                ImagineSlot { name: "ファルファラ".to_string(), last_seen: 3, tier: 0, pending_hits: 0 },
                ImagineSlot { name: "鉄牙".to_string(), last_seen: 4, tier: 1, pending_hits: 0 },
                ImagineSlot { name: "キングムーク".to_string(), last_seen: 5, tier: 0, pending_hits: 0 },
            ];
            e.entities.insert(EntityKey::player(SELF_UID), p);
        }

        let window = get_dps_players(&enc);
        let row = window
            .player_rows
            .iter()
            .find(|r| r.uid as i64 == SELF_UID)
            .expect("SELF row present in DPS ranking");
        assert_eq!(row.imagine_suffix, "-ゴーストカニクモ/ティナ");
        assert_eq!(
            row.role_skill_suffix,
            " (R:アルーナ(3)/ファルファラ/鉄牙(1)/キングムーク)",
            "all 4 simultaneous role skill labels must be shown, none silently dropped"
        );
    }

    // build_players_window_unsorted が stat_type と一致する時系列を PlayerRow.time_series に
    // 載せることを確認する（回復/被ダメタブの推移グラフ・固定基準バーが与ダメ系列を誤って
    // 描いていた問題の回帰テスト）。
    #[test]
    fn player_row_time_series_matches_requested_stat_type() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::entity::Entity;

        const UID: i64 = 77;
        let enc: EncounterMutex = std::sync::Mutex::new(Encounter::default());
        {
            let mut e = enc.lock().unwrap();
            let mut p = Entity::default();
            p.dmg_stats.total = 100;
            p.heal_stats.total = 200;
            p.dmg_taken_stats.total = 300;
            p.time_series =
                VecDeque::from(vec![TimeSeriesPoint { t_ms: 0.0, total_dmg: 100.0, total_dps: 10.0 }]);
            p.heal_time_series =
                VecDeque::from(vec![TimeSeriesPoint { t_ms: 0.0, total_dmg: 200.0, total_dps: 20.0 }]);
            p.dmg_taken_time_series =
                VecDeque::from(vec![TimeSeriesPoint { t_ms: 0.0, total_dmg: 300.0, total_dps: 30.0 }]);
            e.entities.insert(EntityKey::player(UID), p);
        }

        let dmg_row = get_dps_players(&enc)
            .player_rows
            .into_iter()
            .find(|r| r.uid as i64 == UID)
            .expect("dmg row present");
        assert_eq!(dmg_row.time_series.len(), 1);
        assert_eq!(dmg_row.time_series[0].total_dps, 10.0, "dps tab must surface the dmg series");

        let heal_row = get_heal_players(&enc)
            .player_rows
            .into_iter()
            .find(|r| r.uid as i64 == UID)
            .expect("heal row present");
        assert_eq!(heal_row.time_series.len(), 1);
        assert_eq!(
            heal_row.time_series[0].total_dps, 20.0,
            "heal tab must surface the heal series, not the dmg series"
        );

        let taken_row = get_dmg_taken_players(&enc)
            .player_rows
            .into_iter()
            .find(|r| r.uid as i64 == UID)
            .expect("taken row present");
        assert_eq!(taken_row.time_series.len(), 1);
        assert_eq!(
            taken_row.time_series[0].total_dps, 30.0,
            "taken tab must surface the taken series, not the dmg series"
        );
    }

    // get_dps_players の value_pct（シェア%）は build_players_window_unsorted の
    // ratio_pct(entity_stats.total, encounter_stats.total) を経由して実際に算出させる
    // （旧テストは compute 側を一切呼ばず enc.dmg_stats.total と player_dmg を手動で割るだけの
    // 恒真式だった＝processor.rs 側で分母(encounter.dmg_stats)からモンスターの反撃分を除外する
    // 判定が崩れても検出できなかった）。分母からモンスターの反撃分が漏れていれば
    // value_pct が 100% を割り込む（結果モーダルの行バーが縮む/伸びる不具合として現れる）。
    #[test]
    fn get_dps_players_value_pct_reflects_encounter_denominator() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::processor::process_scene_delta;
        use crate::protocol::pb;

        fn damage_delta(target_uuid: i64, attacker_uuid: i64, value: i64) -> pb::SceneDelta {
            pb::SceneDelta {
                uuid: target_uuid,
                skill_effects: Some(pb::SkillImpact {
                    damages: vec![pb::DamageRecord {
                        value,
                        hp_lessen_value: value,
                        attacker_uuid,
                        owner_id: 1001,
                        ..Default::default()
                    }],
                }),
                ..Default::default()
            }
        }

        let player_uuid = (30_i64 << 16) | 640;
        let monster_uuid = (31_i64 << 16) | 64;

        let enc: EncounterMutex = std::sync::Mutex::new(Encounter::default());
        {
            let mut e = enc.lock().unwrap();
            // プレイヤーがモンスターへ1000ダメージ（自分の火力）
            process_scene_delta(&mut e, damage_delta(monster_uuid, player_uuid, 1_000));
            // モンスターの反撃で自分が500ダメージ（分母(encounter.dmg_stats)に混ざってはいけない）
            process_scene_delta(&mut e, damage_delta(player_uuid, monster_uuid, 500));
        }

        let window = get_dps_players(&enc);
        assert_eq!(window.player_rows.len(), 1, "反撃を受けたプレイヤーの行のみ出るはず");
        assert_eq!(
            window.player_rows[0].value_pct, 100.0,
            "モンスターの反撃分が分母に混入し、行のシェア%が100%からずれている"
        );
    }

    // get_skills はタブ(0=dps/1=heal)に応じて集計元(dmg_stats/heal_stats・両スキルmap)を
    // 切り替える。回復タブでも常に与ダメ基準になっていた既存バグの回帰テスト。
    #[test]
    fn get_skills_selects_stat_source_by_tab() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::entity::Entity;

        const UID: i64 = 88;
        let enc: EncounterMutex = std::sync::Mutex::new(Encounter::default());
        {
            let mut e = enc.lock().unwrap();
            let mut p = Entity::default();
            p.dmg_stats.total = 1000;
            p.heal_stats.total = 2000;
            p.skill_uid_to_dps_stats.insert(1, CombatStats { total: 1000, ..Default::default() });
            p.skill_uid_to_heal_stats.insert(2, CombatStats { total: 2000, ..Default::default() });
            e.entities.insert(EntityKey::player(UID), p);
        }

        let dps_sw = get_skills(&enc, UID, StatType::Dmg).expect("dps skills");
        assert_eq!(dps_sw.inspected_player.total_value, 1000.0);
        assert_eq!(dps_sw.skill_rows.len(), 1);
        assert_eq!(dps_sw.skill_rows[0].uid, 1.0);

        let heal_sw = get_skills(&enc, UID, StatType::Heal).expect("heal skills");
        assert_eq!(heal_sw.inspected_player.total_value, 2000.0);
        assert_eq!(heal_sw.skill_rows.len(), 1);
        assert_eq!(heal_sw.skill_rows[0].uid, 2.0, "heal tab must list heal skills, not dps skills");
    }

    #[test]
    fn history_skill_rows_are_sorted_and_omit_time_series() {
        use crate::engine::entity::Entity;

        let mut player = Entity::default();
        player.dmg_stats.total = 1000;
        player.skill_uid_to_dps_stats.insert(
            101,
            CombatStats {
                total: 300,
                hit_count: 3,
                ..Default::default()
            },
        );
        player.skill_uid_to_dps_stats.insert(
            102,
            CombatStats {
                total: 700,
                hit_count: 7,
                ..Default::default()
            },
        );
        player.skill_time_series.insert(
            102,
            VecDeque::from(vec![TimeSeriesPoint {
                t_ms: 100.0,
                total_dmg: 700.0,
                total_dps: 700.0,
            }]),
        );

        let rows = build_skill_rows_for_player(&player, 10.0, false, false);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].uid, 102.0, "history skill rows stay damage-descending");
        assert_eq!(rows[0].value_pct, 70.0);
        assert!(rows[0].time_series.is_empty(), "history stores the breakdown, not graph samples");

        let live_rows = build_skill_rows_for_player(&player, 10.0, false, true);
        assert_eq!(live_rows[0].time_series.len(), 1);
    }

    #[test]
    fn old_history_without_skill_rows_deserializes_as_empty() {
        let snapshot: EncounterSnapshot =
            serde_json::from_str(r#"{"id":7,"playerRows":[]}"#).expect("old history schema");
        assert_eq!(snapshot.id, 7.0);
        assert!(snapshot.player_skill_rows.is_empty());
    }

    // ─── 自分のみ計測（MeasureScope::self_only）の回帰テスト ──────────────────────
    //
    // 実装は取り込み時のフィルタではなく読み出し時の射影。Encounter 側の集計は全員ぶん残るので、
    // 計測を抜ければ元の表示に戻る（可逆であることをテストで固定する）。

    /// self_only 用の Encounter。自分と他プレイヤーが1人ずつ、計測中(Active3Min)。
    fn enc_with_two_players(self_only: bool) -> Encounter {
        use crate::engine::entity::Entity;
        let me = 100_i64;
        let other = 200_i64;
        let mut enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 11_000,
            local_player_uid: me,
            measure_mode: crate::engine::encounter::MeasureMode::Active3Min {
                armed_at_ms: 1_000,
                duration_ms: 180_000,
                scope: crate::engine::encounter::MeasureScope {
                    first_target_only: false,
                    self_only,
                },
            },
            ..Default::default()
        };
        enc.dmg_stats.total = 1_000;
        let mut mine = Entity::default();
        mine.dmg_stats.total = 300;
        let mut theirs = Entity::default();
        theirs.dmg_stats.total = 700;
        enc.entities.insert(EntityKey::player(me), mine);
        enc.entities.insert(EntityKey::player(other), theirs);
        enc
    }

    /// 自分のみ計測が off のときは全員ぶんが母集団になる（既定動作の固定）。
    #[test]
    fn self_only_off_keeps_every_player_and_the_full_total() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let enc = enc_with_two_players(false);
        assert_eq!(total_stats_for(&enc, StatType::Dmg).total, 1_000);
        let window = build_players_window_unsorted(&enc, StatType::Dmg, false, 10.0);
        assert_eq!(window.player_rows.len(), 2);
    }

    /// 自分のみ計測が on なら、行は自分だけ・母集団も自分の集計へ差し替わる。
    /// シェア率の分子と分母が同じ値から出るので、自分のシェアは 100% になる。
    #[test]
    fn self_only_on_projects_rows_and_total_onto_the_local_player() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let enc = enc_with_two_players(true);
        assert_eq!(
            total_stats_for(&enc, StatType::Dmg).total,
            300,
            "母集団が自分の集計へ差し替わっていない"
        );
        let window = build_players_window_unsorted(&enc, StatType::Dmg, false, 10.0);
        assert_eq!(window.player_rows.len(), 1, "他プレイヤーの行が残っている");
        assert_eq!(window.player_rows[0].uid as i64, 100);
    }

    /// 射影は読み出し時だけの操作で、Encounter 側の集計は全員ぶん残る。
    /// 計測を抜ければ（Normal へ戻れば）元の表示に戻る＝不可逆な情報破壊をしていない。
    #[test]
    fn self_only_is_reversible_because_the_underlying_stats_are_untouched() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let mut enc = enc_with_two_players(true);
        assert_eq!(build_players_window_unsorted(&enc, StatType::Dmg, false, 10.0).player_rows.len(), 1);

        enc.measure_mode = crate::engine::encounter::MeasureMode::Normal;

        assert_eq!(total_stats_for(&enc, StatType::Dmg).total, 1_000);
        assert_eq!(
            build_players_window_unsorted(&enc, StatType::Dmg, false, 10.0).player_rows.len(),
            2,
            "計測を抜けても他プレイヤーが戻らない＝取り込み時に捨ててしまっている"
        );
    }

    /// 一覧から行が消えても、消える前に開いていた他プレイヤーの内訳ドリルは呼び出し側に残る。
    /// 自分のみ計測中はそれを断る（Err を返して呼び出し側にドリルを畳ませる）。
    #[test]
    fn self_only_rejects_breakdowns_for_other_players() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let enc: EncounterMutex = std::sync::Mutex::new(enc_with_two_players(true));

        assert!(
            get_skills(&enc, 200, StatType::Dmg).is_err(),
            "自分のみ計測中に他プレイヤーのスキル内訳が返っている"
        );
        assert!(
            get_dmg_taken_attackers(&enc, 200).is_err(),
            "自分のみ計測中に他プレイヤーの被ダメ内訳が返っている"
        );
        assert!(
            get_skills(&enc, 100, StatType::Dmg).is_ok(),
            "自分の内訳まで断っている"
        );
    }

    /// 自分のみ計測が off なら、他プレイヤーの内訳は従来どおり見られる。
    #[test]
    fn self_only_off_keeps_breakdowns_for_other_players() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let enc: EncounterMutex = std::sync::Mutex::new(enc_with_two_players(false));

        assert!(get_skills(&enc, 200, StatType::Dmg).is_ok());
        assert!(get_dmg_taken_attackers(&enc, 200).is_ok());
    }

    /// スナップショットは、その計測に効いていた条件を運ぶ。透かし・履歴・自己ベストのキーは
    /// すべてここから導くため、finalize より前に別途採る必要が無い。
    #[test]
    fn snapshot_carries_the_measure_scope_it_was_taken_under() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let enc = enc_with_two_players(true);

        let snap = build_encounter_snapshot(&enc, 11_000);

        assert!(snap.measure_scope.self_only, "条件がスナップショットに残っていない");
        assert!(!snap.measure_scope.first_target_only);
    }

    /// 通常モードの計測は条件なしで記録される（既定動作の固定）。
    #[test]
    fn snapshot_from_normal_mode_has_no_measure_scope() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let mut enc = enc_with_two_players(false);
        enc.measure_mode = crate::engine::encounter::MeasureMode::Normal;

        let snap = build_encounter_snapshot(&enc, 11_000);

        assert_eq!(
            snap.measure_scope,
            crate::engine::encounter::MeasureScope::default()
        );
    }

    /// 条件フィールドを持たない旧 history.json も、絞り込み無しとして読める。
    #[test]
    fn legacy_snapshot_json_without_scope_still_parses() {
        let snap: EncounterSnapshot = serde_json::from_str(
            r#"{"id":1.0,"startMs":0.0,"endMs":1000.0,"durationMs":1000.0,"totalDmg":5.0,"totalDps":5.0}"#,
        )
        .expect("旧 history.json がパースできなくなっている");

        assert_eq!(snap.total_dmg, 5.0);
        assert_eq!(
            snap.measure_scope,
            crate::engine::encounter::MeasureScope::default()
        );
        assert_eq!(snap.level_map_id, 0, "旧 history.json は level_map_id 不明として 0 で読める");
    }

    /// 与ダメージ0でも自分の行は残る。回復専業や被ダメージ計測では dmg_stats が0のまま
    /// 確定しうるが、落とすと player_rows が空になり履歴 push も自己ベスト判定も飛ぶ。
    #[test]
    fn self_only_keeps_my_row_even_with_zero_damage() {
        use crate::engine::entity::Entity;
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let me = 100_i64;

        let mut enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 11_000,
            local_player_uid: me,
            measure_mode: crate::engine::encounter::MeasureMode::Active3Min {
                armed_at_ms: 1_000,
                duration_ms: 180_000,
                scope: crate::engine::encounter::MeasureScope { first_target_only: false, self_only: true },
            },
            ..Default::default()
        };
        let mut mine = Entity::default();
        mine.heal_stats.total = 5_000; // 回復だけ出した
        enc.entities.insert(EntityKey::player(me), mine);

        let window = build_players_window_unsorted(&enc, StatType::Dmg, false, 10.0);
        assert_eq!(window.player_rows.len(), 1, "与ダメ0の自分の行が落ちている");

        let snap = build_encounter_snapshot(&enc, 11_000);
        assert!(!snap.player_rows.is_empty(), "スナップショットが空＝履歴も自己ベストも飛ぶ");
    }

    /// 自分の Entity がまだ無いあいだ、母集団は0を返す（行は0件なのにヘッダだけ全員の合計、
    /// という食い違いを作らない）。
    #[test]
    fn self_only_total_is_zero_while_my_entity_is_missing() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let mut enc = enc_with_two_players(true);
        enc.entities.remove(&EntityKey::player(100));

        assert_eq!(total_stats_for(&enc, StatType::Dmg).total, 0);
        assert_eq!(
            build_players_window_unsorted(&enc, StatType::Dmg, false, 10.0).player_rows.len(),
            0
        );
    }

    /// オーバーレイ名簿は自分のみ計測の射影を通さない（PT のイマジンタイマーが消えない）。
    #[test]
    fn roster_uids_ignore_the_self_only_projection() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let enc: EncounterMutex = std::sync::Mutex::new(enc_with_two_players(true));

        let roster = get_roster_uids(&enc, StatType::Dmg);

        assert_eq!(roster.len(), 2, "自分のみ計測で名簿が自分だけに縮んでいる");
        assert_eq!(roster[0] as i64, 200, "合計の降順になっていない");
    }

    /// 通常モードは scope を持てないため、絞り込みは常に無効。
    #[test]
    fn normal_mode_never_has_a_measure_scope() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let enc = Encounter::default();
        assert_eq!(enc.measure_scope(), crate::engine::encounter::MeasureScope::default());
        assert_eq!(self_only_uid(&enc), None);
    }

    // ─── 分母(elapsed)の単一化・3分計測固定窓の回帰テスト ──────────────────────────

    use crate::engine::encounter::{MeasureMode, MeasureScope};

    // 未戦闘（time_fight_start_ms==0）は now に関わらず常に0（0除算・パニックの回帰防止）。
    #[test]
    fn combat_elapsed_ms_is_zero_when_never_fought() {
        let enc = Encounter::default();
        assert_eq!(combat_elapsed_ms(&enc, 999_999), 0);
    }

    // 通常モード(Normal)は「戦闘中は現在時刻を分母にする」設計を撤回し、ライブ・確定を問わず
    // 常に実測スパン(time_last_combat_packet_ms - time_fight_start_ms)固定（now が
    // どれだけ進んでも伸びない）。これにより、設定「戦闘終了(秒)=0」での分母無限伸長・
    // 一時停止中の分母進行・時計巻き戻りでの分母割れ、のいずれも構造的に起きない。
    #[test]
    fn combat_elapsed_ms_normal_mode_always_uses_real_span_regardless_of_now() {
        let enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 4_000, // 実測スパンは3秒
            measure_mode: MeasureMode::Normal,
            ..Default::default()
        };
        // 最終着弾から間もない now（一般的には「戦闘中」とみなされ得るタイミング）でも、
        // 最終着弾から大きく経過した now でも、結果は変わらず実測スパン固定。
        assert_eq!(combat_elapsed_ms(&enc, 4_000 + 500), 3_000);
        assert_eq!(combat_elapsed_ms(&enc, 4_000 + 9_000), 3_000);
        // 時計巻き戻り（NTP補正等）で now < time_last_combat_packet_ms になっても
        // 実測スパンをそのまま返す（now を分母計算に使わないため巻き戻りの影響を受けない）。
        assert_eq!(combat_elapsed_ms(&enc, 500), 3_000);
    }

    // 3分計測(Active3Min)のライブ分母は armed_at_ms からの実経過(=now基準)を duration_ms で
    // 頭打ちにする。armed_at_ms は開始時に time_fight_start_ms と同時刻にセットされるため、
    // ライブ表示は実時間で伸び、窓を過ぎたら確定と同じ値(duration_ms)に収束する
    // （3:00到達で確定した瞬間に値が下へ跳ぶ不連続がない。H-1）。
    #[test]
    fn combat_elapsed_ms_3min_live_tracks_now_capped_at_duration() {
        let enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 10_000, // 直近まで攻撃していた（実測10秒）
            measure_mode: MeasureMode::Active3Min { armed_at_ms: 1_000, duration_ms: 180_000, scope: MeasureScope::default() },
            ..Default::default()
        };
        // 計測窓の途中（経過60秒）は armed_at からの実経過をそのまま返す。
        assert_eq!(combat_elapsed_ms(&enc, 1_000 + 60_000), 60_000);
        // 手を止めて放置しても、窓が続く限り now に追従し続ける（一時停止中も分母が動く
        // 旧問題とは別物＝3分計測はそもそも wall-clock ベースの固定窓という仕様のため）。
        assert_eq!(combat_elapsed_ms(&enc, 1_000 + 120_000), 120_000);
        // 窓の終端を過ぎたら duration_ms で頭打ち（確定時と同じ値に収束）。
        assert_eq!(combat_elapsed_ms(&enc, 1_000 + 180_000), 180_000);
        assert_eq!(combat_elapsed_ms(&enc, 1_000 + 999_000), 180_000);
    }

    // 3分計測(Active3Min)の確定時は、実測スパン(この例では10秒で殴り終えている)ではなく
    // 設定された窓長(duration_ms)を分母に使う（早く殴り終えるほど数字が良くなる問題の回帰防止）。
    // ライブ分母と同じ combat_elapsed_ms を通しており、t=duration_ms の値と一致する
    // （ライブ→確定の不連続が構造的に無いことの確認）。
    #[test]
    fn combat_elapsed_ms_uses_fixed_window_regardless_of_real_span_at_finalize() {
        let enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 10_000, // 実測は10秒で殴り終えた
            measure_mode: MeasureMode::Active3Min { armed_at_ms: 1_000, duration_ms: 180_000, scope: MeasureScope::default() },
            ..Default::default()
        };
        // now を計測終了直後にしても、実測スパンでなく設定窓長(180秒)を使う。
        let now = 1_000 + 180_000;
        assert_eq!(combat_elapsed_ms(&enc, now), 180_000);
    }

    // 通常モード(Normal)の確定は従来どおり実測スパン（3分計測の固定窓は Active3Min限定
    // であることの回帰テスト）。
    #[test]
    fn combat_elapsed_ms_normal_mode_uses_real_span_at_finalize() {
        let enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 10_000,
            measure_mode: MeasureMode::Normal,
            ..Default::default()
        };
        assert_eq!(combat_elapsed_ms(&enc, 1_000 + 10_000), 10_000);
    }

    // build_encounter_snapshot: 3分計測確定時、ヘッダ(total_dps)・プレイヤー行・スキル内訳が
    // すべて同じ分母(固定窓)から導出され、互いに食い違わない（不整合の回帰防止）。
    // 実測スパンは10秒だが窓長は180秒 → 実測基準なら1,800,000/10=180,000 dpsになってしまうところ、
    // 固定窓基準なら 1,800,000/180=10,000 dps になるはず。
    #[test]
    fn build_encounter_snapshot_3min_header_row_and_skill_share_same_denominator() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::combat_stats::CombatStats;
        use crate::engine::entity::Entity;

        const UID: i64 = 501;
        let mut enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 10_000,
            measure_mode: MeasureMode::Active3Min { armed_at_ms: 1_000, duration_ms: 180_000, scope: MeasureScope::default() },
            ..Default::default()
        };
        enc.dmg_stats.total = 1_800_000;
        let mut p = Entity::default();
        p.dmg_stats.total = 1_800_000;
        p.skill_uid_to_dps_stats.insert(9, CombatStats { total: 1_800_000, ..Default::default() });
        enc.entities.insert(EntityKey::player(UID), p);

        let now = 1_000 + 180_000;
        let snap = build_encounter_snapshot(&enc, now);

        assert_eq!(snap.duration_ms, 180_000.0, "duration_msは実測スパンでなく設定窓長");
        assert_eq!(snap.total_dps, 10_000.0, "ヘッダDPSは固定窓(180秒)基準");

        let row = snap.player_rows.iter().find(|r| r.uid as i64 == UID).expect("player row");
        assert_eq!(row.value_per_sec, 10_000.0, "プレイヤー行DPSはヘッダと同じ分母");

        let skill_snap = snap
            .player_skill_rows
            .iter()
            .find(|s| s.player_uid as i64 == UID)
            .expect("player skill rows");
        assert_eq!(
            skill_snap.skill_rows[0].value_per_sec, 10_000.0,
            "スキル内訳DPSもヘッダ/プレイヤー行と同じ分母"
        );
    }

    // 3分計測の窓の途中で一度も殴らなかった（ダメージ0）場合でも、固定窓の分母は正の値
    // (duration_ms>=1000、start_3min_measure_mode側の保証)のため0除算・パニックは起きない。
    #[test]
    fn build_encounter_snapshot_3min_zero_damage_no_panic_or_div_by_zero() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000,
            measure_mode: MeasureMode::Active3Min { armed_at_ms: 1_000, duration_ms: 180_000, scope: MeasureScope::default() },
            ..Default::default()
        };
        let snap = build_encounter_snapshot(&enc, 1_000 + 180_000);
        assert_eq!(snap.duration_ms, 180_000.0);
        assert_eq!(snap.total_dps, 0.0);
        assert!(snap.total_dps.is_finite());
    }

    // build_encounter_snapshot は fight_level_map_id（戦闘開始の瞬間のシーン）を保存する。
    // current_level_map_id（その後シーンが変わっているかもしれない現在値）は使わない。
    #[test]
    fn build_encounter_snapshot_uses_fight_level_map_id_not_current() {
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        let enc = Encounter {
            fight_level_map_id: 6545,
            current_level_map_id: 8,
            ..Default::default()
        };
        let snap = build_encounter_snapshot(&enc, 0);
        assert_eq!(snap.level_map_id, 6545);
    }

    // set_selected_uid で自キャラが実際に切り替わったときは current_level_map_id をクリアする
    // （前キャラのシーンを新キャラの最初の戦闘記録に引き継がないため。team と同じ理由）。
    #[test]
    fn set_selected_uid_clears_current_level_map_id_on_character_switch() {
        let _guard = selected_uid::lock_for_test();
        let enc: EncounterMutex = std::sync::Mutex::new(Encounter {
            local_player_uid: 100,
            current_level_map_id: 6545,
            ..Default::default()
        });

        set_selected_uid(&enc, Some(200.0));

        assert_eq!(enc.lock().unwrap().current_level_map_id, 0, "自キャラの切替でクリアする");
        selected_uid::set(None);
    }

    // 同じ uid を再指定しただけ（切替ではない）なら current_level_map_id は保持される。
    #[test]
    fn set_selected_uid_keeps_current_level_map_id_when_uid_unchanged() {
        let _guard = selected_uid::lock_for_test();
        let enc: EncounterMutex = std::sync::Mutex::new(Encounter {
            local_player_uid: 100,
            current_level_map_id: 6545,
            ..Default::default()
        });

        set_selected_uid(&enc, Some(100.0));

        assert_eq!(
            enc.lock().unwrap().current_level_map_id,
            6545,
            "同一 uid の再指定では値を失わない"
        );
        selected_uid::set(None);
    }

    // 選択解除(Some(A)→None)でもクリアする。後の自動検出 0→B では set_local_player_uid が
    // クリアしないため、ここで消さないと前キャラのシーンが B の最初の戦闘に残る。
    #[test]
    fn set_selected_uid_clears_current_level_map_id_on_deselect() {
        let _guard = selected_uid::lock_for_test();
        let enc: EncounterMutex = std::sync::Mutex::new(Encounter {
            local_player_uid: 100,
            current_level_map_id: 6545,
            ..Default::default()
        });

        set_selected_uid(&enc, None);

        assert_eq!(enc.lock().unwrap().current_level_map_id, 0, "選択解除でクリアする");
        selected_uid::set(None);
    }

    // capture_3min_result_skills は finalize（build_encounter_snapshot）と同じ分母
    // (combat_elapsed_ms＝Active3Minは固定窓)を使う（結果モーダルのスキル内訳DPSがヘッダ/
    // プレイヤー行と食い違わないことの回帰テスト。以前はライブの get_skills を別途呼んでいた
    // ため実測スパン基準になり、固定窓とズレていた）。
    #[test]
    fn capture_3min_result_skills_uses_same_denominator_as_finalized_snapshot() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::combat_stats::CombatStats;
        use crate::engine::entity::Entity;

        const UID: i64 = 502;
        let enc: EncounterMutex = std::sync::Mutex::new(Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 10_000, // 実測は10秒で殴り終えた
            measure_mode: MeasureMode::Active3Min { armed_at_ms: 1_000, duration_ms: 180_000, scope: MeasureScope::default() },
            ..Default::default()
        });
        {
            let mut e = enc.lock().unwrap();
            let mut p = Entity::default();
            p.dmg_stats.total = 1_800_000;
            p.skill_uid_to_dps_stats.insert(9, CombatStats { total: 1_800_000, ..Default::default() });
            e.entities.insert(EntityKey::player(UID), p);
            e.dmg_stats.total = 1_800_000;
        }

        let skills = capture_3min_result_skills(&enc);
        let rows = skills.get(&UID).expect("player skill rows present");
        assert_eq!(
            rows[0].value_per_sec, 10_000.0,
            "実測スパン(10秒)基準の180,000dpsではなく、固定窓(180秒)基準の10,000dpsになるべき"
        );
        assert_eq!(rows[0].time_series.len(), 0, "スキルの時系列サンプルは未採取なら空のまま");
    }

    // ダメージを一度も与えていないプレイヤー（スキル内訳が空）は結果から落とす（filter_map）。
    // main.rs 側の既定選択ロジックは `skills.contains_key(&local_uid)` だけを見るため、空 Vec を
    // 残すとダメージ0のプレイヤーでも「スキル内訳あり」と誤判定されてしまう（回帰防止）。
    #[test]
    fn capture_3min_result_skills_drops_players_with_no_skills() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::entity::Entity;

        const UID_NO_DMG: i64 = 503;
        let enc: EncounterMutex = std::sync::Mutex::new(Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 10_000,
            measure_mode: MeasureMode::Active3Min { armed_at_ms: 1_000, duration_ms: 180_000, scope: MeasureScope::default() },
            ..Default::default()
        });
        {
            let mut e = enc.lock().unwrap();
            // ダメージ実績もスキル内訳も無いプレイヤー（例: 見学のみで一度も攻撃していない）。
            e.entities.insert(EntityKey::player(UID_NO_DMG), Entity::default());
        }

        let skills = capture_3min_result_skills(&enc);
        assert!(
            !skills.contains_key(&UID_NO_DMG),
            "スキル0件のプレイヤーは空Vecを残さずキーごと落とす"
        );
    }

    // ─── 有効DPS（実働時間ベース）の回帰テスト ─────────────────────────────────

    // 1発しか当てていないプレイヤーでも、ActiveTime::record_event が初回イベントに
    // 猶予(500ms)を積むため有効DPSは0にならない（旧実装は初回イベントで間隔を積まず、
    // 一撃しか当てていないプレイヤーの有効DPSが常に0と表示されていた）。
    #[test]
    fn single_hit_player_active_dps_is_not_zero() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::entity::Entity;

        const UID: i64 = 701;
        let mut enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 10_000, // 実測スパン10秒(他プレイヤー等で進行)
            ..Default::default()
        };
        let mut p = Entity::default();
        p.dmg_stats.total = 100;
        p.active_dmg_time.record_event(2_000); // 一撃のみ
        enc.entities.insert(EntityKey::player(UID), p);

        let enc: EncounterMutex = std::sync::Mutex::new(enc);
        let window = get_dps_players(&enc);
        let row = window
            .player_rows
            .iter()
            .find(|r| r.uid as i64 == UID)
            .expect("single-hit player row present");
        assert!(
            row.active_value_per_sec > 0.0,
            "一撃のみでも有効DPSは0にならない(初回イベントの猶予500msが分母になる)"
        );
    }

    // 実働時間(active_ms)が実測スパンより長くなり得る極端なケース(例: 初回イベントの猶予が
    // 実測スパンそのものを上回る)でも、有効DPSは実測スパンでクランプされ、通常DPSを
    // 下回らない(≒この境界では一致する)。クランプが無いと有効DPSが通常DPSより低く出て
    // 「有効DPSは通常DPS以上」という前提が崩れる回帰を防ぐ。
    #[test]
    fn active_dps_is_clamped_to_real_elapsed_span_and_never_below_normal_dps() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::entity::Entity;

        const UID: i64 = 702;
        let mut enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 200, // 実測スパンはたった200ms
            ..Default::default()
        };
        let mut p = Entity::default();
        p.dmg_stats.total = 100;
        // 初回イベントの猶予500msは実測スパン200msを上回る
        // （クランプ無しだと active_secs=0.5s → active_dps=200、実測基準の通常DPS(500)を
        // 下回ってしまう＝「有効DPSが通常DPSを下回らない」に違反する）。
        p.active_dmg_time.record_event(9_999);
        enc.entities.insert(EntityKey::player(UID), p);

        let enc: EncounterMutex = std::sync::Mutex::new(enc);
        let window = get_dps_players(&enc);
        let row = window
            .player_rows
            .iter()
            .find(|r| r.uid as i64 == UID)
            .expect("player row present");
        assert_eq!(row.value_per_sec, 500.0, "通常DPS: 100 / 0.2s");
        assert_eq!(
            row.active_value_per_sec, 500.0,
            "有効DPSは実測スパン(200ms)でクランプされ、通常DPSと一致する(下回らない)"
        );
    }

    // Fix: 回復タブ(get_heal_players)は与ダメの実働時間(active_dmg_time)を分母に使わない。
    // 与ダメ・回復の両方をこなすハイブリッド構成でも、回復タブの有効DPS列は常に0
    // （「回復量÷与ダメ実働時間」という定義の無い値を出さない）。
    #[test]
    fn heal_tab_does_not_leak_dmg_active_time_into_active_dps() {
        use crate::engine::entity::Entity;

        const UID: i64 = 703;
        let mut enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 30_000,
            ..Default::default()
        };
        let mut p = Entity::default();
        p.dmg_stats.total = 100;
        p.heal_stats.total = 500;
        p.active_dmg_time.record_event(2_000); // 与ダメの実働時間は非ゼロ
        enc.entities.insert(EntityKey::player(UID), p);

        let enc: EncounterMutex = std::sync::Mutex::new(enc);
        let window = get_heal_players(&enc);
        let row = window
            .player_rows
            .iter()
            .find(|r| r.uid as i64 == UID)
            .expect("heal row present");
        assert_eq!(row.total_value, 500.0);
        assert_eq!(
            row.active_value_per_sec, 0.0,
            "回復タブは与ダメの実働時間を分母に使わない"
        );
    }

    // Fix: get_skills(StatType::Heal) の inspected_player も同様に与ダメの実働時間を
    // 漏らさない(現状 UI からは読まれない値だが、意味の無い値を渡し続けない)。
    #[test]
    fn get_skills_heal_inspected_player_active_dps_is_zero() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::entity::Entity;

        const UID: i64 = 704;
        let mut enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 30_000,
            ..Default::default()
        };
        let mut p = Entity::default();
        p.heal_stats.total = 500;
        p.active_dmg_time.record_event(2_000);
        enc.entities.insert(EntityKey::player(UID), p);

        let enc: EncounterMutex = std::sync::Mutex::new(enc);
        let sw = get_skills(&enc, UID, StatType::Heal).expect("skills window");
        assert_eq!(sw.inspected_player.active_value_per_sec, 0.0);
    }

    // Fix: 被ダメタブ(get_dmg_taken_attackers)の inspected_player も同様(被ダメ量÷自分が
    // 殴っていた時間、という定義の無い値を渡さない)。
    #[test]
    fn get_dmg_taken_attackers_inspected_player_active_dps_is_zero() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::entity::Entity;

        const UID: i64 = 705;
        let mut enc = Encounter {
            time_fight_start_ms: 1_000,
            time_last_combat_packet_ms: 1_000 + 30_000,
            ..Default::default()
        };
        let mut p = Entity::default();
        p.dmg_taken_stats.total = 300;
        p.active_dmg_time.record_event(2_000);
        enc.entities.insert(EntityKey::player(UID), p);

        let enc: EncounterMutex = std::sync::Mutex::new(enc);
        let sw = get_dmg_taken_attackers(&enc, UID).expect("skills window");
        assert_eq!(sw.inspected_player.active_value_per_sec, 0.0);
    }

    // party_only_consumables (runtime_settings atomic): 0ダメージ＋食事/シロップ持ちの
    // 特例行は、true のとき自分/PTメンバーに限られる（AOI appear 同期で街中の無関係
    // プレイヤー全員が並ぶのを防ぐ設定）。false なら全員表示（従来どおり）。
    // atomic はプロセス全体で共有されるため、テスト終了時（panic時含む）に既定値(true)へ
    // 戻すガードを使い、他テストとの並列実行での衝突を防ぐ。
    #[test]
    fn party_only_idle_consumable_filters_non_party_zero_damage_rows() {
        // selected_uid はプロセス共有のグローバル。この関数は間接的にその値を読むため、
        // 他テストの set と直列化しないと空の一覧を観測して間欠的に落ちる。
        let _guard = selected_uid::lock_for_test();
        selected_uid::set(None);
        use crate::engine::consumables::{PlayerConsumables, Timing};
        use crate::engine::entity::Entity;
        use crate::engine::runtime_settings;

        struct RestorePartyOnlyOnDrop;
        impl Drop for RestorePartyOnlyOnDrop {
            fn drop(&mut self) {
                runtime_settings::set_party_only_consumables(true);
            }
        }
        let _restore = RestorePartyOnlyOnDrop;

        const SELF_UID: i64 = 1;
        const PARTY_UID: i64 = 2;
        const STRANGER_UID: i64 = 3;

        fn idle_food() -> PlayerConsumables {
            PlayerConsumables {
                food: Some(Timing {
                    expire_at_ms: 999_999_999_999,
                    duration_ms: 60_000,
                    base_id: 1,
                    buff_uuid: 1,
                    create_time: 0,
                    layer: 1,
                    trusted: true,
                }),
                syrup: None,
            }
        }

        let mut enc = Encounter { local_player_uid: SELF_UID, ..Default::default() };
        enc.team.join(100, SELF_UID, [PARTY_UID]);
        for uid in [SELF_UID, PARTY_UID, STRANGER_UID] {
            enc.entities.insert(EntityKey::player(uid), Entity::default());
            enc.consumables.insert(uid, idle_food());
        }
        let enc: EncounterMutex = std::sync::Mutex::new(enc);

        // (a) party_only=true・非メンバー(ストレンジャー) → 行なし
        // (b) party_only=true・メンバー → 行あり
        // (d) 自分は常に行あり
        runtime_settings::set_party_only_consumables(true);
        let window = get_dps_players(&enc);
        let uids: Vec<i64> = window.player_rows.iter().map(|r| r.uid as i64).collect();
        assert!(!uids.contains(&STRANGER_UID), "party外の0ダメージ食事行は表示されない");
        assert!(uids.contains(&PARTY_UID), "PTメンバーの0ダメージ食事行は表示される");
        assert!(uids.contains(&SELF_UID), "自分の0ダメージ食事行は常に表示される");

        // (c) party_only=false → 全員表示（従来どおり）
        runtime_settings::set_party_only_consumables(false);
        let window_all = get_dps_players(&enc);
        let uids_all: Vec<i64> = window_all.player_rows.iter().map(|r| r.uid as i64).collect();
        assert!(uids_all.contains(&STRANGER_UID), "party_only=falseなら無関係プレイヤーも表示");
    }
}
