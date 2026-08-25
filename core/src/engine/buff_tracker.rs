use crate::protocol::constants::entity;
use crate::protocol::pb;
use crate::protocol::pb::EntityKind;
use std::collections::HashMap;

#[derive(Clone, Default, Debug)]
pub struct BuffTracker {
    // player_uid (uuid >> 16) -> buff_uuid -> state
    buffs: HashMap<i64, HashMap<i32, BuffState>>,
    /// ローカル壁時計とサーバ create_time の差分（オフセット）推定器。
    server_clock_offset: ServerClockOffsetEstimator,
}

/// サーバ時計オフセット推定のバケット幅。ヒューリスティック(min)・SyncServerTime
/// 標本(max)の両方で共有する（BucketedExtremum 参照）。
///
/// 根拠: 実測されたマップ移動 resend の間隔（2026-07-25 probe: 付与から観測まで最大
/// 22.6分の空白）を「2バケット以上の空白＝汚染候補として弾かれる」対象にしてしまうと、
/// この機能が直そうとしている不具合そのものが再発する（単発の古い resend の候補を
/// 誤って新オフセットとして採用してしまう）。そのため単一バケット幅を実測最大空白より
/// 十分大きい 30 分とし、22.6分程度の空白は常に「1バケット分のスライド」に収まる
/// （＝ prev に旧オフセットが残り fold で保護される）ようにする。2バケット(60分)の
/// 完全な空白が続いた場合のみ汚染候補・時計ジャンプからの再学習が働く。
const SERVER_CLOCK_OFFSET_BUCKET_MS: u128 = 30 * 60 * 1000; // 30分

/// SyncServerTime(0x2B) 標本の max 推定が有効とみなされる最大経過時間。
/// 0x2B はダンジョン1周で62回・数秒〜十数秒間隔で届く高頻度パケット（実測
/// docs-private/protocol-map-transition-data.md）のため、これだけ間隔が空くのは
/// 同期そのものが止まっている（スリープ復帰・接続断等）と判断してよい保険。
const SYNC_MAX_AGE_MS: u128 = 60 * 60 * 1000; // 60分

/// SyncServerTime(0x2B) 標本の max 推定に使うバケット幅。0x2B は約 4〜5 秒間隔で届く
/// （2026-08-24 実測 576 標本/36 分）ため、1 バケットに数十標本が入り max は十分安定する。
/// ヒューリスティック(30分)と同じ幅にすると、ローカル時計が 1 バケット未満の幅で後方へ
/// ステップした場合（w32time の補正等）に、ステップ前の「大きすぎる」標本が cur→prev と
/// 残り続け最長 60 分間バフの残時間を膨張させる。5 分なら回復は最長 2 バケット＝10 分。
/// 2 バケット(10分)以上の空白は 0x2B 自体が止まっている（接続断・スリープ）ときだけ。
const SYNC_BUCKET_MS: u128 = 5 * 60 * 1000; // 5分

/// バケット制で「今バケット・前バケット」2本の極値（min または max、keep_max で選択）を
/// 保持する状態機械。server_clock_offset のヒューリスティック推定(min)と
/// SyncServerTime 標本推定(max)の両方で同じ状態遷移を共有するために一般化した
/// （同じ状態遷移を2回書かない）。
///
/// 単調な極値追跡（セッション全体で1個だけ持ち続ける）だと、ローカル時計の前方
/// ジャンプや、たまたま観測した極端な候補による誤学習が永続化し、以後すべての
/// バフが即失効表示になりかねない（must-fix）。バケット制にすることで、汚染された
/// 候補は最長 SERVER_CLOCK_OFFSET_BUCKET_MS*2 で自然に cur/prev から抜け、
/// 逆方向の候補にも再学習できる。
///
/// 2バケット以上の空白（リセット分岐）では、値を単純に捨てるのではなく現在の
/// `value()`（＝リセット直前の有効な推定値）を `prev` へ繰り越す。狙い: アイドル明け
/// 最初の観測が外れ値（マップ移動 resend 等）でも、旧推定が prev に残っているため
/// fold で守られる。戦闘が始まれば新規付与が cur を正しい値で上書きする。ローカル
/// 時計が前へジャンプした後は、繰り越した旧推定が次のバケット境界を跨いだ時点で
/// prev から自然に抜けるため、最長1バケット（SERVER_CLOCK_OFFSET_BUCKET_MS）で
/// 逆方向に再学習できる。時計の後退（bucket が現在より前に戻る）も同じ分岐で
/// 扱ってよい（挙動として害はなく単純）。
#[derive(Clone, Debug)]
struct BucketedExtremum {
    /// true なら大きい方を残す(max)。false なら小さい方を残す(min)。
    keep_max: bool,
    /// バケット幅（ms）。観測頻度に応じて推定器ごとに選ぶ（heuristic_min=30分、sync_max=5分）。
    bucket_ms: u128,
    /// 現在バケットの開始時刻（バケット幅で切り捨てたエポックms）。未観測なら None。
    bucket_start_ms: Option<u128>,
    cur: Option<i64>,
    prev: Option<i64>,
    /// 直近の observe() 呼び出し時刻（バケット境界に丸めない生の値）。
    /// SyncServerTime の SYNC_MAX_AGE_MS のような経過時間ベースの失効判定に使う。
    last_observed_at_ms: Option<u128>,
}

impl BucketedExtremum {
    fn new(keep_max: bool, bucket_ms: u128) -> Self {
        Self {
            keep_max,
            bucket_ms,
            bucket_start_ms: None,
            cur: None,
            prev: None,
            last_observed_at_ms: None,
        }
    }

    /// keep_max に応じて a/b のうち残す方を選ぶ。
    fn fold(&self, a: i64, b: i64) -> i64 {
        if self.keep_max { a.max(b) } else { a.min(b) }
    }

    /// 新しい候補を観測する。now_ms がバケット境界をちょうど1つ越えていれば
    /// cur→prev へスライドする。2バケット以上の空白（長時間放置後の再開・時計後退等）
    /// なら、リセット直前の value() を prev へ繰り越してから cur をリセットする。
    fn observe(&mut self, candidate: i64, now_ms: u128) {
        let bucket = (now_ms / self.bucket_ms) * self.bucket_ms;
        match self.bucket_start_ms {
            None => self.bucket_start_ms = Some(bucket),
            Some(cur) if cur == bucket => {}
            Some(cur) if bucket == cur + self.bucket_ms => {
                // ちょうど1バケット進んだ: cur を prev へスライドする（2バケットを
                // 超える古い情報はここで自然に失効させる＝バケット窓を固定2本に保つ）。
                self.prev = self.cur.take();
                self.bucket_start_ms = Some(bucket);
            }
            Some(_) => {
                // 2バケット以上の空白、またはローカル時計の後退。
                // 旧推定(value())を prev へ繰り越してから cur をリセットする。
                self.prev = self.value();
                self.cur = None;
                self.bucket_start_ms = Some(bucket);
            }
        }
        self.cur = Some(match self.cur {
            None => candidate,
            Some(existing) => self.fold(existing, candidate),
        });
        self.last_observed_at_ms = Some(now_ms);
    }

    /// cur/prev のうち Some 側の fold 結果。両方 None（未観測）なら None。
    fn value(&self) -> Option<i64> {
        match (self.cur, self.prev) {
            (Some(a), Some(b)) => Some(self.fold(a, b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        }
    }
}

/// ローカル壁時計とサーバ時計の差分を、2系統の観測から推定する。
/// - heuristic_min: create_time ヒューリスティック。`now-create_time = 時計差+片道遅延`
///   なので、遅延が大きいほど値が「大きく」なる→ min が時計差の最良近似。
/// - sync_max: SyncServerTime(0x2B) 標本。`client-server = 時計差-片道遅延`
///   （client=クライアント送信時刻、server=サーバ受信時刻）なので、遅延が大きいほど
///   値が「小さく」なる→ max が時計差の最良近似。実測（2026-08-23 probe）で両者は
///   真値を挟むことを確認済み（sync の max ≤ 真値 ≤ heuristic の min）。
///   sync_max が既知（かつ新しい）なら最優先で使う。0x2B が来ない環境・デモ・
///   起動直後は heuristic_min へフォールバックする。
#[derive(Clone, Debug)]
struct ServerClockOffsetEstimator {
    heuristic_min: BucketedExtremum,
    sync_max: BucketedExtremum,
}

impl Default for ServerClockOffsetEstimator {
    fn default() -> Self {
        Self {
            heuristic_min: BucketedExtremum::new(false, SERVER_CLOCK_OFFSET_BUCKET_MS),
            sync_max: BucketedExtremum::new(true, SYNC_BUCKET_MS),
        }
    }
}

impl ServerClockOffsetEstimator {
    fn observe(&mut self, candidate: i64, now_ms: u128) {
        self.heuristic_min.observe(candidate, now_ms);
    }

    /// SyncServerTime(0x2B) から得た標本（client_ms - server_ms）を記録する。
    fn observe_sync(&mut self, candidate: i64, now_ms: u128) {
        self.sync_max.observe(candidate, now_ms);
    }

    /// 現在有効なオフセット推定値。sync_max が既知かつ最終観測から SYNC_MAX_AGE_MS
    /// 以内ならそれを最優先で返す（遅延は常に値を下げる方向に働くため、max が
    /// 時計差の最良近似）。古すぎる／未観測ならヒューリスティック(heuristic_min)へ戻る。
    fn offset(&self, now_ms: u128) -> Option<i64> {
        if let (Some(v), Some(last)) = (self.sync_max.value(), self.sync_max.last_observed_at_ms) {
            if now_ms.saturating_sub(last) < SYNC_MAX_AGE_MS {
                return Some(v);
            }
        }
        self.heuristic_min.value()
    }
}

/// オフセット推定値の変化を info ログに出す閾値判定（初回の確定、または 1 秒以上の変化）。
/// observe_server_time / observe_server_time_sync で同じ基準を共有する。
fn offset_changed_enough(before: Option<i64>, after: Option<i64>) -> bool {
    match (before, after) {
        (Some(b), Some(a)) => (a - b).abs() >= 1000,
        (None, Some(_)) => true,
        _ => false,
    }
}

/// サーバ時刻の妥当性判定に使う許容ズレ幅（ローカル時刻との差の上限）。
/// 24時間を閾値にした根拠: 食事/シロップ等バフの寿命は長くても数時間以内に収まるため
/// 実用上は十分な幅。PC 時計がサーバと1日以上ズレている環境は稀で、そうした環境では
/// フォールバック（受信基準）計算に倒れても実害は小さい。一方、別単位（例: マイクロ秒/
/// ナノ秒相当）で来た値はこの窓から確実に外れるため誤採用を防げる。
pub const MAX_SERVER_TIME_SKEW_MS: i128 = 86_400_000; // 24時間

/// create_time がエポックms として妥当なサーバ時刻か判定する唯一の定義。
/// ローカル時刻(local_ms)との差が MAX_SERVER_TIME_SKEW_MS 以内かで判定する。
/// BuffTick の create_time=0 や TimedEffect.activated_at が別単位で来た場合に
/// 誤ってオフセット/期限計算へ使わないためのガード。consumables.rs からも参照する
/// （同じ判定を2箇所に書かない）。
pub fn is_plausible_server_time(t: i64, local_ms: u128) -> bool {
    t > 0 && (local_ms as i128 - t as i128).abs() < MAX_SERVER_TIME_SKEW_MS
}

/// 通知の create_time が非0（＝信頼できる付与時刻）のときだけ保持し trusted へ引き上げる。
/// create_time=0 の tick で無条件に true にすると、apply_effect 由来のエントリと id が
/// 衝突した場合に未検証の activated_at がサーバ時計基準式へ流れる。また 0 で上書きすると
/// AddBuff/Snapshot で得た正規の付与時刻が消え、再食判別ができず残時間が凍結する。
/// apply_change（BuffTick）と apply_buff_change（BuffChange）で同じ規約を共有する。
fn promote_trusted_create_time(state: &mut BuffState, create_time: i64) {
    if create_time != 0 {
        state.create_time_server = create_time;
        state.server_clock_trusted = true;
    }
}

/// バフの期限（ローカル壁時計のエポックms）を計算する唯一の場所。
/// - duration_ms <= 0 は無期限で None。
/// - trusted（付与元が BuffSnapshot/BuffChange 系＝create_time の意味が実測確定して
///   いる）かつ create_time_server が妥当なサーバ時刻で offset が既知なら
///   `create_time_server + offset + duration_ms`（サーバ時計基準。マップ移動での
///   resend に対して膨張しない）。
/// - それ以外（trusted=false、offset 未知、create_time 不明瞭）は
///   `received_at_local_ms + duration_ms`（従来どおりの受信基準フォールバック。
///   trusted かつ offset が既知になり次第、以降の呼び出しで自動的に補正される）。
///   trusted=false は TimedEffect.activated_at 由来（意味が実測未確認のため常にこちら）。
pub fn expire_at_local_ms(
    create_time_server: i64,
    received_at_local_ms: u128,
    duration_ms: i64,
    offset: Option<i64>,
    trusted: bool,
) -> Option<u128> {
    if duration_ms <= 0 {
        return None;
    }
    if trusted {
        if let Some(offset) = offset {
            if is_plausible_server_time(create_time_server, received_at_local_ms) {
                let expire = create_time_server as i128 + offset as i128 + duration_ms as i128;
                if expire < 0 {
                    warn_negative_expire_once(create_time_server, offset, duration_ms);
                }
                return Some(expire.max(0) as u128);
            }
        }
    }
    Some(received_at_local_ms + duration_ms as u128)
}

/// expire_at_local_ms が負値（0へクランプ）に到達したことを1回だけ警告する。
/// gc/snapshot は毎 poll 走るためスパム防止に AtomicBool で単発化する。
static NEGATIVE_EXPIRE_WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn warn_negative_expire_once(create_time_server: i64, offset: i64, duration_ms: i64) {
    use std::sync::atomic::Ordering;
    if !NEGATIVE_EXPIRE_WARNED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "buff_tracker: expire_at_local_ms が負値のため0へクランプ \
             (create_time={create_time_server}, offset={offset}, duration_ms={duration_ms})。\
             以後は同種の警告を抑止する。"
        );
    }
}

/// gc/make_snapshots 用の薄いラッパ。BuffState から expire_at_local_ms の引数を
/// 正しい順序・意味で組み立てる（引数順の取り違え防止）。
fn expire_of(state: &BuffState, offset: Option<i64>) -> Option<u128> {
    expire_at_local_ms(
        state.create_time_server,
        state.received_at_local_ms,
        state.duration_ms,
        offset,
        state.server_clock_trusted,
    )
}

#[derive(Clone, Debug)]
pub struct BuffState {
    pub buff_uuid: i32,
    pub base_id: i32,
    pub host_uuid: i64,
    pub fire_uuid: i64,
    pub create_time_server: i64,
    pub received_at_local_ms: u128,
    pub duration_ms: i64,
    pub layer: i32,
    pub count: i32,
    pub source_config_id: i32,
    /// create_time_server の出自が BuffSnapshot/BuffChange 系（意味が実測確定済み）なら
    /// true。TimedEffect.activated_at 由来（意味が実測未確認）なら false。
    /// false のときは expire_at_local_ms が常に受信基準フォールバックを使う。
    pub server_clock_trusted: bool,
}

#[derive(Clone)]
pub struct BuffStateSnapshot {
    pub buff_uuid: i32,
    pub base_id: i32,
    pub fire_uuid: i64,
    pub received_at_local_ms: u128,
    pub duration_ms: i64,
    pub remaining_ms: i64,
    pub layer: i32,
    pub count: i32,
    /// サーバ付与時刻。付与の同一性キー（新規付与・重ねがけの判別）に使う。
    /// BuffTick 経由では 0 で来ることがある点に注意。
    pub create_time_server: i64,
    /// ローカル壁時計基準の期限（エポックms）。無期限は None。
    /// サーバ時計オフセットが既知なら create_time_server+offset+duration、
    /// 未知なら received_at_local_ms+duration のフォールバック値
    /// （expire_at_local_ms 参照）。
    pub expire_at_local_ms: Option<u128>,
    /// BuffState.server_clock_trusted の転写。consumables.rs の Timing へも引き継がれ、
    /// tighten がサーバ時計基準式を使ってよいかの判定に使う。
    pub server_clock_trusted: bool,
}

impl BuffTracker {
    pub fn new() -> Self {
        Self {
            buffs: HashMap::new(),
            server_clock_offset: ServerClockOffsetEstimator::default(),
        }
    }

    /// サーバ時計とローカル壁時計の差分候補を観測し、バケット制の推定器へ渡す
    /// （create_time が妥当なサーバ時刻でなければ無視）。有効値が1秒以上動いた
    /// ときだけ info ログを出す（毎 tick 出さない）。
    ///
    /// 呼び出し元は create_time の意味が実測確定している経路のみに限定する:
    /// pb::BuffSnapshot.create_time（apply_full_info/apply_buff_add）・
    /// pb::BuffTick.create_time（apply_change。BuffSnapshot と同一 opcode/payload 由来）・
    /// pb::BuffChange.create_time（apply_buff_change。scene-add 再送の probe 実測で
    /// duration が総時間として不変と確認済み）。TimedEffect.activated_at は
    /// エポックms でも「付与時刻」の意味である実測証拠が無い（失効時刻なら期限が
    /// duration ぶん遅れる）ため、意図的にここを呼ばない（apply_effect 参照）。
    pub fn observe_server_time(&mut self, create_time: i64, now_ms: u128) {
        if !is_plausible_server_time(create_time, now_ms) {
            return;
        }
        let candidate = now_ms as i64 - create_time;
        let before = self.server_clock_offset.offset(now_ms);
        self.server_clock_offset.observe(candidate, now_ms);
        let after = self.server_clock_offset.offset(now_ms);
        if offset_changed_enough(before, after) {
            log::info!("buff_tracker: server_clock_offset_ms {before:?}ms -> {after:?}ms");
        }
    }

    /// SyncServerTime(0x2B) から得た標本（client_milliseconds - server_milliseconds、
    /// ゲームクライアント＝当アプリと同一PC）を記録する。server がローカル±24h の妥当な
    /// エポックmsでない、または差が ±24h を超える標本は捨てる（sync_max は 60 分間
    /// ヒューリスティックより優先されるため、別単位・誤再組み立ての 1 標本が全バフの
    /// 期限を ±数十年ずらし gc/purge が暴走するのを防ぐ。observe_server_time と同じ窓）。
    /// client=クライアント送信時刻・server=サーバ受信時刻なので、この差は
    /// 「時計差 − 片道遅延」＝遅延が大きいほど値が小さくなる。よってバケット内では
    /// 最大値（sync_max, keep_max=true）が時計差の最良近似になる
    /// （実測 2026-08-23 probe で確認: −78 ≤ 真値 ≤ −74 に対し標本は −1346〜−78 で
    /// 分布し、最大値 −78 が最も真値に近かった）。有効値が初回 or 1秒以上変化した
    /// ときだけ info ログを出す。
    pub fn observe_server_time_sync(&mut self, client_ms: i64, server_ms: i64, now_ms: u128) {
        if !is_plausible_server_time(server_ms, now_ms) {
            return;
        }
        let candidate = client_ms.saturating_sub(server_ms);
        if (candidate as i128).abs() >= MAX_SERVER_TIME_SKEW_MS {
            return;
        }
        let before = self.server_clock_offset.offset(now_ms);
        self.server_clock_offset.observe_sync(candidate, now_ms);
        let after = self.server_clock_offset.offset(now_ms);
        if offset_changed_enough(before, after) {
            log::info!(
                "buff_tracker: server_clock_offset exact {before:?}ms -> {after:?}ms (client={client_ms} server={server_ms})"
            );
        }
    }

    pub fn server_clock_offset_ms(&self, now_ms: u128) -> Option<i64> {
        self.server_clock_offset.offset(now_ms)
    }

    /// host_uuid が Player エンティティのバフのみ保存。保存した場合は true を返す。
    pub fn apply_full_info(&mut self, info: &pb::BuffSnapshot, now_ms: u128) -> bool {
        if EntityKind::from(info.host_uuid) != EntityKind::Player {
            return false;
        }
        self.observe_server_time(info.create_time, now_ms);
        let player_uid = entity::get_player_uid(info.host_uuid);

        let source_config_id = info
            .fight_source_info
            .as_ref()
            .map(|s| s.source_config_id)
            .unwrap_or(0);

        let state = BuffState {
            buff_uuid: info.buff_uuid,
            base_id: info.base_id,
            host_uuid: info.host_uuid,
            fire_uuid: info.fire_uuid,
            create_time_server: info.create_time,
            received_at_local_ms: now_ms,
            duration_ms: info.duration as i64,
            layer: info.layer,
            count: info.count,
            source_config_id,
            server_clock_trusted: true, // BuffSnapshot.create_time は意味が実測確定済み
        };

        self.buffs.entry(player_uid).or_default().insert(info.buff_uuid, state);
        true
    }

    /// 差分更新。host_uuid が Player でない場合は無視する。
    pub fn apply_change(&mut self, change: &pb::BuffTick, now_ms: u128) {
        if EntityKind::from(change.host_uuid) != EntityKind::Player {
            return;
        }
        self.observe_server_time(change.create_time, now_ms);
        let player_uid = entity::get_player_uid(change.host_uuid);
        let player_buffs = self.buffs.entry(player_uid).or_default();

        let entry = player_buffs.entry(change.buff_uuid).or_insert_with(|| BuffState {
            buff_uuid: change.buff_uuid,
            base_id: change.base_id,
            host_uuid: change.host_uuid,
            fire_uuid: 0,
            create_time_server: change.create_time,
            received_at_local_ms: now_ms,
            duration_ms: change.duration,
            layer: change.layer,
            count: 0,
            source_config_id: 0,
            // BuffTick は BuffSnapshot と同一 opcode/payload 由来だが、create_time=0 到来時は
            // 意味のある値を持たない。20行下の create_time_server 更新ガードと規約を統一し、
            // change.create_time != 0 のときだけ信頼できる情報とみなす。
            server_clock_trusted: change.create_time != 0,
        });

        // v0.8.3 以前と同様、duration/layer は create_time のガードなしで無条件更新。
        // サーバが BuffTick に create_time を付与しない実装の場合（0 で到来）、
        // 旧ガードは全 tick をスキップし残り秒数が凍結していた。
        entry.received_at_local_ms = now_ms;
        entry.duration_ms = change.duration;
        entry.layer = change.layer;
        entry.base_id = change.base_id;
        // create_time は付与の同一性キー。BuffTick は 0 で来ることがある（規約は
        // promote_trusted_create_time のコメント参照）。
        promote_trusted_create_time(entry, change.create_time);
    }

    /// buff_list (BuffEffect) の AddBuff (LogicEffect.EffectType == 18) を追跡。
    /// RawData は BuffInfo (= pb::BuffSnapshot)。BuffEffect.BuffUuid（インスタンスキー）で保存する。
    /// duration <= 0 は永続扱い（duration_ms=0 に正規化）。
    pub fn apply_buff_add(&mut self, buff_uuid: i32, info: &pb::BuffSnapshot, now_ms: u128, target_uid: i64) {
        self.observe_server_time(info.create_time, now_ms);
        let source_config_id = info
            .fight_source_info
            .as_ref()
            .map(|s| s.source_config_id)
            .unwrap_or(0);
        let duration_ms = if info.duration <= 0 { 0 } else { info.duration as i64 };
        if crate::probe::enabled() && crate::engine::consumables::is_consumable(info.base_id) {
            // 実機検証用（BPSR_PROBE 時のみ）: 再食/切替/マップ移動再送がどの形式で届くかを残す。
            log::info!(
                "consumable buff add: uid={target_uid} base={} uuid={buff_uuid} create_time={} duration={duration_ms} layer={} now={now_ms} offset={:?}",
                info.base_id, info.create_time, info.layer, self.server_clock_offset.offset(now_ms)
            );
        }
        let player_buffs = self.buffs.entry(target_uid).or_default();
        player_buffs.insert(
            buff_uuid,
            BuffState {
                buff_uuid,
                base_id: info.base_id,
                host_uuid: info.host_uuid,
                fire_uuid: info.fire_uuid,
                create_time_server: info.create_time,
                received_at_local_ms: now_ms,
                duration_ms,
                layer: info.layer,
                count: info.count,
                source_config_id,
                // promote_trusted_create_time と同じ規約: create_time=0 の snapshot を
                // trusted 扱いすると、フォールバック期限（受信時刻+総時間）が trusted として
                // consumables へ流れてしまう（appear 同期由来の再発行 buff_uuid が create_time
                // 無しで届くケースで残時間が再膨張する穴になる）。create_time が非0のときだけ
                // 信頼できる付与時刻として扱う。
                server_clock_trusted: info.create_time != 0,
            },
        );
    }

    /// buff_list (BuffEffect) の BuffChange (LogicEffect.EffectType == 19) を追跡。
    /// RawData は BuffChange{layer, duration, create_time}。base_id を持たないため、
    /// 同一 BuffUuid の既存バフの duration/layer を更新する。
    /// received_at_local_ms の再ベースは次の契約で行う: create_time が既存と一致し、
    /// かつ layer・duration のいずれも増加していない「冗長な再通知」は再ベースを
    /// スキップする（スタックが落ちただけの減少通知もここに含める）。layer 増加
    /// （スタック増加）・duration 増加（延長）・create_time 変化（別付与）の
    /// いずれかがあれば必ず再ベースし期限を延ばす
    /// （＝スタック増加・タイマーリフレッシュ ＝ ウィンドウから消えない）。
    /// ただし received_at_local_ms が期限に効くのは expire_at_local_ms のフォールバック
    /// （サーバ時計オフセット未知・create_time 不明）のときだけ。オフセット既知で
    /// create_time が妥当なら期限はサーバ値 create_time+duration で決まり、layer だけの
    /// 増加では延びない（サーバが延長するなら duration か create_time を更新して送る、
    /// という前提。scene-add 再送の probe 実測で duration は総時間として不変だった）。
    /// 未追跡 BuffUuid は無視。同種の「同一付与の冗長な再通知を除外する」述語は
    /// apply_effect にもあるが、create_time==0 の扱いと layer の有無が異なるため
    /// 共有ヘルパへは統合していない（詳細はガード実装直上のコメント）。
    pub fn apply_buff_change(
        &mut self,
        target_uid: i64,
        buff_uuid: i32,
        change: &pb::BuffChange,
        now_ms: u128,
    ) {
        // self.buffs を可変借用する前に呼ぶ（借用チェッカ対策。observe_server_time は
        // self.server_clock_offset_ms のみを触るが &mut self を取るため後段の
        // get_mut チェーンと同時には呼べない）。
        self.observe_server_time(change.create_time, now_ms);

        let Some(player_buffs) = self.buffs.get_mut(&target_uid) else {
            return;
        };
        let Some(state) = player_buffs.get_mut(&buff_uuid) else {
            return;
        };

        // 同一付与の冗長な再通知（create_time が既存と一致 かつ layer・duration が
        // どちらも変化なし）のときだけ received_at_local_ms を再ベースしない。
        // scene-change 経路は同一付与を何度も再通知することが実測されており
        // （probe: 26,918件中1,774キー・延べ2,534件が同一 (host_uuid, buff_uuid,
        // create_time)、最大14回・18.4秒再送）、無条件の再ベースだと残り秒が
        // 毎回満タンへ巻き戻り凍結して見える。
        // layer が「増加」していれば create_time が同一でも実イベント（スタック追加）の
        // 強い証左のため必ず再ベースする（本関数 doc の「スタック増加でウィンドウ
        // から消えない」契約を満たすために必須）。create_time が 0（不明）・
        // 別の付与・duration 増加（延長）の場合も同様に必ず再ベースする。
        //
        // 減少で再ベースしてはいけない。スタックが数秒ごとに1つずつ落ちる減衰型バフは
        // 同一 create_time・同一 duration のまま layer だけ減って再通知されうるため、
        // 「変化したら再ベース」にすると落ちるたびに残り時間が満タンへ巻き戻り、
        // このガードで直したはずの凍結（残り秒が減らない）が別経路で再発する。
        // layer==0（不明）は増加になり得ないのでこの式で自然に除外される。
        //
        // この関数は create_time==0 を「同一付与ではない」扱いにして必ず再ベース
        // する（is_same_grant に != 0 を必須化）。そのため v0.8.3 の凍結バグ
        // （BuffTick に create_time が付与されず 0 到来時に旧ガードが全 tick を
        // スキップし残り秒数が凍結した件）はこの条件式では再発しない。それでも
        // apply_change（BuffTick 経路）へ同ガードを入れないのは、経路が別だから
        // ではなく、probe 実測で BuffTick の同一付与再通知が 0 件＝ガードが
        // 必要かどうかを検証する材料が無く、未検証のまま挙動を変えるリスクを
        // 避けるため。
        if crate::probe::enabled() && crate::engine::consumables::is_consumable(state.base_id) {
            log::info!(
                "consumable buff change: uid={target_uid} base={} uuid={buff_uuid} create_time={}->{} duration={}->{} layer={}->{} now={now_ms}",
                state.base_id, state.create_time_server, change.create_time, state.duration_ms, change.duration, state.layer, change.layer
            );
        }
        let is_same_grant = change.create_time != 0 && change.create_time == state.create_time_server;
        let duration_increased = change.duration != 0 && {
            let normalized = if change.duration < 0 { 0 } else { change.duration };
            normalized > state.duration_ms
        };
        let layer_increased = change.layer > state.layer;
        // received_at の再ベースは受信基準フォールバック（trusted=false／offset 未知／
        // create_time が窓外）のときだけ期限に効く。サーバ時計基準の枝では期限は
        // create_time+duration のみで決まるため、サーバが重ねがけ時に create_time/duration を
        // 更新せず layer だけ増やす実装だった場合は延長されない（probe 実測は scene-add 再送と
        // duration 延長のみで、layer 単独更新の挙動は未確認）。
        if !is_same_grant || duration_increased || layer_increased {
            state.received_at_local_ms = now_ms;
        }

        if change.duration != 0 {
            state.duration_ms = if change.duration < 0 { 0 } else { change.duration };
        }
        if change.layer != 0 {
            state.layer = change.layer;
        }
        promote_trusted_create_time(state, change.create_time);
    }

    /// LocalSceneDelta.effects から取得した TimedEffect を追跡（常に local player 宛）。
    /// duration_ms <= 0 は無期限扱いでスキップ。
    /// activated_at が同一 かつ duration_ms も同一なら周期同期スキップ。
    /// activated_at が同じでも duration_ms が増加した場合は延長・再付与と判断して更新する。
    /// apply_buff_change と同種の「同一付与の冗長な再通知を除外する」述語だが、
    /// TimedEffect には layer 相当の概念が無く create_time==0 の扱いも異なるため
    /// 共有ヘルパへは統合していない（差異の詳細は apply_buff_change のガード直上コメント）。
    pub fn apply_effect(&mut self, effect: &pb::TimedEffect, now_ms: u128, local_uid: i64) {
        if effect.duration_ms <= 0 {
            return;
        }
        // activated_at はサーバ時計オフセット学習に使わない。エポックms として妥当な
        // 大きさに見えても「付与時刻」の意味である実測証拠が無く、もし実際には
        // 「失効時刻」等の別意味なら誤ったオフセットを学習し全バフが即失効表示になる
        // 恐れがある（must-fix）。このため create_time_server は常に受信基準
        // フォールバックのみで扱う（server_clock_trusted=false）。
        let id = effect.id as i32;
        let player_buffs = self.buffs.entry(local_uid).or_default();
        if let Some(existing) = player_buffs.get(&id) {
            if existing.create_time_server == effect.activated_at
                && effect.duration_ms <= existing.duration_ms
            {
                return;
            }
        }
        player_buffs.insert(id, BuffState {
            buff_uuid: id,
            base_id: id,
            host_uuid: 0,
            fire_uuid: 0,
            create_time_server: effect.activated_at,
            received_at_local_ms: now_ms,
            duration_ms: effect.duration_ms,
            layer: 1,
            count: 1,
            source_config_id: 0,
            server_clock_trusted: false,
        });
    }

    pub fn remove(&mut self, player_uid: i64, buff_uuid: i32) {
        if let Some(player_buffs) = self.buffs.get_mut(&player_uid) {
            player_buffs.remove(&buff_uuid);
        }
    }

    /// 期限切れバフを削除する。duration_ms <= 0 は無期限扱いで削除しない。
    /// バフが空になったプレイヤーエントリも除去する。
    pub fn gc(&mut self, now_ms: u128) {
        let offset = self.server_clock_offset_ms(now_ms);
        for player_buffs in self.buffs.values_mut() {
            player_buffs.retain(|_, state| match expire_of(state, offset) {
                None => true, // 無期限
                Some(expire_at) => now_ms < expire_at,
            });
        }
        self.buffs.retain(|_, player_buffs| !player_buffs.is_empty());
    }

    /// 特定プレイヤーの remaining_ms を計算したスナップショットを返す。
    pub fn snapshot_for(&self, player_uid: i64, now_ms: u128) -> Vec<BuffStateSnapshot> {
        let Some(player_buffs) = self.buffs.get(&player_uid) else {
            return vec![];
        };
        make_snapshots(player_buffs, now_ms, self.server_clock_offset_ms(now_ms))
    }

    /// 全プレイヤーのスナップショットを player_uid ごとに返す。
    pub fn snapshot_all(&self, now_ms: u128) -> HashMap<i64, Vec<BuffStateSnapshot>> {
        let offset = self.server_clock_offset_ms(now_ms);
        self.buffs
            .iter()
            .map(|(uid, player_buffs)| (*uid, make_snapshots(player_buffs, now_ms, offset)))
            .collect()
    }

    /// server_clock_offset は意図的に保持する（手動リセット・戦闘終了を跨いでも
    /// サーバ時計オフセットの推定は有効なため）。
    pub fn clear(&mut self) {
        self.buffs.clear();
    }

    /// バフを追跡中の player_uid を first-seen 順（各 player の `received_at_local_ms` 最小値の
    /// 昇順）で返す。バフが空の player（gc 等で消えた直後の幽霊エントリ）は除外する。
    /// イマジン専用モード（メイン集計を回さない軽量モード）でタイマー名簿を自動供給するために使う。
    pub fn tracked_player_uids_by_first_seen(&self) -> Vec<i64> {
        let mut entries: Vec<(i64, u128)> = self
            .buffs
            .iter()
            .filter_map(|(uid, player_buffs)| {
                player_buffs
                    .values()
                    .map(|state| state.received_at_local_ms)
                    .min()
                    .map(|first_seen| (*uid, first_seen))
            })
            .collect();
        entries.sort_by_key(|(_, first_seen)| *first_seen);
        entries.into_iter().map(|(uid, _)| uid).collect()
    }
}

fn make_snapshots(
    player_buffs: &HashMap<i32, BuffState>,
    now_ms: u128,
    offset: Option<i64>,
) -> Vec<BuffStateSnapshot> {
    player_buffs
        .values()
        .map(|state| {
            let expire_at = expire_of(state, offset);
            let remaining_ms = match expire_at {
                None => i64::MAX,
                Some(expire_at) => (expire_at as i128 - now_ms as i128) as i64,
            };
            BuffStateSnapshot {
                buff_uuid: state.buff_uuid,
                base_id: state.base_id,
                fire_uuid: state.fire_uuid,
                received_at_local_ms: state.received_at_local_ms,
                duration_ms: state.duration_ms,
                remaining_ms,
                layer: state.layer,
                count: state.count,
                create_time_server: state.create_time_server,
                expire_at_local_ms: expire_at,
                server_clock_trusted: state.server_clock_trusted,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_buff_info(buff_uuid: i32, host_uuid: i64, duration: i32) -> pb::BuffSnapshot {
        pb::BuffSnapshot {
            buff_uuid,
            base_id: 30001,
            level: 1,
            host_uuid,
            table_uuid: 0,
            create_time: 0,
            fire_uuid: 99,
            layer: 1,
            part_id: 0,
            count: 1,
            duration,
            fight_source_info: None,
        }
    }

    // player_uid << 16 | 640 でプレイヤー packed UUID を構成する
    fn player_uuid(player_uid: i64) -> i64 {
        (player_uid << 16) | 640
    }

    // monster_uid << 16 | 64 でモンスター packed UUID を構成する
    fn monster_uuid(monster_uid: i64) -> i64 {
        (monster_uid << 16) | 64
    }

    #[test]
    fn test_player_uuid_filter() {
        let mut tracker = BuffTracker::new();
        let uid_a = entity::get_player_uid(player_uuid(1));
        let uid_b = entity::get_player_uid(player_uuid(2));

        let info_a = make_buff_info(1, player_uuid(1), 5000);
        let info_b = make_buff_info(2, player_uuid(2), 5000);
        let info_monster = make_buff_info(3, monster_uuid(99), 5000);

        assert!(tracker.apply_full_info(&info_a, 0));
        assert!(tracker.apply_full_info(&info_b, 0));
        assert!(!tracker.apply_full_info(&info_monster, 0));

        // uid_a と uid_b は独立して保持される
        assert_eq!(tracker.snapshot_for(uid_a, 0).len(), 1);
        assert_eq!(tracker.snapshot_for(uid_b, 0).len(), 1);
        assert_eq!(tracker.snapshot_for(uid_a, 0)[0].buff_uuid, 1);
        assert_eq!(tracker.snapshot_for(uid_b, 0)[0].buff_uuid, 2);

        // モンスターは保存されない
        let all = tracker.snapshot_all(0);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_gc_removes_expired() {
        let mut tracker = BuffTracker::new();
        let uuid = player_uuid(1);
        let uid = entity::get_player_uid(uuid);

        let info = make_buff_info(1, uuid, 1000);
        tracker.apply_full_info(&info, 0);

        tracker.gc(999);
        assert_eq!(tracker.snapshot_for(uid, 999).len(), 1);

        tracker.gc(1000);
        assert_eq!(tracker.snapshot_for(uid, 1000).len(), 0);
        // 空になったプレイヤーエントリも除去
        assert!(tracker.snapshot_all(1000).is_empty());
    }

    #[test]
    fn test_snapshot_remaining_ms() {
        let mut tracker = BuffTracker::new();
        let uuid = player_uuid(1);
        let uid = entity::get_player_uid(uuid);

        let info = make_buff_info(1, uuid, 5000);
        tracker.apply_full_info(&info, 0);

        let snaps = tracker.snapshot_for(uid, 2000);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].remaining_ms, 3000);
    }

    #[test]
    fn test_refresh_resets_received_at() {
        let mut tracker = BuffTracker::new();
        let uuid = player_uuid(1);
        let uid = entity::get_player_uid(uuid);

        let info = make_buff_info(1, uuid, 1000);
        tracker.apply_full_info(&info, 0);
        tracker.apply_full_info(&info, 800);

        let snaps = tracker.snapshot_for(uid, 1500);
        assert_eq!(snaps[0].remaining_ms, 300);
    }

    #[test]
    fn test_duration_zero_is_permanent() {
        let mut tracker = BuffTracker::new();
        let uuid = player_uuid(1);
        let uid = entity::get_player_uid(uuid);

        let info = make_buff_info(1, uuid, 0);
        tracker.apply_full_info(&info, 0);

        tracker.gc(u128::MAX / 2);
        assert_eq!(tracker.snapshot_for(uid, u128::MAX / 2).len(), 1);
    }

    #[test]
    fn test_two_players_isolated() {
        let mut tracker = BuffTracker::new();
        let uuid_a = player_uuid(1);
        let uuid_b = player_uuid(2);
        let uid_a = entity::get_player_uid(uuid_a);
        let uid_b = entity::get_player_uid(uuid_b);

        tracker.apply_full_info(&make_buff_info(10, uuid_a, 5000), 0);
        tracker.apply_full_info(&make_buff_info(10, uuid_b, 2000), 0);

        // 同じ buff_uuid でも別プレイヤーとして独立
        let snap_a = tracker.snapshot_for(uid_a, 0);
        let snap_b = tracker.snapshot_for(uid_b, 0);
        assert_eq!(snap_a[0].duration_ms, 5000);
        assert_eq!(snap_b[0].duration_ms, 2000);
    }

    // buff_list (BuffEffect) 経路: AddBuff で BuffUuid キーに保存し、
    // BuffChange (EffectType 19) で duration/layer を更新し received_at を再ベースする
    // ＝期限間際の再付与でも消えずに延長され、スタック数が反映される。
    #[test]
    fn test_buff_change_refreshes_and_stacks() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;

        // AddBuff: buff_uuid=77（インスタンスキー）, base_id=30001, duration=1000
        tracker.apply_buff_add(77, &make_buff_info(1, player_uuid(1), 1000), 0, uid);

        // 期限間際(900ms)に BuffChange: duration=1000, layer=3（スタック3）
        let change = pb::BuffChange { layer: 3, duration: 1000, create_time: 0 };
        tracker.apply_buff_change(uid, 77, &change, 900);

        // 再ベースされていれば期限は 900+1000=1900ms。1500ms 時点で残り 400ms（消えない）。
        let snaps = tracker.snapshot_for(uid, 1500);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].remaining_ms, 400);
        assert_eq!(snaps[0].layer, 3);
        assert_eq!(snaps[0].base_id, 30001);
    }

    // scene-change 経路の同一付与再通知: create_time が既存と一致するなら受信のたびに
    // 再ベースしない＝残り秒が凍結せず減り続ける（probe実測: 最大14回・18.4秒再送）。
    #[test]
    fn test_buff_change_same_create_time_does_not_rebase() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;

        // AddBuff: create_time=100_000_000（ローカル時刻窓±24hの外に置き、サーバ時計
        // オフセット学習の対象外にする識別子。値そのものに意味は無い）, duration=25000
        let mut info = make_buff_info(1, player_uuid(1), 25000);
        info.create_time = 100_000_000;
        tracker.apply_buff_add(77, &info, 0, uid);

        // 同一 create_time の BuffChange が複数回再通知される（duration は変わらず）
        let change = pb::BuffChange { layer: 1, duration: 25000, create_time: 100_000_000 };
        tracker.apply_buff_change(uid, 77, &change, 5000);
        tracker.apply_buff_change(uid, 77, &change, 10000);
        tracker.apply_buff_change(uid, 77, &change, 18400);

        // received_at_local_ms は初回付与時の 0 のまま＝期限は 0+25000=25000ms
        let snaps = tracker.snapshot_for(uid, 20000);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].remaining_ms, 5000);
    }

    // create_time が変わった場合（別の付与）は従来どおり再ベースされる。
    #[test]
    fn test_buff_change_different_create_time_rebases() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;

        let mut info = make_buff_info(1, player_uuid(1), 5000);
        info.create_time = 100_000_000; // ローカル時刻窓の外に置く識別子（値に意味は無い）
        tracker.apply_buff_add(77, &info, 0, uid);

        // 別の付与（create_time=200_000_000）として通知
        let change = pb::BuffChange { layer: 1, duration: 5000, create_time: 200_000_000 };
        tracker.apply_buff_change(uid, 77, &change, 3000);

        // 再ベースされていれば期限は 3000+5000=8000ms。7000ms 時点で残り 1000ms。
        let snaps = tracker.snapshot_for(uid, 7000);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].remaining_ms, 1000);
        assert_eq!(snaps[0].create_time_server, 200_000_000);
    }

    // 同一 create_time でも duration が増えた場合（延長）は再ベースされる。
    #[test]
    fn test_buff_change_same_create_time_but_extended_duration_rebases() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;

        let mut info = make_buff_info(1, player_uuid(1), 5000);
        info.create_time = 100_000_000; // ローカル時刻窓の外に置く識別子（値に意味は無い）
        tracker.apply_buff_add(77, &info, 0, uid);

        // 同一 create_time だが duration が 5000→10000 へ延長
        let change = pb::BuffChange { layer: 1, duration: 10000, create_time: 100_000_000 };
        tracker.apply_buff_change(uid, 77, &change, 4000);

        // 延長として再ベースされていれば期限は 4000+10000=14000ms。9000ms 時点で残り 5000ms。
        let snaps = tracker.snapshot_for(uid, 9000);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].remaining_ms, 5000);
        assert_eq!(snaps[0].duration_ms, 10000);
    }

    // 同一 create_time でも layer が増えた場合（スタック増加）は再ベースされる。
    // ＝ doc の「スタック増加・タイマーリフレッシュ ＝ ウィンドウから消えない」契約。
    #[test]
    fn test_buff_change_same_create_time_but_increased_layer_rebases() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;

        // AddBuff: create_time=100_000_000（ローカル時刻窓の外に置く識別子）, duration=5000, layer=1
        let mut info = make_buff_info(1, player_uuid(1), 5000);
        info.create_time = 100_000_000;
        tracker.apply_buff_add(77, &info, 0, uid);

        // 同一 create_time だが layer が 1→2 へ増加（スタック追加）。duration は同じ。
        let change = pb::BuffChange { layer: 2, duration: 5000, create_time: 100_000_000 };
        tracker.apply_buff_change(uid, 77, &change, 4000);

        // 再ベースされていれば期限は 4000+5000=9000ms。8000ms 時点で残り 1000ms（消えない）。
        let snaps = tracker.snapshot_for(uid, 8000);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].remaining_ms, 1000);
        assert_eq!(snaps[0].layer, 2);
    }

    // 同一 create_time かつ layer も同一なら再ベースされない（凍結抑止が layer 追加後も壊れていない）。
    #[test]
    fn test_buff_change_same_create_time_and_same_layer_does_not_rebase() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;

        // AddBuff: create_time=100_000_000（ローカル時刻窓の外に置く識別子）, duration=5000, layer=1
        let mut info = make_buff_info(1, player_uuid(1), 5000);
        info.create_time = 100_000_000;
        tracker.apply_buff_add(77, &info, 0, uid);

        // 同一 create_time・同一 layer=1 の冗長な再通知
        let change = pb::BuffChange { layer: 1, duration: 5000, create_time: 100_000_000 };
        tracker.apply_buff_change(uid, 77, &change, 4000);

        // 再ベースされていなければ期限は 0+5000=5000ms。4500ms 時点で残り 500ms。
        let snaps = tracker.snapshot_for(uid, 4500);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].remaining_ms, 500);
    }

    // 同一 create_time で layer が「減った」だけの再通知は再ベースしない。
    // スタックが数秒ごとに1つ落ちる減衰型バフは同一 create_time・同一 duration のまま
    // layer だけ減って再通知されうるため、減少でも再ベースすると落ちるたびに残り時間が
    // 満タンへ巻き戻り、凍結（残り秒が減らない）が再発する。
    #[test]
    fn test_buff_change_decreased_layer_does_not_rebase() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;

        // AddBuff: create_time=100_000_000（ローカル時刻窓の外に置く識別子）,
        // duration=5000, layer=1（期限 0+5000=5000ms）
        let mut info = make_buff_info(1, player_uuid(1), 5000);
        info.create_time = 100_000_000;
        tracker.apply_buff_add(77, &info, 0, uid);

        // t=1000: スタック追加(1→3)。増加なので再ベースされ期限は 1000+5000=6000ms。
        let up = pb::BuffChange { layer: 3, duration: 5000, create_time: 100_000_000 };
        tracker.apply_buff_change(uid, 77, &up, 1000);

        // t=3000: スタックが1つ落ちる(3→2)。create_time も duration も同じ＝減少のみ。
        let down = pb::BuffChange { layer: 2, duration: 5000, create_time: 100_000_000 };
        tracker.apply_buff_change(uid, 77, &down, 3000);

        // 減少で再ベースしていなければ期限は 6000ms のまま。5500ms 時点で残り 500ms。
        // （再ベースしてしまうと 3000+5000=8000ms になり残り 2500ms へ巻き戻る）
        let snaps = tracker.snapshot_for(uid, 5500);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].remaining_ms, 500, "スタック減少で残り時間が巻き戻っている");
        // 再ベースしないだけで layer 自体は最新値へ更新する。
        assert_eq!(snaps[0].layer, 2);
    }

    // BuffTick が create_time=0 で来ても、既存の正規 create_time を 0 で潰さない。
    // ＝ 食事/シロップの再付与判定が受動 tick で壊れない（残時間がグレーへ戻らない）。
    #[test]
    fn test_buff_tick_zero_create_time_preserves_existing() {
        let mut tracker = BuffTracker::new();
        let uuid = player_uuid(1);
        let uid = entity::get_player_uid(uuid);

        // AddBuff/Snapshot 相当: 正規の create_time=1234 を持つ
        let mut info = make_buff_info(7, uuid, 5000);
        info.create_time = 1234;
        tracker.apply_full_info(&info, 0);

        // BuffTick が create_time=0 で到来
        let tick = pb::BuffTick {
            host_uuid: uuid,
            buff_uuid: 7,
            base_id: 30001,
            duration: 5000,
            create_time: 0,
            layer: 1,
        };
        tracker.apply_change(&tick, 1000);

        let snaps = tracker.snapshot_for(uid, 1000);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].create_time_server, 1234); // 0 で潰されていない
    }

    // 未追跡 BuffUuid への BuffChange は無視する（base_id 不明で表示できないため）。
    #[test]
    fn test_buff_change_unknown_uuid_ignored() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;
        let change = pb::BuffChange { layer: 2, duration: 1000, create_time: 0 };
        tracker.apply_buff_change(uid, 999, &change, 0);
        assert!(tracker.snapshot_for(uid, 0).is_empty());
    }

    // Remove イベントは BuffUuid キーで削除する。
    #[test]
    fn test_buff_remove_by_uuid() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;
        tracker.apply_buff_add(42, &make_buff_info(1, player_uuid(1), 5000), 0, uid);
        assert_eq!(tracker.snapshot_for(uid, 0).len(), 1);
        tracker.remove(uid, 42);
        assert!(tracker.snapshot_for(uid, 0).is_empty());
    }

    // AddBuff の duration <= 0 は永続（duration_ms=0 に正規化）として gc で消えない。
    #[test]
    fn test_buff_add_negative_duration_is_permanent() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;
        tracker.apply_buff_add(42, &make_buff_info(1, player_uuid(1), -1), 0, uid);

        tracker.gc(u128::MAX / 2);
        let snaps = tracker.snapshot_for(uid, u128::MAX / 2);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].duration_ms, 0);
    }

    // first-seen 順: 各 player の最小 received_at_local_ms 昇順で並ぶ。
    // 後から検出された player でも先に最初のバフを受けていれば先頭に来る。
    #[test]
    fn test_tracked_player_uids_by_first_seen_orders_by_min_received_at() {
        let mut tracker = BuffTracker::new();
        // uid=200 は t=500 で初検出
        tracker.apply_buff_add(1, &make_buff_info(1, player_uuid(200), 5000), 500, 200);
        // uid=100 は t=100 で初検出（後で挿入するが先に検出されている）
        tracker.apply_buff_add(2, &make_buff_info(2, player_uuid(100), 5000), 100, 100);
        // uid=100 に2つ目のバフが t=900 で付与されても、最小値である100は変わらない
        tracker.apply_buff_add(3, &make_buff_info(3, player_uuid(100), 5000), 900, 100);

        let order = tracker.tracked_player_uids_by_first_seen();
        assert_eq!(order, vec![100, 200]);
    }

    // バフが空の player（gc 等で全消失）は列挙から除外される。
    #[test]
    fn test_tracked_player_uids_by_first_seen_excludes_empty() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;
        tracker.apply_buff_add(1, &make_buff_info(1, player_uuid(1), 1000), 0, uid);
        tracker.gc(2000); // duration_ms=1000 のバフは期限切れ→空エントリは除去される
        assert!(tracker.tracked_player_uids_by_first_seen().is_empty());
    }

    // ─── サーバ時計オフセット推定（マップ移動での残時間膨張バグの修正） ─────────────

    // オフセットは同一バケット内では観測最小値を保つ（古い付与の再送で増えない）・
    // clear() で消えない。now はすべて同一バケット内に収める
    // （バケットをまたぐ挙動は test_server_clock_offset_bucket_expiry_* で別途検証）。
    #[test]
    fn test_server_clock_offset_tracks_minimum_and_survives_clear() {
        let mut tracker = BuffTracker::new();
        const BASE: u128 = 10 * SERVER_CLOCK_OFFSET_BUCKET_MS;

        // 初回観測: create_time=BASE, now=BASE+5000 → offset=5000
        tracker.observe_server_time(BASE as i64, BASE + 5_000);
        assert_eq!(tracker.server_clock_offset_ms(BASE + 5_000), Some(5_000));

        // 古い付与の再送（マップ移動）: create_time は据え置きだが受信が遅い→候補が大きい→更新されない
        tracker.observe_server_time(BASE as i64, BASE + 100_000);
        assert_eq!(tracker.server_clock_offset_ms(BASE + 100_000), Some(5_000));

        // より小さい候補が来れば更新される（同バケット内）
        tracker.observe_server_time((BASE + 147_000) as i64, BASE + 150_000);
        assert_eq!(tracker.server_clock_offset_ms(BASE + 150_000), Some(3_000));

        tracker.clear();
        assert_eq!(tracker.server_clock_offset_ms(BASE + 150_000), Some(3_000)); // clear() で消えない
    }

    // expire_at_local_ms 単体: オフセット未知・create_time implausible（BuffTick の0等）は
    // 受信基準フォールバック。無期限（duration<=0）は None。
    #[test]
    fn test_expire_at_local_ms_fallback_and_permanent() {
        // オフセット未知
        assert_eq!(expire_at_local_ms(1_700_000_000_000, 1000, 5000, None, true), Some(6000));
        // create_time=0 (BuffTick) はオフセットが既知でもフォールバック
        assert_eq!(expire_at_local_ms(0, 1000, 5000, Some(300), true), Some(6000));
        // create_time は絶対値としては妥当に見えても、received_at_local_ms との差が
        // MAX_SERVER_TIME_SKEW_MS（24h）を超える場合は別単位・無関係な値とみなし
        // フォールバックする（窓外→フォールバック。別単位混入の除外を示す例）。
        assert_eq!(expire_at_local_ms(1_700_000_000_000, 1000, 5000, Some(300), true), Some(6000));
        // オフセット既知・create_time が received_at_local_ms の24h以内ならサーバ時計基準
        assert_eq!(
            expire_at_local_ms(1_700_000_000_000, 1_700_000_000_700, 5000, Some(300), true),
            Some(1_700_000_005_300)
        );
        // trusted=false（TimedEffect.activated_at 由来）は create_time/offset が妥当でも
        // 常に受信基準フォールバック（付与時刻としての意味が実測未確認のため）。
        assert_eq!(
            expire_at_local_ms(1_700_000_000_000, 1_700_000_000_700, 5000, Some(300), false),
            Some(1_700_000_005_700)
        );
        // 無期限
        assert_eq!(expire_at_local_ms(1_700_000_000_000, 1_700_000_000_700, 0, Some(300), true), None);
        assert_eq!(expire_at_local_ms(1_700_000_000_000, 1_700_000_000_700, -1, Some(300), true), None);
    }

    // create_time+offset+duration が負になっても 0 へクランプする（極端なオフセットへの
    // 防御。ログは1回だけ warn するのみでテスト対象外）。
    #[test]
    fn test_expire_at_local_ms_clamps_negative_to_zero() {
        assert_eq!(expire_at_local_ms(1000, 1000, 500, Some(-2000), true), Some(0));
    }

    // マップ移動の resend モデル: 元付与から22.6分後、同一 create_time・同一 duration の
    // まま別 buff_uuid で再送されても、サーバ時計オフセットが既知なら受信時刻に依存せず
    // 「create_time+offset+duration」で期限を計算する＝残時間が膨張しない
    // （2026-08-22 不具合の根本原因の再現・修正確認）。
    #[test]
    fn test_map_move_resend_uses_server_time_not_receive_time() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;

        const T0: i64 = 1_700_000_000_000; // 妥当なサーバ時刻(epoch ms)
        const TRUE_OFFSET: i64 = 300; // ローカル-サーバの真の時計ズレ+初回受信遅延
        const DURATION: i32 = 600_000; // 10分

        // 初回付与: ほぼ即時受信でオフセットが確定する(300ms)
        let mut info = make_buff_info(3, player_uuid(1), DURATION);
        info.create_time = T0;
        tracker.apply_buff_add(3, &info, (T0 + TRUE_OFFSET) as u128, uid);
        assert_eq!(tracker.server_clock_offset_ms((T0 + TRUE_OFFSET) as u128), Some(TRUE_OFFSET));

        // 22.6分後、マップ移動により同一付与が新 buff_uuid=8 で再送される
        // （create_time・duration は不変、受信時刻だけ22.6分後）
        const ELAPSED_MS: i64 = 1_356_000;
        let now = (T0 + TRUE_OFFSET + ELAPSED_MS) as u128;
        let mut resend = make_buff_info(8, player_uuid(1), DURATION);
        resend.create_time = T0;
        tracker.apply_buff_add(8, &resend, now, uid);

        // 旧実装(受信基準)なら now+600_000 になり残り10分と誤表示される。
        // 修正後は create_time+offset+duration-now を使うため、22.6分>10分ゆえ既に失効(負値)。
        let snaps = tracker.snapshot_for(uid, now);
        let snap8 = snaps.iter().find(|s| s.buff_uuid == 8).unwrap();
        let expected = (T0 as i128 + TRUE_OFFSET as i128 + DURATION as i128 - now as i128) as i64;
        assert_eq!(snap8.remaining_ms, expected);
        assert!(snap8.remaining_ms < 0, "22.6分経過後は10分バフは失効しているはず");

        // gc も同じ期限で消す
        tracker.gc(now);
        assert!(tracker.snapshot_for(uid, now).iter().all(|s| s.buff_uuid != 8));
    }

    // apply_effect は create_time_server（TimedEffect.activated_at）の意味＝「付与時刻」
    // である実測証拠が無いため、サーバ時計オフセット学習に一切使わない
    // （意味が違えば誤ったオフセットを学習し全バフ即失効表示になりうるため must-fix）。
    // activated_at をエポックms として妥当かつ now とほぼ一致させ、「もし学習対象なら
    // 確実にオフセットが確定する」状況を意図的に作った上でも None のままであることを確認する。
    #[test]
    fn test_apply_effect_never_learns_server_clock_offset() {
        let mut tracker = BuffTracker::new();
        let uid = 5000;

        let now: u128 = 1_700_000_000_100;
        let effect = pb::TimedEffect {
            id: 1,
            activated_at: 1_700_000_000_000,
            duration_ms: 5000,
            flag: 0,
            extra: 0,
        };
        tracker.apply_effect(&effect, now, uid);

        assert_eq!(tracker.server_clock_offset_ms(now), None);
    }

    // 2バケット以上の空白の直後に大きい候補（マップ移動 resend 相当）だけが届いても、
    // リセット直前の推定値が prev へ繰り越されるため即座には上書きされない
    // （single-observation でオフセットが汚染される既知の問題への対処）。
    #[test]
    fn test_server_clock_offset_carries_forward_old_estimate_across_long_gap() {
        let mut tracker = BuffTracker::new();

        let now1: u128 = 10 * SERVER_CLOCK_OFFSET_BUCKET_MS; // バケット境界に揃える
        tracker.observe_server_time(now1 as i64 - 300, now1); // candidate=300
        assert_eq!(tracker.server_clock_offset_ms(now1), Some(300));

        // 3バケット分の空白（2バケット以上）の後、resend 相当の大きい候補だけが届く
        let now2 = now1 + 3 * SERVER_CLOCK_OFFSET_BUCKET_MS;
        tracker.observe_server_time(now2 as i64 - 1_356_000, now2); // candidate=1,356,000

        // 旧推定(300)が prev へ繰り越されているため、まだ上書きされない
        assert_eq!(tracker.server_clock_offset_ms(now2), Some(300));
    }

    // その後さらに1バケット進んで、より大きい候補だけが観測され続ければ、繰り越した
    // 旧推定が prev から自然に抜け、新しい値へ切り替わる（上方向の再学習。単調 min の
    // ままだと古い小さいオフセットに永久固定され、ローカル時計の前方ジャンプ後の正しい
    // オフセットへ追従できない＝must-fix）。切り替わりはリセット直後ではなく、
    // 繰り越した旧推定が prev から抜ける「もう1バケット後」に起こる。
    #[test]
    fn test_server_clock_offset_bucket_expiry_allows_upward_relearning() {
        let mut tracker = BuffTracker::new();

        let now1: u128 = 10 * SERVER_CLOCK_OFFSET_BUCKET_MS;
        tracker.observe_server_time(now1 as i64 - 300, now1); // candidate=300

        let now2 = now1 + 3 * SERVER_CLOCK_OFFSET_BUCKET_MS; // 2バケット以上の空白
        tracker.observe_server_time(now2 as i64 - 1_356_000, now2); // candidate=1,356,000
        assert_eq!(tracker.server_clock_offset_ms(now2), Some(300)); // リセット直後はまだ旧値

        // さらに1バケット進んで、より大きい候補だけが観測され続ける
        let now3 = now2 + SERVER_CLOCK_OFFSET_BUCKET_MS;
        tracker.observe_server_time(now3 as i64 - 5_000, now3); // candidate=5000（旧値300より大きい）

        // 旧推定(300)は繰り越し後さらに1バケット経過して prev から抜けたため、
        // 新しい値へ切り替わる。
        assert_eq!(tracker.server_clock_offset_ms(now3), Some(5_000));
    }

    // 「ちょうど+1バケット」のスライドでは prev に旧オフセットが残るため、
    // マップ移動 resend（実測最大22.6分後相当）の大きい候補では上書きされない。
    // これが SERVER_CLOCK_OFFSET_BUCKET_MS を30分にした根拠そのもの
    // （22.6分の空白は常に1バケット分のスライドに収まる必要がある）。
    #[test]
    fn test_server_clock_offset_single_bucket_slide_protects_against_map_move_resend() {
        const OBSERVED_MAX_RESEND_GAP_MS: i64 = 1_356_000; // 実測(2026-07-25 probe)の最大空白 約22.6分
        assert!(
            SERVER_CLOCK_OFFSET_BUCKET_MS as i64 > OBSERVED_MAX_RESEND_GAP_MS,
            "バケット幅が実測 resend 間隔以下だと2バケット以上のジャンプになり守られない"
        );

        let mut tracker = BuffTracker::new();

        // バケット k 内で候補 300 を観測
        let now1: u128 = 10 * SERVER_CLOCK_OFFSET_BUCKET_MS;
        tracker.observe_server_time(now1 as i64 - 300, now1);
        assert_eq!(tracker.server_clock_offset_ms(now1), Some(300));

        // ちょうど1バケット進んだ(k+1)時点で、resend 相当の大きい候補のみ観測
        let now2 = now1 + SERVER_CLOCK_OFFSET_BUCKET_MS;
        tracker.observe_server_time(now2 as i64 - OBSERVED_MAX_RESEND_GAP_MS, now2);

        // 1バケットのスライドでは prev に旧オフセット(300)が残るため守られる
        assert_eq!(tracker.server_clock_offset_ms(now2), Some(300));
    }

    // ─── SyncServerTime(0x2B) 由来の sync_max（バケット制の max 推定） ──────────────

    // sync_max（SyncServerTime 由来）が既知かつ新しければ、ヒューリスティック
    // (heuristic_min、バケット min 推定)より優先される。
    #[test]
    fn test_server_clock_offset_sync_max_overrides_heuristic() {
        let mut tracker = BuffTracker::new();

        // ヒューリスティックで 5000 を確定させる
        tracker.observe_server_time(1_700_000_000_000, 1_700_000_005_000);
        assert_eq!(tracker.server_clock_offset_ms(1_700_000_005_000), Some(5_000));

        // sync標本(0x2B相当)が届く。ヒューリスティックの結果とは異なる値でも最優先される。
        tracker.observe_server_time_sync(1_700_000_010_100, 1_700_000_010_000, 1_700_000_010_100);
        assert_eq!(tracker.server_clock_offset_ms(1_700_000_010_100), Some(100));
    }

    // 観測から SYNC_MAX_AGE_MS(60分) 以上経った sync_max は無視してヒューリスティックへ
    // フォールバックする（スリープ復帰等で同期が止まった場合の保険）。
    #[test]
    fn test_server_clock_offset_sync_max_falls_back_to_heuristic_after_max_age() {
        let mut tracker = BuffTracker::new();

        // フォールバック先のヒューリスティックをあらかじめ確定させておく
        tracker.observe_server_time(1_700_000_000_000, 1_700_000_005_000);
        assert_eq!(tracker.server_clock_offset_ms(1_700_000_005_000), Some(5_000));

        // sync標本(-78相当)が届く
        tracker.observe_server_time_sync(1_700_000_010_000, 1_700_000_010_078, 1_700_000_010_000); // candidate=-78
        assert_eq!(tracker.server_clock_offset_ms(1_700_000_010_000), Some(-78));

        // 60分未満: sync_max がまだ有効
        let just_before = 1_700_000_010_000 + SYNC_MAX_AGE_MS - 1;
        assert_eq!(tracker.server_clock_offset_ms(just_before), Some(-78));

        // 60分以上経過: sync_max を無視し、ヒューリスティックへフォールバックする
        let after = 1_700_000_010_000 + SYNC_MAX_AGE_MS;
        assert_eq!(tracker.server_clock_offset_ms(after), Some(5_000));
    }

    // 実測(2026-08-23 probe): ロード中(ServerHandover直後)の同期は片道遅延が大きく
    // 標本が外れ値(-1346)になった。client=クライアント送信時刻・server=サーバ受信時刻
    // なので client-server は「時計差-片道遅延」＝遅延が大きいほど値が小さくなる。
    // 旧仕様（無条件上書き）だと次の同期まで外れ値をそのまま採用してしまっていた。
    // 新仕様（同一バケット内は max を保持）なら、より真値に近い標本(-78)が一度届けば、
    // 後から外れ値が再度届いても退行しない。
    #[test]
    fn test_server_clock_offset_sync_max_ignores_outlier_after_recovery() {
        let mut tracker = BuffTracker::new();

        const T1_CLIENT: i64 = 1_787_461_204_697;
        const T1_SERVER: i64 = 1_787_461_206_043; // candidate = -1346
        let t1 = T1_CLIENT as u128;
        tracker.observe_server_time_sync(T1_CLIENT, T1_SERVER, t1);
        assert_eq!(tracker.server_clock_offset_ms(t1), Some(-1346));

        // 約5秒後、片道遅延が小さい標本(-78)が届く。同一バケット内では
        // max(-1346, -78) = -78 を採用する。
        const T2_CLIENT: i64 = 1_787_461_209_709;
        const T2_SERVER: i64 = 1_787_461_209_787; // candidate = -78
        let t2 = T2_CLIENT as u128;
        tracker.observe_server_time_sync(T2_CLIENT, T2_SERVER, t2);
        assert_eq!(tracker.server_clock_offset_ms(t2), Some(-78));

        // さらに外れ値(-1346)が届いても、max を保持するため -78 のまま
        // （旧仕様の無条件上書きなら -1346 へ退行していた）。
        let t3 = t2 + 5_000;
        tracker.observe_server_time_sync(t3 as i64, t3 as i64 + 1346, t3); // candidate=-1346
        assert_eq!(
            tracker.server_clock_offset_ms(t3),
            Some(-78),
            "外れ値の再到来で退行してはいけない"
        );
    }

    // 同一バケット内では sync_max（keep_max=true）が候補の最大値を保持する
    // （heuristic_min とは逆方向。より小さい候補が来ても縮まず、より大きい候補が
    // 来れば更新される）。
    #[test]
    fn test_server_clock_offset_sync_max_keeps_larger_value_within_bucket() {
        let mut tracker = BuffTracker::new();
        let t: u128 = 10 * SYNC_BUCKET_MS;

        tracker.observe_server_time_sync(t as i64 - 50, t as i64, t); // candidate=-50
        assert_eq!(tracker.server_clock_offset_ms(t), Some(-50));

        // より小さい候補(-100)が同一バケット内で来ても、max により -50 のまま
        let t2 = t + 1_000;
        tracker.observe_server_time_sync(t2 as i64 - 100, t2 as i64, t2); // candidate=-100
        assert_eq!(tracker.server_clock_offset_ms(t2), Some(-50));

        // より大きい候補(-10)が来れば更新される（max なので採用）
        let t3 = t2 + 1_000;
        tracker.observe_server_time_sync(t3 as i64 - 10, t3 as i64, t3); // candidate=-10
        assert_eq!(tracker.server_clock_offset_ms(t3), Some(-10));
    }

    // 2バケット以上の空白の後に新しい候補が届いても、リセット直前の推定値が prev へ
    // 繰り越されるため即座には上書きされない。さらに1バケット進んで同じ新しい候補が
    // 観測され続ければ、繰り越した旧推定が prev から自然に抜け、新しい値へ切り替わる
    // （heuristic_min のバケット expiry テストと対称の仕組み。sync_max では「より
    // 大きい古い値」から「より小さい持続的な新しい値」へ、1バケット遅れて再学習する
    // 例で示す）。
    #[test]
    fn test_server_clock_offset_sync_max_relearns_one_bucket_after_gap() {
        let mut tracker = BuffTracker::new();

        let now1: u128 = 10 * SYNC_BUCKET_MS;
        tracker.observe_server_time_sync(now1 as i64 - 50, now1 as i64, now1); // candidate=-50
        assert_eq!(tracker.server_clock_offset_ms(now1), Some(-50));

        // 3バケット分の空白（2バケット以上）の後、より小さい候補(-1346)が届く
        let now2 = now1 + 3 * SYNC_BUCKET_MS;
        tracker.observe_server_time_sync(now2 as i64 - 1346, now2 as i64, now2); // candidate=-1346
        // リセット直後は旧推定(-50)が prev へ繰り越されているため、
        // max(-1346, -50) = -50 のまま（即座には切り替わらない）。
        assert_eq!(tracker.server_clock_offset_ms(now2), Some(-50));

        // さらに1バケット進んで、同じ候補(-1346)が観測され続ける
        let now3 = now2 + SYNC_BUCKET_MS;
        tracker.observe_server_time_sync(now3 as i64 - 1346, now3 as i64, now3); // candidate=-1346
        // 旧推定(-50)は繰り越し後さらに1バケット経過して prev から抜けたため、
        // 新しい値へ切り替わる。
        assert_eq!(tracker.server_clock_offset_ms(now3), Some(-1346));
    }

    // sync 標本の妥当性ガード: server がローカル±24h の窓外（別単位・誤再組み立て）、または
    // client-server が ±24h を超える標本は採用しない（sync_max は 60 分間ヒューリスティックより
    // 優先されるため、1 標本の異常値が全バフの期限を数十年ずらすのを防ぐ）。
    #[test]
    fn test_server_clock_offset_sync_rejects_implausible_samples() {
        let mut tracker = BuffTracker::new();
        let now: u128 = 1_787_500_000_000; // 実エポックms（2026-08）。差の閾値(24h)より十分大きい

        // server が µs 相当（ローカルの 1000 倍）→ 窓外で無視
        tracker.observe_server_time_sync(now as i64, now as i64 * 1000, now);
        assert_eq!(tracker.server_clock_offset_ms(now), None);
        // client が 0 相当の巨大な差（processor のガードを抜けた想定）→ 無視
        tracker.observe_server_time_sync(1, now as i64, now);
        assert_eq!(tracker.server_clock_offset_ms(now), None);
        // 妥当な標本は採用される
        tracker.observe_server_time_sync(now as i64 - 78, now as i64, now);
        assert_eq!(tracker.server_clock_offset_ms(now), Some(-78));
    }

    // ローカル時計が 1 バケット未満の幅で後方へステップした場合（w32time 補正等）、ステップ前の
    // 「大きすぎる」標本は cur→prev と残るが、sync のバケット幅(5分)なら最長 2 バケットで抜ける。
    #[test]
    fn test_server_clock_offset_sync_max_recovers_from_backward_clock_step_within_two_buckets() {
        let mut tracker = BuffTracker::new();
        let t0: u128 = 10 * SYNC_BUCKET_MS;
        tracker.observe_server_time_sync(t0 as i64 - 78, t0 as i64, t0); // 真値 -78
        assert_eq!(tracker.server_clock_offset_ms(t0), Some(-78));

        // 同一バケット内でローカル時計が 30 秒戻る → 以後の標本は -30078
        let mut t = t0 + 10_000;
        while t < t0 + 2 * SYNC_BUCKET_MS + 10_000 {
            tracker.observe_server_time_sync(t as i64 - 30_078, t as i64, t);
            t += 5_000;
        }
        // 2 バケット進んだ時点で旧値(-78)は prev からも抜け、新しい時計差へ再学習済み
        assert_eq!(tracker.server_clock_offset_ms(t), Some(-30_078));
    }
}
