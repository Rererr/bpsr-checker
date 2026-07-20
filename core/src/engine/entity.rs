use crate::models::TimeSeriesPoint;
use crate::engine::class::{Class, ClassSpec};
use crate::engine::combat_stats::CombatStats;
use crate::protocol::pb::EntityKind;
use std::collections::{HashMap, VecDeque};

/// バトルイマジンの装備枠数（SlotPositionId 7/8）。プレイヤーが同時に装備・表示できる
/// バトルイマジンは最大この数。`imagines`（確定・表示用）の上限として processor/compute が共有する。
pub const MAX_IMAGINE_NAMES: usize = 2;

/// ロールスキル(簡易版バトルイマジン、S3で追加された職業ユーティリティ枠)の装備枠数
/// （SlotPositionId 21-24）。プレイヤーが同時に装備・表示できるロールスキルは最大この数。
/// `role_skill_imagines`（確定・表示用）の上限として processor/compute が共有する。
pub const MAX_ROLE_SKILL_IMAGINES: usize = 4;

#[derive(Debug, Default, Clone, Copy)]
pub struct SkillMeta {
    pub property: u8,
    pub damage_mode: u8,
}

/// 検知済みバトルイマジン1枠。`last_seen` は wall-clock ではなく processor が発行する
/// 単調増加の検知シーケンス番号（`next_imagine_seq`）。鮮度比較にのみ使う。
///
/// `tier` はイマジンレベル（凸数。召喚 spawn の `ATTR_SKILL_REMODEL_LEVEL` から取得）。
/// 0 は「未判明」を意味し表示に `(N)` を付けない。再検知で非0が来たら更新する。
///
/// `pending_hits` は `pending_imagine` としての再検知回数（休眠相方によるスタック自己修復の
/// 判定に使う）。**確定済み（`imagines` 内）のスロットでは常に 0 で無視する**フィールド。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagineSlot {
    pub name: String,
    pub last_seen: u64,
    pub tier: i32,
    pub pending_hits: u32,
}

#[derive(Debug, Default, Clone)]
pub struct Entity {
    pub entity_type: EntityKind,

    pub dmg_stats: CombatStats,
    pub skill_uid_to_dps_stats: HashMap<i32, CombatStats>,
    pub skill_meta: HashMap<i32, SkillMeta>,

    pub dmg_stats_boss_only: CombatStats,
    pub skill_uid_to_dps_stats_boss_only: HashMap<i32, CombatStats>,

    pub heal_stats: CombatStats,
    pub skill_uid_to_heal_stats: HashMap<i32, CombatStats>,

    pub dmg_taken_stats: CombatStats,
    pub attacker_uid_to_dmg_taken_stats: HashMap<i64, CombatStats>,
    pub attacker_skill_to_dmg_taken_stats: HashMap<(i64, i32), CombatStats>,

    // Players
    pub name: Option<String>,
    pub class: Option<Class>,
    pub class_spec: Option<ClassSpec>,
    pub ability_score: Option<i32>,
    pub season_level: Option<i32>,
    pub season_strength: Option<i32>,

    // Player combat stats (主に自キャラ。パケット attr から取得し戦闘中も追従する)
    // 命名はゲーム内ステータス画面の表記に合わせる（物理/魔法攻撃力, ファスト=haste など）。
    // ※ 整数系: attack_power(物攻) / magic_attack(魔攻) / defense_power(物防) / magic_defense(魔防)
    //          / endurance(耐久) / strength(筋力) / intelligence(知力) / agility(敏捷)
    // ※ 割合系(値/100=%): attack_speed(攻撃速度) / cast_speed(詠唱速度) / haste(ファスト)
    //          / lucky(幸運) / crit_stat(会心) / versatility(万能) / resist(レジスト)
    //          / crit_dmg(会心ダメージ) / lucky_dmg(幸運の一撃倍率) / dexterity(器用さ)
    // ※ attack_power / defense_power / endurance / dexterity / attack_speed / haste / lucky は
    //   旧 probe で decode 配線済み。それ以外（magic_*/agility/crit_stat/versatility/resist/
    //   cast_speed/crit_dmg/lucky_dmg）は attr_id 未確定で decode 未配線（実機 probe で確定後に配線）。
    pub attack_power: Option<i32>,
    pub magic_attack: Option<i32>,
    pub defense_power: Option<i32>,
    pub magic_defense: Option<i32>,
    pub endurance: Option<i32>,
    pub strength: Option<i32>,
    pub intelligence: Option<i32>,
    pub agility: Option<i32>,
    pub dexterity: Option<i32>,
    pub attack_speed: Option<i32>,
    pub cast_speed: Option<i32>,
    pub haste: Option<i32>,
    pub lucky: Option<i32>,
    pub crit_stat: Option<i32>,
    pub versatility: Option<i32>,
    pub resist: Option<i32>,
    pub crit_dmg: Option<i32>,
    pub lucky_dmg: Option<i32>,

    /// 表示用に確定したバトルイマジン（召喚エンティティの `AttrSkillId` から解決・重複なし）。
    /// 定員 [`MAX_IMAGINE_NAMES`] 未満なら新規名は無条件で確定に追加されるが、定員一杯の状態で
    /// 新規名を検知しても**この Vec は変えない**（`pending_imagine` へ保留する）。確定を書き換えるのは
    /// 「ゲーム上あり得ない事象＝既に外れた枠の再spawn」が観測できたときだけ（rule1/5、pending 方式）。
    /// これにより画面に新旧混在ペアが一瞬でも出ることを防ぐ。表示順（Vec の並び）は挿入順で安定し、
    /// 再検知しても並べ替えない。compute/UI は [`Entity::imagine_display_names`] 経由で名前だけを読む。
    /// 更新は processor の `try_attribute_summon_imagine` 経由、永続は name_cache。
    /// 詳細は [[imagine-2stage-display]] [[imagine-pending-confirm]]。
    pub imagines: Vec<ImagineSlot>,

    /// 定員一杯の状態で検知した「まだ確証の無い」新規イマジン候補（最大1件）。表示には一切使わない
    /// （`imagine_display_names` は `imagines` のみ参照）。既存スロットの再検知（＝現役の確定証拠）が
    /// 得られた時か、pending とは別の新規名がもう1件検知された時（＝両枠同時交換の確定）にのみ
    /// `imagines` へ昇格・反映され、それまでは保留され続ける。`clear_combat_stats` で Entity ごと
    /// 破棄される（保留状態はグループ境界を跨がない）。
    pub pending_imagine: Option<ImagineSlot>,

    /// ロールスキル(簡易版バトルイマジン、S3で追加された職業ユーティリティ枠)の検知結果
    /// （最大 [`MAX_ROLE_SKILL_IMAGINES`] 件、SlotPositionId 21-24）。実イマジン2枠
    /// （`imagines`/`pending_imagine`）とは完全に独立した別枠であり、ロールスキルの対象は
    /// 装備中の実イマジン2枠のいずれかである必要が無い。権威的に更新されるのは
    /// `apply_skill_list_imagines`（フル装備スキルリスト attr116）からのみで、召喚ベースの
    /// ヒューリスティック（`try_attribute_summon_imagine`）から直接セットされることはない
    /// （同一の召喚シグナルが実イマジンとロールスキル両方から発生し得るため、真偽の判定は
    /// attr116 の権威的スナップショットにのみ委ねる）。`pending_hits` は未使用（`ImagineSlot`
    /// 再利用のため常に0のまま無視してよい）。
    pub role_skill_imagines: Vec<ImagineSlot>,

    // Monsters（curr_hp / max_hp は自キャラの HP にも流用する）
    pub monster_id: Option<u32>,
    pub curr_hp: Option<u64>,
    pub max_hp: Option<u64>,

    // Per-entity DPS time series (sampled alongside encounter-wide series)
    pub time_series: VecDeque<TimeSeriesPoint>,
    pub last_sample_total_dmg: i64,

    // Per-skill DPS time series（スキル別の推移グラフ用。entity の time_series と同タイミングで採取）
    pub skill_time_series: HashMap<i32, VecDeque<TimeSeriesPoint>>,
    pub skill_last_sample_total_dmg: HashMap<i32, i64>,
}

impl Entity {
    /// 表示用のイマジン名一覧（挿入順=表示順で安定）。名前のみ（凸数なし）。
    /// 検知ロジックの一致判定・name_cache 永続化はこちらを使う。
    pub fn imagine_display_names(&self) -> Vec<String> {
        self.imagines.iter().map(|s| s.name.clone()).collect()
    }

    /// イマジン凸数一覧（`imagine_display_names` と同順の並列配列）。name_cache 永続化用。
    pub fn imagine_tiers(&self) -> Vec<i32> {
        self.imagines.iter().map(|s| s.tier).collect()
    }

    /// 表示用のイマジンラベル一覧。ユーザー上書き（`imagine_overrides`）を解決した上で、
    /// 凸数が判明していれば「名前(N)」、未判明なら名前のみ。IGNORE設定の枠は結果から除外する。
    /// compute/UI の表示はこちらを読む。canonical名の一致判定・永続化には上書き非適用の
    /// [`Entity::imagine_display_names`] / [`Entity::imagine_tiers`] を使う。
    pub fn imagine_display_labels(&self) -> Vec<String> {
        self.imagines
            .iter()
            .filter_map(|s| {
                let display = crate::engine::imagine_overrides::resolve_display(&s.name)?;
                Some(if s.tier > 0 {
                    format!("{}({})", display, s.tier)
                } else {
                    display
                })
            })
            .collect()
    }

    /// ロールスキル(簡易版バトルイマジン)の検知名一覧（挿入順で安定）。名前のみ（凸数なし）。
    /// 検知ロジックの一致判定・name_cache 永続化はこちらを使う（`imagine_display_names` の
    /// ロールスキル版）。
    pub fn role_skill_imagine_names(&self) -> Vec<String> {
        self.role_skill_imagines.iter().map(|s| s.name.clone()).collect()
    }

    /// ロールスキルの凸数一覧（`role_skill_imagine_names` と同順の並列配列）。name_cache 永続化用
    /// （`imagine_tiers` のロールスキル版）。
    pub fn role_skill_imagine_tiers(&self) -> Vec<i32> {
        self.role_skill_imagines.iter().map(|s| s.tier).collect()
    }

    /// ロールスキル(簡易版バトルイマジン)の表示ラベル一覧（"名前(N)"形式）。imagine_display_labels
    /// と全く同じパターンでユーザー上書き(imagine_overrides)を解決し、IGNORE設定の枠は結果から
    /// 除外する。未検知（`role_skill_imagines` が空）なら空 Vec。表示件数の [`MAX_ROLE_SKILL_IMAGINES`]
    /// への丸めは `imagine_display_labels` 同様ここでは行わない（表示層の `format_imagine_suffix`
    /// が保険としてそちらを担う）。
    pub fn role_skill_imagine_labels(&self) -> Vec<String> {
        self.role_skill_imagines
            .iter()
            .filter_map(|s| {
                let display = crate::engine::imagine_overrides::resolve_display(&s.name)?;
                Some(if s.tier > 0 {
                    format!("{}({})", display, s.tier)
                } else {
                    display
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::imagine_overrides;

    fn slot(name: &str, tier: i32) -> ImagineSlot {
        ImagineSlot {
            name: name.to_string(),
            last_seen: 0,
            tier,
            pending_hits: 0,
        }
    }

    #[test]
    fn imagine_display_labels_uses_canonical_when_no_override() {
        let _guard = crate::engine::imagine_test_support::guard();
        let canonical = "__entity_test_no_override__";
        imagine_overrides::clear(canonical);
        let entity = Entity {
            imagines: vec![slot(canonical, 0)],
            ..Default::default()
        };
        assert_eq!(entity.imagine_display_labels(), vec![canonical.to_string()]);
    }

    #[test]
    fn imagine_display_labels_applies_display_override_and_keeps_tier_suffix() {
        let _guard = crate::engine::imagine_test_support::guard();
        let canonical = "__entity_test_display__";
        imagine_overrides::set_display(canonical, Some("表示上書き".to_string()));
        let entity = Entity {
            imagines: vec![slot(canonical, 3)],
            ..Default::default()
        };
        assert_eq!(
            entity.imagine_display_labels(),
            vec!["表示上書き(3)".to_string()]
        );
        // canonical名を返す系（一致判定・永続化用）は上書き非適用のまま。
        assert_eq!(entity.imagine_display_names(), vec![canonical.to_string()]);
        imagine_overrides::clear(canonical);
    }

    #[test]
    fn imagine_display_labels_hides_ignored_slot_but_keeps_sibling() {
        let _guard = crate::engine::imagine_test_support::guard();
        let canonical = "__entity_test_ignored__";
        let sibling = "__entity_test_ignored_sibling__";
        imagine_overrides::clear(sibling);
        imagine_overrides::set_ignored(canonical, true);
        let entity = Entity {
            imagines: vec![slot(canonical, 0), slot(sibling, 0)],
            ..Default::default()
        };
        assert_eq!(entity.imagine_display_labels(), vec![sibling.to_string()]);
        // imagine_display_names は上書き非適用なので ignored でも両方残る。
        assert_eq!(entity.imagine_display_names().len(), 2);
        imagine_overrides::clear(canonical);
    }

    #[test]
    fn role_skill_imagine_labels_uses_canonical_when_no_override() {
        let _guard = crate::engine::imagine_test_support::guard();
        let canonical = "__entity_test_role_skill_no_override__";
        imagine_overrides::clear(canonical);
        let entity = Entity {
            role_skill_imagines: vec![slot(canonical, 0)],
            ..Default::default()
        };
        assert_eq!(
            entity.role_skill_imagine_labels(),
            vec![canonical.to_string()]
        );
    }

    #[test]
    fn role_skill_imagine_labels_applies_display_override_and_keeps_tier_suffix() {
        let _guard = crate::engine::imagine_test_support::guard();
        let canonical = "__entity_test_role_skill_display__";
        imagine_overrides::set_display(canonical, Some("表示上書き".to_string()));
        let entity = Entity {
            role_skill_imagines: vec![slot(canonical, 3)],
            ..Default::default()
        };
        assert_eq!(
            entity.role_skill_imagine_labels(),
            vec!["表示上書き(3)".to_string()]
        );
        imagine_overrides::clear(canonical);
    }

    #[test]
    fn role_skill_imagine_labels_hides_ignored_slot_but_keeps_sibling() {
        let _guard = crate::engine::imagine_test_support::guard();
        let canonical = "__entity_test_role_skill_ignored__";
        let sibling = "__entity_test_role_skill_ignored_sibling__";
        imagine_overrides::clear(sibling);
        imagine_overrides::set_ignored(canonical, true);
        let entity = Entity {
            role_skill_imagines: vec![slot(canonical, 0), slot(sibling, 0)],
            ..Default::default()
        };
        assert_eq!(entity.role_skill_imagine_labels(), vec![sibling.to_string()]);
        imagine_overrides::clear(canonical);
    }

    #[test]
    fn role_skill_imagine_labels_is_empty_when_undetected() {
        let entity = Entity::default();
        assert!(entity.role_skill_imagine_labels().is_empty());
    }

    // ロールスキルは最大4枠(SlotPositionId 21-24)を同時装備できる。3〜4件同時に検知しても
    // 全件が欠落なく解決・表示されることを確認する（実イマジン2枠側の複数件テストと同じ形）。
    #[test]
    fn role_skill_imagine_labels_resolves_all_simultaneous_slots() {
        let _guard = crate::engine::imagine_test_support::guard();
        let a = "__entity_test_role_skill_multi_a__";
        let b = "__entity_test_role_skill_multi_b__";
        let c = "__entity_test_role_skill_multi_c__";
        let d = "__entity_test_role_skill_multi_d__";
        for name in [a, b, c, d] {
            imagine_overrides::clear(name);
        }
        let entity = Entity {
            role_skill_imagines: vec![slot(a, 1), slot(b, 0), slot(c, 5), slot(d, 0)],
            ..Default::default()
        };
        assert_eq!(
            entity.role_skill_imagine_labels(),
            vec![
                format!("{a}(1)"),
                b.to_string(),
                format!("{c}(5)"),
                d.to_string(),
            ]
        );
    }
}
