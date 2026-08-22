//! 食事(food)/シロップ(alchemy)バフの判定と、戦闘終了をまたいで残時間を保持する
//! 永続ストア。base_id 集合は ConsumableBuffIds.json を埋め込む（姉妹リポ
//! ../resonance-logs-cn の BuffName.json で Icon が `buff_food_up*`=食事 /
//! `buff_agentia_up*`=シロップ のものを抽出）。
//!
//! clear_combat_stats は buff_tracker を消すため、戦闘終了→新規戦闘で食事バフを
//! 忘れてしまう。ゲーム内では効果が継続するので、観測時に終了時刻を控えて
//! buff_tracker が消えても保持し、自然失効/履歴クリアで消す（手動リセットでは保持）。
//! expire_at_ms は壁時計(エポックms)基準なので、consumables.json へディスク永続化して
//! アプリ再起動後も残時間を復元する（load 時に失効分を除去）。

use crate::engine::buff_tracker::{expire_at_local_ms, BuffStateSnapshot, BuffTracker};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock, RwLock};

#[derive(serde::Deserialize)]
struct Ids {
    food: Vec<i32>,
    syrup: Vec<i32>,
}

static IDS: LazyLock<(HashSet<i32>, HashSet<i32>)> = LazyLock::new(|| {
    let data = include_str!("../../data/json/ConsumableBuffIds.json");
    let parsed: Ids = serde_json::from_str(data).expect("invalid ConsumableBuffIds.json");
    (
        parsed.food.into_iter().collect(),
        parsed.syrup.into_iter().collect(),
    )
});

/// base_id が食事/シロップのいずれかなら true（buff_tracker の観測ログ用）。
pub fn is_consumable(base_id: i32) -> bool {
    let (food_ids, syrup_ids) = &*IDS;
    food_ids.contains(&base_id) || syrup_ids.contains(&base_id)
}

/// 1バフの終了時刻・総時間（残量比率算出用）と種類解決用の base_id。
/// `buff_uuid`/`create_time`/`layer` は付与の同一性キー。受動再観測では expire を
/// 凍結し、別インスタンスの再付与（buff_uuid 変化＝再食）・同一インスタンスの
/// タイマーリフレッシュ（create_time 変化）・重ねがけ（layer 増）でのみ更新する。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timing {
    pub expire_at_ms: u128,
    pub duration_ms: u128,
    pub base_id: i32,
    pub buff_uuid: i32,
    pub create_time: i64,
    pub layer: i32,
    /// 観測時点の BuffStateSnapshot.server_clock_trusted の転写。tighten がサーバ時計
    /// 基準式を使ってよいかの判定に使う。旧バージョンが保存した JSON にはこのフィールドが
    /// 無いため #[serde(default)] で false にする＝tighten は常に受信基準フォールバック
    /// （now_ms+duration_ms、既存以上なので min で縮まない）＝従来どおり触らない挙動になる。
    #[serde(default)]
    pub trusted: bool,
}

impl Timing {
    pub fn remaining_ms(&self, now_ms: u128) -> i64 {
        (self.expire_at_ms as i128 - now_ms as i128) as i64
    }
}

/// プレイヤーの食事/シロップ状態。
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerConsumables {
    pub food: Option<Timing>,
    pub syrup: Option<Timing>,
}

/// buff_tracker の観測でストアを更新し、失効分を除去する。
/// buff_tracker に無い（戦闘終了で消えた）バフは保持し続け、now が終了時刻を
/// 過ぎたら除去する。
///
/// 残り時間はゲームが送る duration を尊重し、新規付与（create_time 変化）・
/// 重ねがけ（layer 増）でのみ更新する。受動的な BuffTick/Snapshot 再観測は
/// `received_at_local_ms` を再ベースするが、ここでは expire を凍結して
/// 残り時間が膨張・リセットしないようにする（ゲーム実値と食い違わせない）。
pub fn refresh(store: &mut HashMap<i64, PlayerConsumables>, tracker: &BuffTracker, now_ms: u128) {
    let (food_ids, syrup_ids) = &*IDS;
    for (uid, snaps) in tracker.snapshot_all(now_ms) {
        // 食事/シロップそれぞれ、終了時刻が最も遅い候補を代表（重ねがけ後の最新）とする。
        let mut food_cand: Option<&BuffStateSnapshot> = None;
        let mut syrup_cand: Option<&BuffStateSnapshot> = None;
        for s in &snaps {
            // 無期限（None）はタイマー対象外。None＝無期限の判定は expire_at_local_ms
            // 一箇所に統一する（duration_ms<=0 をここで再判定しない）。
            let Some(expire) = s.expire_at_local_ms else {
                continue;
            };
            if food_ids.contains(&s.base_id) && later_expire(expire, s.buff_uuid, food_cand) {
                food_cand = Some(s);
            }
            if syrup_ids.contains(&s.base_id) && later_expire(expire, s.buff_uuid, syrup_cand) {
                syrup_cand = Some(s);
            }
        }

        let existing = store.get(&uid).copied().unwrap_or_default();
        let food = merge(existing.food, food_cand, now_ms);
        let syrup = merge(existing.syrup, syrup_cand, now_ms);
        if food.is_some() || syrup.is_some() {
            let e = store.entry(uid).or_default();
            e.food = food;
            e.syrup = syrup;
        }
    }
    // オフセットが判明済みなら、起動直後にオフセット未知のまま受信基準で入った膨張値・
    // 旧バージョンが保存した膨張値を、候補の有無にかかわらず全エントリで補正する
    // （merge は同一 buff_uuid・据置 create_time を凍結するため、これが唯一の補正経路）。
    if let Some(offset) = tracker.server_clock_offset_ms() {
        tighten_all(store, offset, now_ms);
    }
    purge_expired(store, now_ms);
}

/// 全 store エントリの expire_at_ms を、判明したサーバ時計オフセットで下方修正する。
/// create_time が妥当なサーバ時刻でなければ何もしない（BuffTick 由来等 create_time=0 は対象外）。
fn tighten_all(store: &mut HashMap<i64, PlayerConsumables>, offset: i64, now_ms: u128) {
    for pc in store.values_mut() {
        tighten(&mut pc.food, offset, now_ms);
        tighten(&mut pc.syrup, offset, now_ms);
    }
}

/// tighten が更新に踏み切る最小の短縮幅。実機ログでオフセット推定値（min）が1msずつ
/// 下がるたびに `consumables: saved` が数十回連発していた（表示は秒粒度なので1msの
/// 改善は体感できず、JSON 書き込みとログだけが無駄に走る）。1秒未満の改善は据え置く。
const TIGHTEN_MIN_DELTA_MS: u128 = 1_000;

fn tighten(slot: &mut Option<Timing>, offset: i64, now_ms: u128) {
    let Some(t) = slot else {
        return;
    };
    // trusted は観測時点の BuffStateSnapshot.server_clock_trusted をそのまま転写した値
    // （t.trusted、fresh() 参照）。local には now_ms（refresh 呼び出し時点のローカル
    // 壁時計）を渡す。trusted=false／create_time が implausible／offset が使えない
    // 場合は expire_at_local_ms が受信基準フォールバック（now_ms+duration_ms）を返す。
    // この値は既存 expire_at_ms 以上になるため（duration は不変で、時間は進んでいる
    // だけ）、以降の min による下方修正では無害（意図せず縮めない）。
    let Some(corrected) = expire_at_local_ms(t.create_time, now_ms, t.duration_ms as i64, Some(offset), t.trusted) else {
        return; // duration_ms<=0 はここに来ない想定だが念のため
    };
    // 改善幅が TIGHTEN_MIN_DELTA_MS 未満なら据え置く（corrected が既存以上のとき
    // saturating_sub は 0 になり、同じ枝で自然にスキップされる）。
    if t.expire_at_ms.saturating_sub(corrected) >= TIGHTEN_MIN_DELTA_MS {
        t.expire_at_ms = corrected;
    }
}

/// now が終了時刻を過ぎた food/syrup を None にし、両方空になった uid を除去する。
fn purge_expired(store: &mut HashMap<i64, PlayerConsumables>, now_ms: u128) {
    for pc in store.values_mut() {
        if pc.food.is_some_and(|f| now_ms >= f.expire_at_ms) {
            pc.food = None;
        }
        if pc.syrup.is_some_and(|f| now_ms >= f.expire_at_ms) {
            pc.syrup = None;
        }
    }
    store.retain(|_, pc| pc.food.is_some() || pc.syrup.is_some());
}

/// `expire`（対象候補の期限。呼び出し元の refresh ループで無期限=None は除外済み）が
/// 現候補より遅ければ true。同値のときは `buff_uuid` が大きい方を決定的に採用する
/// （HashMap 順に依存すると代表 uuid が refresh のたびに入れ替わり、無意味なディスク
/// 書き込みが起きる。buff_uuid はシーン切替で振り直されるため「大きい＝新しい」では
/// ないが、目的は決定性の確保）。
fn later_expire(expire: u128, buff_uuid: i32, cand: Option<&BuffStateSnapshot>) -> bool {
    match cand.and_then(|c| c.expire_at_local_ms.map(|e| (e, c.buff_uuid))) {
        None => true,
        Some((cand_expire, cand_uuid)) => {
            expire > cand_expire || (expire == cand_expire && buff_uuid > cand_uuid)
        }
    }
}

/// 既存 Timing と観測候補から、更新後の Timing を決める。
/// - 候補なし: 既存を保持（戦闘クリアで buff_tracker が消えても凍結）。
/// - 既存なし: 観測値で初期化。
/// - 既存失効済み: 観測値で再付与扱い。
/// - 別インスタンスの再付与（buff_uuid 変化＝再食）/ 重ねがけ（layer 増）/
///   同一インスタンスのリフレッシュ（create_time が両者非0で変化）/
///   同一 buff_uuid・同一 create_time のまま総時間が延長（duration 増加＝再食を
///   サーバが duration の書き換えとして送る実装。例: 料理を25分経過(残5分)時点で
///   再食すると総時間が30分→35分へ延びる）: 観測値で更新。
/// - それ以外（受動再観測・同一 buff_uuid・create_time 据置/0・duration 据置/減少）:
///   既存 expire を凍結。
fn merge(existing: Option<Timing>, cand: Option<&BuffStateSnapshot>, now_ms: u128) -> Option<Timing> {
    let Some(s) = cand else {
        return existing;
    };
    // 無期限（None）は refresh 側の候補ループで既に除外済み。None＝無期限の判定は
    // expire_at_local_ms 一箇所に統一するため、ここでは防御的に既存を保持するのみ
    // （実際には到達しない想定）。
    let Some(s_expire) = s.expire_at_local_ms else {
        return existing;
    };
    let fresh = || Timing {
        expire_at_ms: s_expire,
        duration_ms: s.duration_ms as u128,
        base_id: s.base_id,
        buff_uuid: s.buff_uuid,
        create_time: s.create_time_server,
        layer: s.layer,
        trusted: s.server_clock_trusted,
    };
    let Some(e) = existing else {
        return Some(fresh());
    };
    if now_ms >= e.expire_at_ms {
        return Some(fresh()); // 既存は失効済み → 新規付与として採用
    }
    if s.buff_uuid != e.buff_uuid {
        // 別インスタンスの再付与（再食＝新規付与）。残時間の長短は比較せず、観測された
        // 現行インスタンスを信頼する。候補は refresh 側の later_expire が現スナップショット
        // 群から最遅 expire を選んでおり、より長い既存インスタンスが tracker に残っていれば
        // そちらが候補になる。新 uuid が候補に選ばれる＝旧インスタンスは既に tracker から
        // 消えている＝新 uuid が正規バフ、なので過大評価された旧 expire を実値へ補正する。
        // 伸長時のみ採用するガードは、古い expire に凍結したままグレーへ戻る不具合を
        // 別経路で再発させるため入れない。
        return Some(fresh());
    }
    if s.layer > e.layer {
        return Some(fresh()); // 重ねがけ（スタック増）
    }
    // ここに到達するのは buff_uuid が一致する場合のみ（再食は上で処理済み）。
    if s.create_time_server != 0 && e.create_time != 0 && s.create_time_server != e.create_time {
        return Some(fresh()); // 同一 buff_uuid のタイマーリフレッシュ（create_time 変化）
    }
    if s.duration_ms as u128 > e.duration_ms {
        // 同一付与の総時間延長＝再食。create_time 据置でも採用する（サーバが再食を
        // 新規 create_time ではなく同一 uuid・同一 create_time のまま duration の
        // 書き換えとして送る実装があるため）。expire はサーバ時計基準で計算済み
        // （trusted かつ offset 既知なら create_time+offset+新duration）なので、
        // 受信時刻を使う旧凍結ロジックのようには膨張しない。フォールバック
        // （trusted=false／offset 未知）で採用しても、offset 判明後は tighten が補正する。
        return Some(fresh());
    }
    Some(e) // 受動再観測 → 凍結
}

// ─── ディスク永続化 ─────────────────────────────────────────────────────────
//
// expire_at_ms は壁時計(エポックms)なので保存値は再起動後もそのまま有効。
// 起動時に load し、変化時のみ save_if_changed で書き戻す（selected_uid 同形）。

/// 保存ファイル構造（前方互換のため version 付き）。
#[derive(Serialize, Deserialize)]
struct ConsumablesFile {
    version: u32,
    /// player_uid -> 食事/シロップ状態。serde_json は i64 キーを文字列キーに直列化する。
    players: HashMap<i64, PlayerConsumables>,
}

const FILE_VERSION: u32 = 1;

struct PersistState {
    path: Option<PathBuf>,
    /// 直近に書き込んだ JSON。無変化時の再書き込みを避けるためのキャッシュ。
    last_json: Option<String>,
}

static PERSIST: OnceLock<RwLock<PersistState>> = OnceLock::new();

fn persist() -> &'static RwLock<PersistState> {
    PERSIST.get_or_init(|| {
        RwLock::new(PersistState {
            path: None,
            last_json: None,
        })
    })
}

/// 保存先パスを登録する（起動時に1回）。
pub fn init(path: PathBuf) {
    let Ok(mut g) = persist().write() else {
        warn!("consumables: ロック取得失敗 (init)");
        return;
    };
    g.path = Some(path);
}

/// 永続ファイルを読み込み、now で失効済みの分を除いて返す。
/// ファイル無し/パース失敗/未init は空 map（warn ログ）。
pub fn load(now_ms: u128) -> HashMap<i64, PlayerConsumables> {
    let path = {
        let Ok(g) = persist().read() else {
            return HashMap::new();
        };
        g.path.clone()
    };
    let Some(path) = path else {
        return HashMap::new();
    };
    let Ok(data) = std::fs::read_to_string(&path) else {
        info!("consumables: ファイルなし ({})、空で起動", path.display());
        return HashMap::new();
    };
    let parsed: ConsumablesFile = match serde_json::from_str(&data) {
        Ok(f) => f,
        Err(e) => {
            warn!("consumables: パース失敗 ({}): {e}、空で起動", path.display());
            return HashMap::new();
        }
    };
    let mut store = parsed.players;
    purge_expired(&mut store, now_ms); // 閉じている間に失効した分を除去
    info!("consumables: 読み込み完了 {} 件", store.len());
    store
}

/// store を直列化し、前回書き込み内容と異なる時のみファイルへ書き出す。
/// 食事/シロップは付与/失効でしか変わらないため実質ゼロ I/O。
pub fn save_if_changed(store: &HashMap<i64, PlayerConsumables>) {
    let file = ConsumablesFile {
        version: FILE_VERSION,
        players: store.clone(),
    };
    let json = match serde_json::to_string(&file) {
        Ok(j) => j,
        Err(e) => {
            warn!("consumables: シリアライズ失敗: {e}");
            return;
        }
    };
    let path = {
        let Ok(g) = persist().read() else {
            return;
        };
        match &g.path {
            Some(p) if needs_write(g.last_json.as_deref(), &json) => p.clone(),
            _ => return, // パス未設定 or 無変化
        }
    };
    if let Err(e) = write_file(&path, &json) {
        warn!("consumables: 保存失敗 ({}): {e}", path.display());
        return;
    }
    // 実機検証用: 状態が変わった時だけ全文を残す（付与/失効/延長/切替でしか変わらない）。
    info!("consumables: saved {json}");
    if let Ok(mut g) = persist().write() {
        g.last_json = Some(json);
    }
}

/// last_json と new_json が異なれば書き込みが必要。
fn needs_write(last_json: Option<&str>, new_json: &str) -> bool {
    last_json != Some(new_json)
}

fn write_file(path: &Path, json: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::buff_tracker::BuffTracker;
    use crate::protocol::pb;

    const FOOD_ID: i32 = 700083; // ConsumableBuffIds.json food[0]
    const FOOD_ID_B: i32 = 700084; // ConsumableBuffIds.json food[1]（別種の料理への切替テスト用）
    const SYRUP_ID: i32 = 681836; // ConsumableBuffIds.json syrup[0]
    const UID: i64 = 5000;

    fn buff_info(base_id: i32, duration: i32, create_time: i64, layer: i32) -> pb::BuffSnapshot {
        pb::BuffSnapshot {
            buff_uuid: 1,
            base_id,
            level: 1,
            host_uuid: 0,
            table_uuid: 0,
            create_time,
            fire_uuid: 0,
            layer,
            part_id: 0,
            count: 1,
            duration,
            fight_source_info: None,
        }
    }

    fn food_info(duration: i32, create_time: i64, layer: i32) -> pb::BuffSnapshot {
        buff_info(FOOD_ID, duration, create_time, layer)
    }

    // 受動 BuffTick（create_time=0 で received_at を再ベース）では expire を凍結する。
    #[test]
    fn passive_reobservation_does_not_rebase() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(1, &food_info(600_000, 1000, 1), 0, UID);
        refresh(&mut store, &tracker, 0);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(0), 600_000);

        // create_time=0 の受動 tick で now=100s に再ベース
        let tick = pb::BuffTick {
            host_uuid: (UID << 16) | 640,
            buff_uuid: 1,
            base_id: FOOD_ID,
            duration: 600_000,
            create_time: 0,
            layer: 1,
        };
        tracker.apply_change(&tick, 100_000);
        refresh(&mut store, &tracker, 100_000);
        // 凍結されていれば残 500s（再ベースされると 600s に膨張する）
        assert_eq!(store[&UID].food.unwrap().remaining_ms(100_000), 500_000);
    }

    // 重ねがけ（layer 増）で expire を更新する。
    // create_time はローカル時刻窓（±24h）の外に置く識別子（値に意味は無い。
    // サーバ時計オフセット学習の対象外にして受信基準の期待値をそのまま検証するため）。
    #[test]
    fn stacking_layer_increase_refreshes() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(1, &food_info(600_000, 100_000_000, 1), 0, UID);
        refresh(&mut store, &tracker, 0);

        let change = pb::BuffChange { layer: 2, duration: 600_000, create_time: 200_000_000 };
        tracker.apply_buff_change(UID, 1, &change, 100_000);
        refresh(&mut store, &tracker, 100_000);
        // 100s + 600s = 700s 終了 → 残 600s、layer=2
        assert_eq!(store[&UID].food.unwrap().remaining_ms(100_000), 600_000);
        assert_eq!(store[&UID].food.unwrap().layer, 2);
    }

    // 再食（別 buff_uuid の新規付与）は古い残時間に固まらず expire を延長する。
    // ＝ ボス戦リセット後に再食してもアイコンがグレーへ戻らない。
    // create_time はローカル時刻窓（±24h）の外に置く識別子（値に意味は無い。
    // サーバ時計オフセット学習の対象外にして受信基準の期待値をそのまま検証するため）。
    #[test]
    fn reeat_new_instance_extends() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        // 最初の食事: buff_uuid=1, 残 600s
        tracker.apply_buff_add(1, &food_info(600_000, 100_000_000, 1), 0, UID);
        refresh(&mut store, &tracker, 0);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(0), 600_000);

        // 300s 後に再食: 別インスタンス buff_uuid=2, 残 600s（古いインスタンスは残存）
        tracker.apply_buff_add(2, &food_info(600_000, 200_000_000, 1), 300_000, UID);
        refresh(&mut store, &tracker, 300_000);
        // 300s + 600s = 900s 終了 → 残 600s に延長され、新インスタンスが採用される
        assert_eq!(store[&UID].food.unwrap().remaining_ms(300_000), 600_000);
        assert_eq!(store[&UID].food.unwrap().buff_uuid, 2);
    }

    // create_time=0 の受動再食でも buff_uuid が変われば延長する（create_time に依存しない）。
    #[test]
    fn reeat_zero_create_time_still_extends_via_buff_uuid() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(1, &food_info(600_000, 0, 1), 0, UID);
        refresh(&mut store, &tracker, 0);

        tracker.apply_buff_add(2, &food_info(600_000, 0, 1), 300_000, UID);
        refresh(&mut store, &tracker, 300_000);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(300_000), 600_000);
    }

    // 片方（食事）のみ再食しても、もう片方（シロップ）の凍結残時間は影響を受けない。
    #[test]
    fn reeat_food_does_not_disturb_syrup() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(10, &food_info(600_000, 0, 1), 0, UID);
        tracker.apply_buff_add(20, &buff_info(SYRUP_ID, 600_000, 0, 1), 0, UID);
        refresh(&mut store, &tracker, 0);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(0), 600_000);
        assert_eq!(store[&UID].syrup.unwrap().remaining_ms(0), 600_000);

        // 300s 後に食事のみ再食（シロップは再観測されるが据え置き）
        tracker.apply_buff_add(11, &food_info(600_000, 0, 1), 300_000, UID);
        refresh(&mut store, &tracker, 300_000);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(300_000), 600_000); // 延長
        assert_eq!(store[&UID].syrup.unwrap().remaining_ms(300_000), 300_000); // 凍結維持
    }

    // buff_tracker.clear（戦闘終了）後も保持し、失効時に除去する。
    #[test]
    fn persists_across_clear_until_expiry() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(1, &food_info(600_000, 1000, 1), 0, UID);
        refresh(&mut store, &tracker, 0);

        tracker.clear(); // 戦闘終了で buff_tracker が空に
        refresh(&mut store, &tracker, 300_000);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(300_000), 300_000);

        refresh(&mut store, &tracker, 600_000); // 失効
        assert!(store.get(&UID).is_none());
    }

    fn timing(expire_at_ms: u128) -> Timing {
        Timing {
            expire_at_ms,
            duration_ms: 600_000,
            base_id: FOOD_ID,
            buff_uuid: 7,
            create_time: 1234,
            layer: 1,
            trusted: true,
        }
    }

    // serde ラウンドトリップで Timing（u128 含む）が保たれる。
    #[test]
    fn persist_roundtrip_preserves_timing() {
        let mut players = HashMap::new();
        players.insert(
            UID,
            PlayerConsumables {
                food: Some(timing(900_000)),
                syrup: None,
            },
        );
        let file = ConsumablesFile { version: FILE_VERSION, players };
        let json = serde_json::to_string(&file).unwrap();
        let back: ConsumablesFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, FILE_VERSION);
        assert_eq!(back.players[&UID].food, Some(timing(900_000)));
        assert_eq!(back.players[&UID].syrup, None);
    }

    // load 相当の purge: now を過ぎた food/syrup が除去され、空 uid が消える。
    #[test]
    fn purge_drops_expired_keeps_live() {
        let mut store = HashMap::new();
        store.insert(
            UID,
            PlayerConsumables {
                food: Some(timing(100_000)),  // 失効
                syrup: Some(timing(900_000)), // 生存
            },
        );
        store.insert(
            UID + 1,
            PlayerConsumables {
                food: Some(timing(50_000)), // 失効のみ → uid ごと消える
                syrup: None,
            },
        );
        purge_expired(&mut store, 500_000);
        assert_eq!(store[&UID].food, None);
        assert_eq!(store[&UID].syrup, Some(timing(900_000)));
        assert!(!store.contains_key(&(UID + 1)));
    }

    // 無変化なら書き込み不要、差分があれば必要。
    #[test]
    fn needs_write_detects_change() {
        assert!(needs_write(None, "x"));
        assert!(!needs_write(Some("x"), "x"));
        assert!(needs_write(Some("x"), "y"));
    }

    // ─── サーバ時計オフセット反映（マップ移動での残時間膨張バグの修正・実データ再現） ───
    // 2026-08-22 実測: create_time=1787368015470(12:06:55) duration_ms=2340005(39分)の
    // バフが受信12:29:32（付与から22.6分後）に expire=13:08:32 として誤記録された実例を再現する。

    const REAL_T0: i64 = 1_787_368_015_470; // 12:06:55 相当
    const REAL_DURATION: i32 = 2_340_005; // 39分

    // オフセット既知（即時受信）での付与は create_time+offset+duration で expire する。
    #[test]
    fn offset_known_grant_uses_server_time_expire() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        // now_ms == create_time の即時受信としてオフセット0を確定させる
        tracker.apply_buff_add(3, &food_info(REAL_DURATION, REAL_T0, 1), REAL_T0 as u128, UID);
        refresh(&mut store, &tracker, REAL_T0 as u128);

        let expected_expire = REAL_T0 as u128 + REAL_DURATION as u128;
        assert_eq!(store[&UID].food.unwrap().expire_at_ms, expected_expire);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(REAL_T0 as u128), REAL_DURATION as i64);
    }

    // マップ移動由来の再送（別 buff_uuid・同一 create_time・同一 duration）は
    // 受信が22.6分後でも expire を膨張させない（13:08:32相当へ巻き戻らない）。
    #[test]
    fn map_move_resend_does_not_inflate_expire() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(3, &food_info(REAL_DURATION, REAL_T0, 1), REAL_T0 as u128, UID);
        refresh(&mut store, &tracker, REAL_T0 as u128);

        const ELAPSED_MS: i64 = 1_357_000; // 12:06:55 → 12:29:32 相当
        let now = (REAL_T0 + ELAPSED_MS) as u128;
        tracker.apply_buff_add(8, &food_info(REAL_DURATION, REAL_T0, 1), now, UID);
        refresh(&mut store, &tracker, now);

        // 旧実装(受信基準)なら now+DURATION(=13:08:32相当)へ膨張する。修正後は T0+DURATIONのまま。
        // uuid 3/8 は期限が同値で later_expire はどちらも候補になりうる（HashMap 順）ため、
        // buff_uuid は断定しない。不変条件は期限のみ。
        let expected_expire = REAL_T0 as u128 + REAL_DURATION as u128;
        assert_eq!(store[&UID].food.unwrap().expire_at_ms, expected_expire);
    }

    // 戦闘終了で tracker がクリアされた後のマップ移動再送（新 uuid が必ず候補になり
    // merge の「別インスタンス＝fresh()」経路を通る）でも expire は膨張しない。
    #[test]
    fn map_move_resend_after_tracker_clear_does_not_inflate_expire() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(3, &food_info(REAL_DURATION, REAL_T0, 1), REAL_T0 as u128, UID);
        refresh(&mut store, &tracker, REAL_T0 as u128);
        tracker.clear(); // 戦闘終了（オフセットは保持される）

        const ELAPSED_MS: i64 = 1_357_000;
        let now = (REAL_T0 + ELAPSED_MS) as u128;
        tracker.apply_buff_add(8, &food_info(REAL_DURATION, REAL_T0, 1), now, UID);
        refresh(&mut store, &tracker, now);

        let food = store[&UID].food.unwrap();
        assert_eq!(food.buff_uuid, 8); // 新インスタンスとして採用される
        assert_eq!(food.expire_at_ms, REAL_T0 as u128 + REAL_DURATION as u128);
    }

    // 起動直後シナリオ: tracker のオフセットが未知のまま resend だけが来ると、
    // 一旦は受信基準で膨張した expire が入る → その後に別バフの新規付与でオフセットが
    // 確定すると、次の refresh で create_time+offset+duration へ引き締まる。
    #[test]
    fn offset_unknown_then_learned_tightens_existing_entry() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();

        // 起動直後: オフセット未知のまま、マップ移動由来の resend だけが来る
        // （create_time=REAL_T0 は過去、受信は22.6分後）
        const ELAPSED_MS: i64 = 1_357_000;
        let now1 = (REAL_T0 + ELAPSED_MS) as u128;
        tracker.apply_buff_add(8, &food_info(REAL_DURATION, REAL_T0, 1), now1, UID);
        refresh(&mut store, &tracker, now1);
        // オフセット未知のため受信基準フォールバック→ now1+DURATION に膨張
        assert_eq!(store[&UID].food.unwrap().expire_at_ms, now1 + REAL_DURATION as u128);

        // その後、無関係な別バフ（シロップ枠）の新規付与がほぼ即時受信され、オフセットが確定する
        let now2 = now1 + 1000;
        tracker.apply_buff_add(20, &buff_info(SYRUP_ID, 30_000, now2 as i64, 1), now2, UID);
        assert_eq!(tracker.server_clock_offset_ms(), Some(0));

        // 次の refresh で食事枠の expire が引き締まる
        refresh(&mut store, &tracker, now2);
        assert_eq!(store[&UID].food.unwrap().expire_at_ms, REAL_T0 as u128 + REAL_DURATION as u128);
    }

    // ディスク復元相当: store に膨張した Timing を直接入れ、tracker にオフセットだけ既知の
    // 状態で refresh すると引き締まる（tracker に対応するバフが無くても効く＝候補の有無を問わない）。
    #[test]
    fn disk_restored_inflated_timing_tightens_once_offset_known() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();

        let inflated = Timing {
            expire_at_ms: (REAL_T0 + 5_000_000) as u128, // 旧バージョンが保存した膨張値を模す
            duration_ms: REAL_DURATION as u128,
            base_id: FOOD_ID,
            buff_uuid: 3,
            create_time: REAL_T0,
            layer: 1,
            trusted: true, // ディスク復元時点で明示（食事/シロップは常にトラステッドな経路で観測される）
        };
        store.insert(UID, PlayerConsumables { food: Some(inflated), syrup: None });

        // tracker には対応するバフが無い（戦闘終了で消えた等）が、オフセットだけ既知
        tracker.observe_server_time(REAL_T0, REAL_T0 as u128); // offset=0 を直接確定
        assert_eq!(tracker.server_clock_offset_ms(), Some(0));

        refresh(&mut store, &tracker, REAL_T0 as u128);
        assert_eq!(store[&UID].food.unwrap().expire_at_ms, REAL_T0 as u128 + REAL_DURATION as u128);
    }

    // 改善幅が TIGHTEN_MIN_DELTA_MS(1秒) 未満なら tighten は据え置く（実機ログで
    // オフセット推定値が1msずつ動くたびに保存/ログが連発していた問題への対処）。
    #[test]
    fn tighten_ignores_sub_threshold_improvement() {
        const CREATE_TIME: i64 = 1_700_000_000_000;
        const DURATION_MS: u128 = 5_000;
        // 正しい値(offset=0)は CREATE_TIME+0+DURATION_MS = 1_700_000_005_000。
        // 既存はそれより 500ms(<1000ms) だけ大きい膨張値にしておく。
        let mut slot = Some(Timing {
            expire_at_ms: 1_700_000_005_500,
            duration_ms: DURATION_MS,
            base_id: FOOD_ID,
            buff_uuid: 1,
            create_time: CREATE_TIME,
            layer: 1,
            trusted: true,
        });

        tighten(&mut slot, 0, CREATE_TIME as u128 + 100);

        assert_eq!(
            slot.unwrap().expire_at_ms,
            1_700_000_005_500,
            "改善幅500ms(<1000ms)は据え置かれるはず"
        );
    }

    // 同一 buff_uuid のまま create_time が変化（タイマーリフレッシュ）した場合も従来どおり新値採用。
    // create_time はローカル時刻窓（±24h）の外に置く識別子（値に意味は無い。
    // サーバ時計オフセット学習の対象外にして受信基準の期待値をそのまま検証するため）。
    #[test]
    fn timer_refresh_same_uuid_new_create_time_extends() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(1, &food_info(600_000, 100_000_000, 1), 0, UID);
        refresh(&mut store, &tracker, 0);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(0), 600_000);

        // 同一 buff_uuid のまま create_time が変化（タイマーリフレッシュ）
        let change = pb::BuffChange { layer: 1, duration: 600_000, create_time: 200_000_000 };
        tracker.apply_buff_change(UID, 1, &change, 300_000);
        refresh(&mut store, &tracker, 300_000);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(300_000), 600_000);
        assert_eq!(store[&UID].food.unwrap().create_time, 200_000_000);
    }

    // later_expire の同値タイは buff_uuid が大きい方を決定的に採用する。
    // HashMap の反復順に依存すると、期限が同値の複数インスタンスがある場合に代表
    // uuid が refresh のたびに入れ替わり、無意味なディスク書き込みが起きる。
    #[test]
    fn later_expire_tie_picks_larger_buff_uuid_deterministically() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();

        // 同一 create_time・同一 duration の2インスタンス(uuid 3/8)を両方 tracker に保持させる
        // （期限が同値のタイになる）。
        tracker.apply_buff_add(3, &food_info(REAL_DURATION, REAL_T0, 1), REAL_T0 as u128, UID);
        tracker.apply_buff_add(8, &food_info(REAL_DURATION, REAL_T0, 1), REAL_T0 as u128, UID);

        refresh(&mut store, &tracker, REAL_T0 as u128);
        let first = store[&UID].food.unwrap().buff_uuid;
        refresh(&mut store, &tracker, REAL_T0 as u128);
        let second = store[&UID].food.unwrap().buff_uuid;

        assert_eq!(first, 8, "同値タイは buff_uuid が大きい方を採用するはず");
        assert_eq!(second, 8, "2回目の refresh でも同じ代表 uuid になるはず（決定性）");
    }

    // ─── 再食の総時間延長（同一 uuid・同一 create_time のまま duration が増える場合） ───
    // ゲーム仕様: 料理A(30分)を25分経過(残5分)時点で再食すると残り35分になる
    // （A→A は総時間が延長される）。シロップも同様。

    // 同一 uuid・同一 create_time のまま duration が増加（再食をサーバが duration の
    // 書き換えとして送る実装）した場合、consumables 側も追従して期限を延ばす
    // （凍結し続けない）。
    #[test]
    fn reeat_same_grant_extends_total_duration() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();

        const T0: i64 = 1_700_000_000_000;
        const DURATION_A: i32 = 1_800_000; // 30分
        const DURATION_EXTENDED: i64 = 3_600_000; // 再食後の総時間(60分)

        // T0: 付与。即時受信としてオフセット0を確定させる（オフセット既知の状態を作る）。
        tracker.apply_buff_add(1, &food_info(DURATION_A, T0, 1), T0 as u128, UID);
        refresh(&mut store, &tracker, T0 as u128);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(T0 as u128), DURATION_A as i64);

        // T0+25分: 再食が同一 uuid・同一 create_time のまま duration 延長として届く
        const TWENTY_FIVE_MIN_MS: i64 = 25 * 60 * 1000;
        let now = (T0 + TWENTY_FIVE_MIN_MS) as u128;
        let change = pb::BuffChange { layer: 1, duration: DURATION_EXTENDED, create_time: T0 };
        tracker.apply_buff_change(UID, 1, &change, now);
        refresh(&mut store, &tracker, now);

        // 残り時間は 35分（総時間60分 - 経過25分）
        const THIRTY_FIVE_MIN_MS: i64 = 35 * 60 * 1000;
        assert_eq!(store[&UID].food.unwrap().remaining_ms(now), THIRTY_FIVE_MIN_MS);
    }

    // シロップ版: 同一 uuid・同一 create_time のまま duration 増加で期限が延びる。
    #[test]
    fn reeat_syrup_same_grant_extends_total_duration() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();

        const T0: i64 = 1_700_000_000_000;
        const DURATION_A: i32 = 600_000; // 10分
        const DURATION_EXTENDED: i64 = 1_200_000; // 再食後の総時間(20分)

        tracker.apply_buff_add(1, &buff_info(SYRUP_ID, DURATION_A, T0, 1), T0 as u128, UID);
        refresh(&mut store, &tracker, T0 as u128);
        assert_eq!(store[&UID].syrup.unwrap().remaining_ms(T0 as u128), DURATION_A as i64);

        // T0+5分（半分経過）時点で再食
        const FIVE_MIN_MS: i64 = 5 * 60 * 1000;
        let now = (T0 + FIVE_MIN_MS) as u128;
        let change = pb::BuffChange { layer: 1, duration: DURATION_EXTENDED, create_time: T0 };
        tracker.apply_buff_change(UID, 1, &change, now);
        refresh(&mut store, &tracker, now);

        // 残り時間は 15分（総時間20分 - 経過5分）
        const FIFTEEN_MIN_MS: i64 = 15 * 60 * 1000;
        assert_eq!(store[&UID].syrup.unwrap().remaining_ms(now), FIFTEEN_MIN_MS);
    }

    // 別種の料理へ切り替え（別 base_id・別 buff_uuid）。旧インスタンスを明示的に
    // remove した場合は新インスタンスが唯一の候補になり、そのまま採用される。
    #[test]
    fn switch_to_other_food_replaces_remaining() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();

        const T0: i64 = 1_700_000_000_000;
        const DURATION_A: i32 = 1_800_000; // 30分

        tracker.apply_buff_add(1, &food_info(DURATION_A, T0, 1), T0 as u128, UID);
        refresh(&mut store, &tracker, T0 as u128);

        const TWENTY_FIVE_MIN_MS: i64 = 25 * 60 * 1000;
        let now = (T0 + TWENTY_FIVE_MIN_MS) as u128;
        tracker.remove(UID, 1);
        tracker.apply_buff_add(2, &buff_info(FOOD_ID_B, DURATION_A, T0 + TWENTY_FIVE_MIN_MS, 1), now, UID);
        refresh(&mut store, &tracker, now);

        let food = store[&UID].food.unwrap();
        assert_eq!(food.base_id, FOOD_ID_B);
        assert_eq!(food.remaining_ms(now), DURATION_A as i64);
    }

    // 別種の料理へ切り替えた際、旧インスタンス（A）を remove しなくても、期限が
    // 遅い方（B、新たに30分の満タンで付与されたばかり）が代表として採用される。
    #[test]
    fn switch_to_other_food_without_remove_still_prefers_later_expire() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();

        const T0: i64 = 1_700_000_000_000;
        const DURATION_A: i32 = 1_800_000; // 30分

        tracker.apply_buff_add(1, &food_info(DURATION_A, T0, 1), T0 as u128, UID);
        refresh(&mut store, &tracker, T0 as u128);

        const TWENTY_FIVE_MIN_MS: i64 = 25 * 60 * 1000;
        let now = (T0 + TWENTY_FIVE_MIN_MS) as u128;
        // remove を呼ばない: A は tracker に残ったまま
        tracker.apply_buff_add(2, &buff_info(FOOD_ID_B, DURATION_A, T0 + TWENTY_FIVE_MIN_MS, 1), now, UID);
        refresh(&mut store, &tracker, now);

        let food = store[&UID].food.unwrap();
        assert_eq!(food.base_id, FOOD_ID_B, "期限が遅い方(B)が代表になるはず");
        assert_eq!(food.remaining_ms(now), DURATION_A as i64);
    }
}
