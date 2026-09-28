use crate::models::TimeSeriesPoint;
use crate::engine::buff_tracker::BuffTracker;
use crate::engine::combat_stats::{ActiveTime, CombatStats};
use crate::engine::entity::{Entity, EntityKey};
use std::collections::{HashMap, HashSet, VecDeque};

pub type EncounterMutex = std::sync::Mutex<Encounter>;

/// 計測ボタンで始めた計測の絞り込み条件。
///
/// 開始時の設定値をここへコピーして `MeasureMode` が運ぶ（`duration_ms` と同じ扱い）。
/// 走行中に設定を変えても計測結果がぶれない。通常モード（[`MeasureMode::Normal`]）は
/// バリアントとして条件を持たないため、既定値＝絞り込み無しであることが型で保証される。
/// 履歴（`EncounterSnapshot`）へ焼き込むため serde 可能にしてある。フィールド単位の
/// `default` で、旧 history.json（条件フィールドを持たない）も絞り込み無しとして読める。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeasureScope {
    /// 自分が最初にダメージを与えた対象への与ダメージだけを集計する。
    ///
    /// こちらは読み出し時に絞れない。`Entity` は攻撃者ごとに分かれているだけで対象別の内訳を
    /// 持たないため、集計後に対象別へ分解する手段が無い。取り込み時に落とすしかない
    /// （＝計測中に切り替えても遡って復元できない）。
    ///
    /// **単体の対象を殴っている前提が崩れると数字が大きく変わる**。2026-08-26 の実測では、
    /// 複数の木人を叩く計測で自分の火力の 99.6%、ダンジョンの乱戦で 90% から 98% が落ちた。
    pub first_target_only: bool,
    /// 自分の記録だけを集計・表示する。
    ///
    /// 実装は取り込み時のフィルタではなく読み出し時の射影（compute.rs）。`Entity` は
    /// 攻撃者ごとに分かれているため、取り込み時に他人を捨てても自分の数値は変わらず、
    /// 消えるのは他人のデータだけになる（＝不可逆な情報破壊と引き換えに何も得られない）。
    pub self_only: bool,
}

#[derive(Debug, Clone, Default)]
pub enum MeasureMode {
    #[default]
    Normal,
    Pending3Min {
        duration_ms: u128,
        scope: MeasureScope,
    },
    Active3Min {
        armed_at_ms: u128,
        duration_ms: u128,
        scope: MeasureScope,
    },
}

#[derive(Debug, Default, Clone)]
pub struct Encounter {
    pub is_paused: bool,
    pub time_fight_start_ms: u128,
    pub time_last_combat_packet_ms: u128,
    /// 観測中の全エンティティ。キーは種別コードを含む [`EntityKey`]（＝パケットの UUID）。
    /// `uuid >> 16` で束ねるとプレイヤーと同番号のモンスター/召喚体が同じ Entity を共有し、
    /// PT メンバーが一覧から消える（[`EntityKey`] のドキュメント参照）。
    pub entities: HashMap<EntityKey, Entity>,
    pub dmg_stats: CombatStats,
    pub dmg_stats_boss_only: CombatStats,
    pub heal_stats: CombatStats,
    pub dmg_taken_stats: CombatStats,
    pub time_series: VecDeque<TimeSeriesPoint>,
    pub last_sample_ms: u128,
    pub last_sample_total_dmg: i64,
    pub local_player_uid: i64,
    pub has_selected_participant: bool,
    pub participant_player_uids: HashSet<i64>,
    pub measure_mode: MeasureMode,
    pub active_connection: Option<crate::capture::server::Server>,
    pub conn_to_uid: std::collections::HashMap<crate::capture::server::Server, i64>,
    pub buff_tracker: BuffTracker,
    /// 食事/シロップバフの残時間ストア。clear_combat_stats・手動リセットでは消さず
    /// （戦闘終了後もゲーム内効果は継続）、自然失効・履歴クリアでのみ消す。
    /// consumables.json にディスク永続化され、アプリ再起動後に復元される。
    pub consumables: std::collections::HashMap<i64, crate::engine::consumables::PlayerConsumables>,
    /// 初撃対象ロック（[`MeasureScope::first_target_only`]）の対象。
    /// 解除は `clear_combat_stats` の1箇所だけ（通常のロールオーバー・手動リセット・計測の
    /// 開始/中止/確定がすべてそこを通るため、解除条件を書き足す必要が無い）。
    pub locked_target: Option<EntityKey>,
    /// パーティ(PT)構成。consumables と同様、PT構成はキャラ選択・戦闘状態と無関係に
    /// アプリ全体で使うため clear_combat_stats・ServerHandover を跨いで保持する
    /// （戦闘リセットのたびに PT情報が消えると「PTメンバーのみ食事行表示」フィルタが
    /// リセット直後だけ全員非表示になってしまう）。
    pub team: crate::engine::team::TeamState,
    /// 自キャラの最新シーン(level_map_id)。consumables/team と同様、戦闘状態と無関係に
    /// アプリ全体で使うため clear_combat_stats・ServerHandover を跨いで保持する。
    pub current_level_map_id: u32,
    /// 戦闘開始の瞬間に `current_level_map_id` を写した値。「この計測がどこで行われたか」
    /// の記録用で、戦闘中にシーンが変わっても遡って書き換えない。`clear_combat_stats` で
    /// 0 に戻す（次の戦闘開始時に改めて写される）。
    pub fight_level_map_id: u32,
}

impl Encounter {
    /// `local_player_uid` を更新する。processor.rs 側の自動検出経路（should_accept /
    /// learn_connection / process_world_enter_snapshot / process_enter_scene）はすべて
    /// このメソッドを経由すること（同じ判定を複数箇所に書かない）。
    /// 旧値が非0で、かつ異なる非0の新値へ切り替わるときだけ team をクリアする
    /// （PT構成はキャラ単位。別キャラへの切替・再ログインで前キャラの PT を引き継がない）。
    /// 0→X（初回確定）・X→0 はここでいう「切替」ではないため対象外
    /// （明示的な手動切替は compute::set_selected_uid が別途無条件でクリアする）。
    pub fn set_local_player_uid(&mut self, uid: i64) {
        if self.local_player_uid != 0 && uid != 0 && self.local_player_uid != uid {
            self.team = crate::engine::team::TeamState::default();
        }
        self.local_player_uid = uid;
    }

    /// 自キャラのプレイヤー UID。`selected_uid`（手動指定）が優先で、無ければ自動検出値。
    /// 未確定なら `None`。
    ///
    /// 取り込み側のゲート（processor）と表示側の射影（compute）が同じ「自分」を指すための
    /// 単一定義。片方だけ導出規則が違うと、集計している人と表示している人がずれる。
    pub fn self_player_uid(&self) -> Option<i64> {
        let uid = crate::engine::selected_uid::get().unwrap_or(self.local_player_uid);
        (uid != 0).then_some(uid)
    }

    /// [`Self::self_player_uid`] を `entities` のキーへ変換したもの。
    pub fn self_player_key(&self) -> Option<EntityKey> {
        self.self_player_uid().map(EntityKey::player)
    }

    /// 現在の計測スコープ。通常モードは既定値（絞り込み無し）。
    /// `MeasureMode` を分解して scope を取り出す唯一の場所（同じ match を複数箇所に書かない）。
    pub fn measure_scope(&self) -> MeasureScope {
        match self.measure_mode {
            MeasureMode::Normal => MeasureScope::default(),
            MeasureMode::Pending3Min { scope, .. } | MeasureMode::Active3Min { scope, .. } => scope,
        }
    }

    /// 「戦闘中」判定の純粋版（`timeout_ms` を明示注入できる。テスト用途）。
    /// 最終着弾(`time_last_combat_packet_ms`)から `timeout_ms` 以内なら戦闘中とみなす
    /// （processor.rs の旧ロールオーバー判定 `diff > timeout_ms` と境界を一致させるため
    /// `<=` を使う。`<` にすると diff == timeout_ms の1点だけロールオーバーが早まる）。
    /// `timeout_ms == 0`（タイムアウト無効化設定）のときは常に戦闘中とみなす
    /// （ロールオーバー自体が発火しない既存仕様と合わせる）。一度も戦闘していなければ常に false。
    pub fn is_combat_active_with_timeout(&self, now: u128, timeout_ms: u128) -> bool {
        if self.time_last_combat_packet_ms == 0 {
            return false;
        }
        if timeout_ms == 0 {
            return true;
        }
        now.saturating_sub(self.time_last_combat_packet_ms) <= timeout_ms
    }

    /// 「戦闘中」判定（実運用版・`runtime_settings::COMBAT_EXIT_TIMEOUT_MS` を使う）。
    /// processor.rs のロールオーバー判定が呼ぶ唯一の定義（他に同じ述語を書かない）。
    /// ライブ表示の分母（compute.rs）は「戦闘中は現在時刻を分母にする」設計を撤回済みのため
    /// 本メソッドを参照しない（通常モードは実測スパン固定、3分計測は armed_at 基準）。
    pub fn is_combat_active(&self, now: u128) -> bool {
        let timeout_ms = u128::from(
            crate::engine::runtime_settings::COMBAT_EXIT_TIMEOUT_MS
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        self.is_combat_active_with_timeout(now, timeout_ms)
    }

    /// Player entities are removed so their identity is re-populated fresh
    /// from disk cache on next appearance. Monster entities are kept with stats
    /// reset so HP/monster_id tracking survives the rollover.
    pub fn clear_combat_stats(&mut self) {
        // active_connection と conn_to_uid は保持する。
        // これらはセッション間でコネクション識別に再利用するため、
        // ServerHandover 受信時と set_selected_uid 変更時のみクリアする。
        // current_level_map_id も consumables/team と同様に保持する
        // （戦闘リセットとシーン移動は無関係）。
        self.is_paused = false;
        self.time_fight_start_ms = 0;
        self.fight_level_map_id = 0;
        self.time_last_combat_packet_ms = 0;
        self.dmg_stats = CombatStats::default();
        self.dmg_stats_boss_only = CombatStats::default();
        self.heal_stats = CombatStats::default();
        self.dmg_taken_stats = CombatStats::default();
        self.time_series.clear();
        self.last_sample_ms = 0;
        self.last_sample_total_dmg = 0;
        self.has_selected_participant = false;
        self.participant_player_uids.clear();
        self.locked_target = None;
        self.buff_tracker.clear();
        self.entities.retain(|key, _| !key.is_player());
        for entity in self.entities.values_mut() {
            entity.dmg_stats = CombatStats::default();
            entity.dmg_stats_boss_only = CombatStats::default();
            entity.heal_stats = CombatStats::default();
            entity.dmg_taken_stats = CombatStats::default();
            entity.active_dmg_time = ActiveTime::default();
            entity.skill_uid_to_dps_stats.clear();
            entity.skill_uid_to_dps_stats_boss_only.clear();
            entity.skill_uid_to_heal_stats.clear();
            entity.skill_meta.clear();
            entity.attacker_uid_to_dmg_taken_stats.clear();
            entity.attacker_skill_to_dmg_taken_stats.clear();
            entity.time_series.clear();
            entity.last_sample_total_dmg = 0;
            entity.heal_time_series.clear();
            entity.last_sample_total_heal = 0;
            entity.dmg_taken_time_series.clear();
            entity.last_sample_total_dmg_taken = 0;
            entity.skill_time_series.clear();
            entity.skill_last_sample_total_dmg.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // timeout_ms==0（タイムアウト無効化設定）はどれだけ経過しても常に戦闘中とみなす
    // （processor.rs のロールオーバー自体が発火しない既存仕様と合わせる）。
    #[test]
    fn is_combat_active_with_timeout_never_expires_when_disabled() {
        let enc = Encounter {
            time_last_combat_packet_ms: 1_000,
            ..Default::default()
        };
        assert!(enc.is_combat_active_with_timeout(1_000_000_000, 0));
    }

    // 一度も戦闘していなければ（time_last_combat_packet_ms==0）常に false。
    #[test]
    fn is_combat_active_with_timeout_false_when_never_fought() {
        let enc = Encounter::default();
        assert!(!enc.is_combat_active_with_timeout(999_999, 8_000));
    }

    // 境界は processor.rs の旧ロールオーバー判定 `diff > timeout_ms` と一致させるため `<=`。
    // diff == timeout_ms はまだ戦闘中（ロールオーバーしない）、diff > timeout_ms で戦闘終了。
    #[test]
    fn is_combat_active_with_timeout_boundary_matches_legacy_gt_comparison() {
        let enc = Encounter {
            time_last_combat_packet_ms: 1_000,
            ..Default::default()
        };
        assert!(enc.is_combat_active_with_timeout(1_000 + 8_000, 8_000), "diff==timeoutはまだ戦闘中");
        assert!(!enc.is_combat_active_with_timeout(1_000 + 8_001, 8_000), "diff>timeoutで戦闘終了");
    }

    // 有効DPS（実働時間ベース）のトラッカー(Entity::active_dmg_time)は
    // clear_combat_stats で他の集計と同じく0へ戻る（Encounter 側にも同名フィールドが
    // あったが、production コードから一度も読まれなかったため削除済み）。
    #[test]
    fn clear_combat_stats_resets_active_dmg_time() {
        let mut enc = Encounter::default();
        let entity = enc.entities.entry(EntityKey::monster(1)).or_default();
        entity.active_dmg_time.record_event(1_000);
        entity.active_dmg_time.record_event(2_000);
        assert_ne!(entity.active_dmg_time.active_ms, 0);

        enc.clear_combat_stats();

        // モンスターは entities に残るがプレイヤーは除去されるため、モンスターで確認する。
        let entity = enc.entities.entry(EntityKey::monster(1)).or_default();
        assert_eq!(entity.active_dmg_time.active_ms, 0);
    }
}
