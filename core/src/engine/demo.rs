//! 撮影・UI 動作確認用のデモデータフィーダー。
//! slint-app を `BPSR_DEMO=1` で起動したときのみ呼ばれ、WinDivert キャプチャの
//! 代わりに実観測と同一の `process_scene_delta` 経路へ合成 SceneDelta
//! （ダメージ/回復/バフ）を流す。通常起動では一切使われない。

use crate::engine::encounter::EncounterMutex;
use crate::engine::entity::Entity;
use crate::engine::name_cache;
use crate::engine::processor::{self, now_ms};
use crate::protocol::pb::{self, EntityKind};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const PLAYER_TYPE: i64 = 640;
const MONSTER_TYPE: i64 = 64;
const SELF_UID: i64 = 90001;
const BOSS_UID: i64 = 80001;
const BOSS_MONSTER_ID: u32 = 118; // 訓練用ダミー（MonsterNameBoss.json 収録）

fn player_uuid(uid: i64) -> i64 {
    (uid << 16) | PLAYER_TYPE
}

fn boss_uuid() -> i64 {
    (BOSS_UID << 16) | MONSTER_TYPE
}

/// 依存追加を避けるための xorshift64 簡易乱数。
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        Self(now_ms() as u64 | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn f64(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f64()
    }
    fn chance(&mut self, p: f64) -> bool {
        self.f64() < p
    }
}

/// デモ用スキル（id は SkillName.json 収録の実 ID。先頭スキルで職業を推定させる）。
struct DemoSkill {
    id: i32,
    weight: f64,
    base: f64,
}

struct DemoPlayer {
    uid: i64,
    name: &'static str,
    class_id: i32,
    score: i32,
    season_lv: i32,
    season_str: i32,
    hits_per_sec: f64,
    crit: f64,
    /// DPS 波形の周期（秒）と位相。推移グラフに個性を出す。
    period: f64,
    phase: f64,
    skills: &'static [DemoSkill],
    heals: &'static [DemoSkill],
}

const PLAYERS: &[DemoPlayer] = &[
    DemoPlayer {
        uid: SELF_UID,
        name: "ソラ",
        class_id: 1, // ストームブレイド（雷刃型: 1714）
        score: 6480,
        season_lv: 78,
        season_str: 24,
        hits_per_sec: 2.3,
        crit: 0.38,
        period: 26.0,
        phase: 0.0,
        skills: &[
            DemoSkill { id: 1714, weight: 3.0, base: 22_000.0 },
            DemoSkill { id: 1717, weight: 2.5, base: 18_000.0 },
            DemoSkill { id: 1725, weight: 2.0, base: 14_000.0 },
            DemoSkill { id: 1726, weight: 2.0, base: 16_000.0 },
            DemoSkill { id: 1721, weight: 1.0, base: 55_000.0 },
            DemoSkill { id: 1734, weight: 0.5, base: 110_000.0 },
        ],
        heals: &[],
    },
    DemoPlayer {
        uid: 90002,
        name: "カエデ",
        class_id: 2, // フロストメイジ（氷牙型: 120901）
        score: 6210,
        season_lv: 75,
        season_str: 22,
        hits_per_sec: 1.7,
        crit: 0.32,
        period: 34.0,
        phase: 1.6,
        skills: &[
            DemoSkill { id: 120901, weight: 3.0, base: 26_000.0 },
            DemoSkill { id: 120902, weight: 2.5, base: 20_000.0 },
            DemoSkill { id: 1241, weight: 2.0, base: 15_000.0 },
        ],
        heals: &[],
    },
    DemoPlayer {
        uid: 90003,
        name: "ハヤテ",
        class_id: 11, // ディバインアーチャー（鷹弓型: 220112）
        score: 6035,
        season_lv: 72,
        season_str: 20,
        hits_per_sec: 2.0,
        crit: 0.30,
        period: 22.0,
        phase: 3.1,
        skills: &[
            DemoSkill { id: 220112, weight: 3.0, base: 18_000.0 },
            DemoSkill { id: 2203622, weight: 1.2, base: 48_000.0 },
        ],
        heals: &[],
    },
    DemoPlayer {
        uid: 90004,
        name: "ノクス",
        class_id: 4, // ゲイルランサー（烈風型: 1405）
        score: 5980,
        season_lv: 70,
        season_str: 19,
        hits_per_sec: 2.1,
        crit: 0.26,
        period: 30.0,
        phase: 4.4,
        skills: &[
            DemoSkill { id: 1405, weight: 3.0, base: 16_000.0 },
            DemoSkill { id: 1418, weight: 2.0, base: 22_000.0 },
            DemoSkill { id: 1419, weight: 1.5, base: 13_000.0 },
        ],
        heals: &[],
    },
    DemoPlayer {
        uid: 90005,
        name: "ミレイユ",
        class_id: 5, // ヴァーダントオラクル（威咲型: 1518）
        score: 6120,
        season_lv: 74,
        season_str: 21,
        hits_per_sec: 1.4,
        crit: 0.22,
        period: 28.0,
        phase: 2.2,
        skills: &[
            DemoSkill { id: 1518, weight: 2.0, base: 12_000.0 },
            DemoSkill { id: 21402, weight: 1.0, base: 14_000.0 },
        ],
        heals: &[
            DemoSkill { id: 20301, weight: 3.0, base: 22_000.0 },
            DemoSkill { id: 1541, weight: 1.0, base: 15_000.0 },
        ],
    },
    DemoPlayer {
        uid: 90006,
        name: "リコリス",
        class_id: 13, // ビートパフォーマー（響奏型: 2307）
        score: 5890,
        season_lv: 69,
        season_str: 18,
        hits_per_sec: 1.3,
        crit: 0.20,
        period: 36.0,
        phase: 5.0,
        skills: &[DemoSkill { id: 2306, weight: 2.0, base: 11_000.0 }],
        heals: &[
            DemoSkill { id: 2307, weight: 3.0, base: 16_000.0 },
            DemoSkill { id: 55302, weight: 1.0, base: 20_000.0 },
        ],
    },
    DemoPlayer {
        uid: 90007,
        name: "ティオ",
        class_id: 9, // ヘヴィガーディアン（剛守型: 1930）
        score: 5760,
        season_lv: 68,
        season_str: 17,
        hits_per_sec: 1.8,
        crit: 0.18,
        period: 24.0,
        phase: 0.9,
        skills: &[
            DemoSkill { id: 1930, weight: 3.0, base: 7_000.0 },
            DemoSkill { id: 1931, weight: 2.0, base: 8_000.0 },
            DemoSkill { id: 199902, weight: 0.8, base: 26_000.0 },
        ],
        heals: &[],
    },
    DemoPlayer {
        uid: 90008,
        name: "ユズリハ",
        class_id: 12, // シールドファイター（光盾型: 2406）
        score: 5830,
        season_lv: 71,
        season_str: 18,
        hits_per_sec: 1.7,
        crit: 0.20,
        period: 32.0,
        phase: 3.8,
        skills: &[
            DemoSkill { id: 2406, weight: 3.0, base: 9_000.0 },
            DemoSkill { id: 2405, weight: 2.0, base: 11_000.0 },
        ],
        heals: &[],
    },
];

/// 自キャラ（ソラ）のバフ/デバフ（base_id は buff_dictionary 収録の実 ID）。
/// (base_id, 持続ms, スタック数)
const SELF_BUFFS: &[(i32, i64, i32)] = &[
    (510011, 45_000, 1),  // 会心アップ
    (2110042, 38_000, 1), // ファスト
    (30501, 60_000, 1),   // 激励
    (55333, 25_000, 3),   // アンコール ×3
    (2110024, 15_000, 1), // 超会心
    (21421, 90_000, 1),   // ライフサージ
];
const SELF_DEBUFFS: &[(i32, i64, i32)] = &[
    (4501, 12_000, 1),   // 燃焼
    (802911, 18_000, 1), // 脆弱
];

/// イマジン重複使用無効デバフ（buff_source::classify_buff 対応 ID）。
/// (対象uid, base_id, 持続ms)
const IMAGINE_DEBUFFS: &[(i64, i32, i64)] = &[
    (SELF_UID, 2110056, 75_000), // ソラ: ティナ
    (SELF_UID, 2110055, 60_000), // ソラ: タータ
    (SELF_UID, 2110057, 90_000), // ソラ: アルーナ
    (90002, 2110057, 85_000),    // カエデ: アルーナ
    (90002, 2110050, 45_000),    // カエデ: バジリスク
    (90002, 2110056, 70_000),    // カエデ: ティナ
    (90003, 2110056, 75_000),    // ハヤテ: ティナ
    (90003, 2110055, 55_000),    // ハヤテ: タータ
    (90003, 2110050, 50_000),    // ハヤテ: バジリスク
    (90004, 2110055, 60_000),    // ノクス: タータ
    (90004, 2110057, 90_000),    // ノクス: アルーナ
    (90004, 2110050, 40_000),    // ノクス: バジリスク
];

/// 食事/シロップ（ConsumableBuffIds.json 収録 ID）。
/// (対象uid, base_id, 総持続ms, 消費済み割合)
const CONSUMABLES: &[(i64, i32, i64, f64)] = &[
    (SELF_UID, 2032011, 1_800_000, 0.60), // ソラ: 物攻+15（食事）
    (SELF_UID, 2033011, 1_800_000, 0.15), // ソラ: 火属性強度+240（シロップ）
    (90002, 2032021, 1_800_000, 0.80),    // カエデ: 魔攻+15（食事）
    (90002, 2033021, 1_800_000, 0.40),    // カエデ: 氷属性強度+240（シロップ）
    (90003, 2032011, 1_800_000, 0.10),    // ハヤテ: 物攻+15（食事）
    (90005, 2033031, 1_800_000, 0.65),    // ミレイユ: 森属性強度+240（シロップ）
    (90006, 2032021, 1_800_000, 0.35),    // リコリス: 魔攻+15（食事）
];

fn make_record(attacker_uuid: i64, skill: i32, value: i64, crit: bool, lucky: bool, heal: bool) -> pb::DamageRecord {
    pb::DamageRecord {
        is_miss: false,
        r#type: if heal {
            pb::DmgKind::Heal as i32
        } else {
            pb::DmgKind::Normal as i32
        },
        type_flag: if crit { 1 } else { 0 },
        value,
        lucky_value: if lucky { value } else { 0 },
        hp_lessen_value: value,
        attacker_uuid,
        owner_id: skill,
        is_dead: false,
        property: 0,
        top_summoner_id: 0,
        damage_mode: 0,
    }
}

fn pick_skill<'a>(rng: &mut Rng, skills: &'a [DemoSkill]) -> &'a DemoSkill {
    let total: f64 = skills.iter().map(|s| s.weight).sum();
    let mut r = rng.f64() * total;
    for s in skills {
        if r < s.weight {
            return s;
        }
        r -= s.weight;
    }
    &skills[skills.len() - 1]
}

/// 指定バフが無ければ付与する。`consumed_frac` 分だけ過去に付与した扱いにして
/// 残時間バーが一様に満タンにならないようにする。
fn ensure_buff(
    encounter: &mut crate::engine::encounter::Encounter,
    uid: i64,
    base_id: i32,
    duration_ms: i64,
    layer: i32,
    consumed_frac: f64,
    now: u128,
) {
    let alive = encounter
        .buff_tracker
        .snapshot_for(uid, now)
        .iter()
        .any(|s| s.base_id == base_id && s.remaining_ms > 500);
    if alive {
        return;
    }
    let info = pb::BuffSnapshot {
        buff_uuid: base_id,
        base_id,
        level: 1,
        host_uuid: player_uuid(uid),
        table_uuid: 0,
        create_time: 1_700_000_000_000 + uid + base_id as i64,
        fire_uuid: 0,
        layer,
        part_id: 0,
        count: layer,
        duration: duration_ms as i32,
        fight_source_info: None,
    };
    let backdated = now.saturating_sub((duration_ms as f64 * consumed_frac) as u128);
    encounter.buff_tracker.apply_buff_add(base_id, &info, backdated, uid);
}

/// 全プレイヤーの職業を先頭スキルで確定させる初回シード
/// （1ダメージ/1回復。表示順や統計への影響は無視できる規模）。
fn prime_entities(enc: &EncounterMutex) {
    for p in PLAYERS {
        let (skill, heal) = match (p.skills.first(), p.heals.first()) {
            (_, Some(h)) if p.class_id == 13 => (h.id, true), // リコリスは回復スキルで響奏型に
            (Some(s), _) => (s.id, false),
            (None, Some(h)) => (h.id, true),
            (None, None) => continue,
        };
        let target = if heal { player_uuid(p.uid) } else { boss_uuid() };
        let delta = pb::SceneDelta {
            uuid: target,
            attrs: None,
            buff_list: None,
            skill_effects: Some(pb::SkillImpact {
                damages: vec![make_record(player_uuid(p.uid), skill, 1, false, false, heal)],
            }),
        };
        if let Ok(mut e) = enc.lock() {
            processor::process_scene_delta(&mut e, delta);
        }
    }

    // 自キャラの戦闘ステータスをデモ値で投入（表示の器を確認するため）。
    // 値はゲーム内ステータス画面（docs/images の参考スクショ）のスケールに合わせる。
    // ※ 割合系は「値 / 100 = %」（例: crit_stat 2485 = 24.85%）。
    if let Ok(mut e) = enc.lock() {
        if let Some(ent) = e.entities.get_mut(&SELF_UID) {
            ent.curr_hp = Some(320_000);
            ent.max_hp = Some(398_624);
            ent.attack_power = Some(3_569); // 物理攻撃力
            ent.magic_attack = Some(0); // 魔法攻撃力
            ent.defense_power = Some(12_387); // 物理防御力
            ent.magic_defense = Some(179); // 魔法防御力
            ent.endurance = Some(28_342);
            ent.strength = Some(5_507);
            ent.intelligence = Some(543);
            ent.agility = Some(543);
            ent.season_strength = Some(3_184); // 幻夢強度
            ent.ability_score = Some(47_421); // 能力スコア
            ent.attack_speed = Some(3_124); // 31.24%
            ent.cast_speed = Some(3_040); // 30.4%
            ent.haste = Some(3_040); // ファスト 30.4%
            ent.lucky = Some(632); // 6.32%
            ent.crit_stat = Some(2_485); // 会心 24.85%
            ent.versatility = Some(2_647); // 万能 26.47%
            ent.resist = Some(0); // レジスト 0%
            ent.dexterity = Some(4_172); // 器用さ 41.72%
            ent.crit_dmg = Some(5_000); // 会心ダメージ 50%
            ent.lucky_dmg = Some(4_158); // 幸運の一撃倍率 41.58%
        }
    }
}

fn ensure_all_buffs(enc: &EncounterMutex, rng: &mut Rng, first: bool) {
    let now = now_ms();
    let Ok(mut e) = enc.lock() else {
        return;
    };
    for &(base_id, dur, layer) in SELF_BUFFS {
        let frac = if first { rng.range(0.1, 0.7) } else { 0.0 };
        ensure_buff(&mut e, SELF_UID, base_id, (dur as f64 * rng.range(0.9, 1.1)) as i64, layer, frac, now);
    }
    for &(base_id, dur, layer) in SELF_DEBUFFS {
        let frac = if first { rng.range(0.1, 0.6) } else { 0.0 };
        ensure_buff(&mut e, SELF_UID, base_id, (dur as f64 * rng.range(0.9, 1.2)) as i64, layer, frac, now);
    }
    for &(uid, base_id, dur) in IMAGINE_DEBUFFS {
        let frac = if first { rng.range(0.1, 0.8) } else { 0.0 };
        ensure_buff(&mut e, uid, base_id, (dur as f64 * rng.range(0.85, 1.15)) as i64, 1, frac, now);
    }
    if first {
        for &(uid, base_id, dur, frac) in CONSUMABLES {
            ensure_buff(&mut e, uid, base_id, dur, 1, frac, now);
        }
    }
}

/// デモフィーダーを開始する。WinDivert 不要・管理者権限不要。
pub fn spawn(enc: Arc<EncounterMutex>) {
    log::info!("demo feeder: starting (synthetic combat data, no capture)");

    for p in PLAYERS {
        name_cache::update(
            p.uid,
            Some(p.name),
            Some(p.class_id),
            Some(p.score),
            Some(p.season_lv),
            Some(p.season_str),
        );
    }

    if let Ok(mut e) = enc.lock() {
        e.local_player_uid = SELF_UID;
        e.entities.entry(BOSS_UID).or_insert_with(|| Entity {
            entity_type: EntityKind::Monster,
            monster_id: Some(BOSS_MONSTER_ID),
            curr_hp: Some(8_000_000_000),
            max_hp: Some(8_000_000_000),
            ..Default::default()
        });
    }

    let builder = thread::Builder::new().name("bpsr-demo".into());
    let spawn_result = builder.spawn(move || {
        // 観測ステータスもデモで再現する（緑ドット＝ゲーム通信受信中）
        crate::capture::status::set_state(crate::capture::status::STATE_RUNNING);
        let mut rng = Rng::new();
        prime_entities(&enc);
        ensure_all_buffs(&enc, &mut rng, true);

        let start = Instant::now();
        let tick = Duration::from_millis(250);
        let dt = 0.25_f64;
        // ヒット数の端数を持ち越す累積器（プレイヤー×[攻撃, 回復]）
        let mut acc = vec![[0.0_f64; 2]; PLAYERS.len()];
        let mut tick_count: u64 = 0;

        loop {
            crate::capture::status::mark_packet();
            crate::capture::status::mark_game_packet();
            let t = start.elapsed().as_secs_f64();
            let mut boss_damages: Vec<pb::DamageRecord> = Vec::new();
            let mut heal_deltas: Vec<pb::SceneDelta> = Vec::new();

            for (i, p) in PLAYERS.iter().enumerate() {
                // 波形（周期変動）＋約28秒ごとのバーストでグラフに起伏を出す
                let wave = 0.72 + 0.28 * (t * std::f64::consts::TAU / p.period + p.phase).sin();
                let burst = if (t + p.phase * 4.0) % 28.0 < 4.5 { 1.65 } else { 1.0 };
                let activity = wave * burst;

                acc[i][0] += p.hits_per_sec * dt * activity;
                while acc[i][0] >= 1.0 {
                    acc[i][0] -= 1.0;
                    let s = pick_skill(&mut rng, p.skills);
                    let crit = rng.chance(p.crit);
                    let lucky = rng.chance(0.06);
                    let mut v = s.base * rng.range(0.85, 1.15) * activity;
                    if crit {
                        v *= 2.0;
                    }
                    boss_damages.push(make_record(
                        player_uuid(p.uid),
                        s.id,
                        v as i64,
                        crit,
                        lucky,
                        false,
                    ));
                }

                if !p.heals.is_empty() {
                    acc[i][1] += 1.2 * dt * wave;
                    while acc[i][1] >= 1.0 {
                        acc[i][1] -= 1.0;
                        let s = pick_skill(&mut rng, p.heals);
                        let crit = rng.chance(0.15);
                        let mut v = s.base * rng.range(0.85, 1.15);
                        if crit {
                            v *= 2.0;
                        }
                        let target = PLAYERS[(rng.next() % PLAYERS.len() as u64) as usize].uid;
                        heal_deltas.push(pb::SceneDelta {
                            uuid: player_uuid(target),
                            attrs: None,
                            buff_list: None,
                            skill_effects: Some(pb::SkillImpact {
                                damages: vec![make_record(
                                    player_uuid(p.uid),
                                    s.id,
                                    v as i64,
                                    crit,
                                    false,
                                    true,
                                )],
                            }),
                        });
                    }
                }
            }

            if !boss_damages.is_empty() {
                let delta = pb::SceneDelta {
                    uuid: boss_uuid(),
                    attrs: None,
                    buff_list: None,
                    skill_effects: Some(pb::SkillImpact { damages: boss_damages }),
                };
                if let Ok(mut e) = enc.lock() {
                    processor::process_scene_delta(&mut e, delta);
                }
            }
            for delta in heal_deltas {
                if let Ok(mut e) = enc.lock() {
                    processor::process_scene_delta(&mut e, delta);
                }
            }

            // 約2秒に1回、ボスがタンクを殴る（被ダメタブ用）
            if tick_count % 8 == 0 && rng.chance(0.9) {
                let tank = if rng.chance(0.7) { 90007 } else { 90008 };
                let delta = pb::SceneDelta {
                    uuid: player_uuid(tank),
                    attrs: None,
                    buff_list: None,
                    skill_effects: Some(pb::SkillImpact {
                        damages: vec![make_record(
                            boss_uuid(),
                            999_001,
                            rng.range(25_000.0, 90_000.0) as i64,
                            rng.chance(0.1),
                            false,
                            false,
                        )],
                    }),
                };
                if let Ok(mut e) = enc.lock() {
                    processor::process_scene_delta(&mut e, delta);
                }
            }

            // 1秒ごとにバフ補充（期限切れの再付与）
            if tick_count % 4 == 0 {
                ensure_all_buffs(&enc, &mut rng, false);
            }

            tick_count += 1;
            thread::sleep(tick);
        }
    });
    if let Err(e) = spawn_result {
        log::error!("demo feeder: failed to spawn thread: {e}");
    }
}
