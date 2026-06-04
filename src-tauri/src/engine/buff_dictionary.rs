use std::collections::HashMap;
use std::sync::LazyLock;

/// バフ/デバフの種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuffCategory {
    Buff,
    Debuff,
    Recovery,
    Item,
}

impl BuffCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buff => "buff",
            Self::Debuff => "debuff",
            Self::Recovery => "recovery",
            Self::Item => "item",
        }
    }
}

/// HUD 表示優先度 (BuffTable.BuffPriority に対応)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DisplayPriority {
    Hidden = 0, // 非表示（フィルタ対象）
    Low = 1,
    Normal = 2,
    High = 3,
    Alert = 4,
}

pub struct BuffMeta {
    pub category: BuffCategory,
    pub priority: DisplayPriority,
}

impl BuffMeta {
    const fn new(category: BuffCategory, priority: DisplayPriority) -> Self {
        Self { category, priority }
    }
}

// BuffType 対応: Debuff=0, Buff=1, Recovery=2, Item=3
// BuffPriority 対応: NotShow=0 → Hidden, Secondly=1 → Low, Highest=2 → Normal, Notice=3 → High
// 未登録 base_id は「未知バフ」として表示する（Hidden 扱いにしない）

static DICT: LazyLock<HashMap<i32, BuffMeta>> = LazyLock::new(|| {
    use BuffCategory::{Buff, Debuff, Recovery};
    use DisplayPriority::{Alert, Hidden, High, Low, Normal};

    [
        // ── 汎用デバフ ──────────────────────────────────────
        (4501,    BuffMeta::new(Debuff, High)),    // 燃焼
        (21416,   BuffMeta::new(Debuff, Normal)),  // スロウ
        (25201,   BuffMeta::new(Debuff, Normal)),  // 霜寒
        (25202,   BuffMeta::new(Debuff, Normal)),  // 減速
        (25205,   BuffMeta::new(Debuff, High)),    // 凍結
        (25206,   BuffMeta::new(Debuff, High)),    // 凍結 (別ID)
        (32201,   BuffMeta::new(Debuff, Normal)),  // 重傷
        (44501,   BuffMeta::new(Debuff, Normal)),  // 重傷 (別ID)
        (55239,   BuffMeta::new(Debuff, Low)),     // 閃心停滞
        (55412,   BuffMeta::new(Debuff, Hidden)),  // コンクエスト (内部フラグ、非表示)
        (55417,   BuffMeta::new(Debuff, Hidden)),  // コンクエスト (別ID、非表示)
        (55425,   BuffMeta::new(Debuff, Normal)),  // 目くらまし
        (55426,   BuffMeta::new(Debuff, Normal)),  // 沈黙

        // ── イマジン由来デバフ ──────────────────────────────
        (2100008, BuffMeta::new(Debuff, Normal)),  // 停滞
        (2100201, BuffMeta::new(Debuff, High)),    // スタン
        (2110026, BuffMeta::new(Debuff, Normal)),  // 重傷
        (2110050, BuffMeta::new(Debuff, Normal)),  // バジリスク: 重複使用無効デバフ
        (2110055, BuffMeta::new(Debuff, Normal)),  // タータ: 疲労・烈火の焼身
        (2110056, BuffMeta::new(Debuff, Normal)),  // ティナ: 時間凝固
        (2110057, BuffMeta::new(Debuff, Normal)),  // アルーナ: 虚弱・祈り禁止
        (2110069, BuffMeta::new(Debuff, Normal)),  // 破傷風
        (2110070, BuffMeta::new(Debuff, Normal)),  // ドレッドロア
        (2110078, BuffMeta::new(Debuff, Normal)),  // ショック防御破壊
        (2110100, BuffMeta::new(Debuff, Normal)),  // 回復禁止
        (2110111, BuffMeta::new(Debuff, Normal)),  // 呪術
        (2110099, BuffMeta::new(Debuff, Hidden)),  // 気刃突刺計数 (内部カウンタ、非表示)

        // ── 回復・防御バフ ───────────────────────────────────
        (21421,   BuffMeta::new(Recovery, High)),  // ライフサージ
        (21422,   BuffMeta::new(Buff, High)),      // バリア
        (21423,   BuffMeta::new(Buff, High)),      // 共生の印
        (50024,   BuffMeta::new(Buff, High)),      // バリア (別ID)
        (50040,   BuffMeta::new(Buff, High)),      // 挑発&無敵
        (50042,   BuffMeta::new(Buff, High)),      // 剛体
        (55402,   BuffMeta::new(Buff, High)),      // 物理防御力アップ
        (55403,   BuffMeta::new(Buff, High)),      // 神聖なる接触
        (55405,   BuffMeta::new(Buff, High)),      // ホーリーガード
        (55413,   BuffMeta::new(Buff, High)),      // セイクリッドオース
        (55414,   BuffMeta::new(Buff, High)),      // セイクリッドオース (別ID)
        (55416,   BuffMeta::new(Buff, High)),      // 光盾障壁
        (55422,   BuffMeta::new(Buff, High)),      // レジストブースト
        (500113,  BuffMeta::new(Buff, Normal)),    // 無敵復活

        // ── 攻撃・強化バフ ───────────────────────────────────
        (21404,   BuffMeta::new(Buff, High)),      // HP持続回復
        (21412,   BuffMeta::new(Buff, High)),      // ファストグロウス
        (27002,   BuffMeta::new(Buff, High)),      // 玄氷
        (27007,   BuffMeta::new(Buff, Normal)),    // パーマフロスト
        (27012,   BuffMeta::new(Buff, High)),      // 高速詠唱
        (30003,   BuffMeta::new(Buff, High)),      // 鋭利リセットカウントダウン
        (30501,   BuffMeta::new(Buff, High)),      // 激励
        (31201,   BuffMeta::new(Buff, Normal)),    // 風姿卓絶
        (31301,   BuffMeta::new(Buff, High)),      // 護風
        (31602,   BuffMeta::new(Buff, High)),      // 激励 (別ID)
        (43101,   BuffMeta::new(Buff, Normal)),    // 無限の雷霆
        (43201,   BuffMeta::new(Buff, Normal)),    // 千雷
        (45108,   BuffMeta::new(Buff, High)),      // エネルギー充填
        (55221,   BuffMeta::new(Buff, High)),      // 閃心強化
        (55223,   BuffMeta::new(Buff, Normal)),    // メンタルフォーカス
        (55330,   BuffMeta::new(Buff, Alert)),     // バイオレントスイング
        (55333,   BuffMeta::new(Buff, High)),      // アンコール
        (510011,  BuffMeta::new(Buff, High)),      // 会心アップ
        (510012,  BuffMeta::new(Buff, Normal)),    // 会心アップ (別ID)
        (510021,  BuffMeta::new(Buff, High)),      // 幸運アップ
        (501706,  BuffMeta::new(Buff, Normal)),    // 力の封印
        (501707,  BuffMeta::new(Buff, Normal)),    // 力の解放
        (501710,  BuffMeta::new(Buff, Normal)),    // 力の封印 (別ID)
        (501712,  BuffMeta::new(Buff, Normal)),    // 力の封印 (別ID)

        // ── イマジン由来バフ ────────────────────────────────
        (2100002, BuffMeta::new(Buff, High)),      // スターフォール
        (2100106, BuffMeta::new(Buff, High)),      // 激戦のオーラ
        (2100151, BuffMeta::new(Buff, High)),      // 心眼
        (2100152, BuffMeta::new(Buff, High)),      // 奮起
        (2100154, BuffMeta::new(Buff, Normal)),    // 祝福
        (2100203, BuffMeta::new(Buff, Normal)),    // バトルロア
        (2100211, BuffMeta::new(Buff, High)),      // 激情
        (2110024, BuffMeta::new(Buff, High)),      // 超会心
        (2110034, BuffMeta::new(Buff, High)),      // リキャスト短縮
        (2110042, BuffMeta::new(Buff, High)),      // ファスト
        (2110053, BuffMeta::new(Buff, High)),      // 器用さアップ
        (2110060, BuffMeta::new(Buff, Normal)),    // スウィフトスワール
        (2110061, BuffMeta::new(Buff, Normal)),    // フレイムハート
        (2110062, BuffMeta::new(Buff, Normal)),    // ロアコマンド
        (2110065, BuffMeta::new(Buff, Normal)),    // 灼熱の戦意
        (2110066, BuffMeta::new(Buff, Normal)),    // アースバリア
        (2110068, BuffMeta::new(Buff, Normal)),    // プロテクトフィールド
        (2110075, BuffMeta::new(Buff, High)),      // 羽化
        (2110077, BuffMeta::new(Buff, Normal)),    // 威圧
        (2110084, BuffMeta::new(Buff, Normal)),    // 琉火の盾
        (2110095, BuffMeta::new(Buff, High)),      // 会心強化
        (2110096, BuffMeta::new(Buff, High)),      // サンダーボールバリア
        (2110101, BuffMeta::new(Buff, High)),      // 変異ミーン
        (2110102, BuffMeta::new(Buff, High)),      // 幸運強化
        (2110103, BuffMeta::new(Buff, High)),      // ハウリングウェーブ
        (2110107, BuffMeta::new(Buff, High)),      // 特殊攻撃強化
        (2110108, BuffMeta::new(Buff, High)),      // レジスト強化
        (2110109, BuffMeta::new(Buff, High)),      // 幸運のゴブリン
        (2110110, BuffMeta::new(Buff, High)),      // 魚人の幸運
        (2200151, BuffMeta::new(Buff, Normal)),    // 刃域
        (2201311, BuffMeta::new(Buff, Alert)),     // 憤怒
        (2201511, BuffMeta::new(Buff, Alert)),     // 岩身
        (2201611, BuffMeta::new(Buff, Alert)),     // 岩心
        (2202751, BuffMeta::new(Buff, Alert)),     // 瞬間開花
        (2204371, BuffMeta::new(Buff, Normal)),    // 無尽極寒
        (2204551, BuffMeta::new(Buff, Alert)),     // 湧力法則
        (2205211, BuffMeta::new(Buff, Normal)),    // 追撃身法
        (2206031, BuffMeta::new(Buff, Normal)),    // 光力の恩惠
        (2206551, BuffMeta::new(Buff, Alert)),     // 光核
    ]
    .into_iter()
    .collect()
});

pub fn lookup(base_id: i32) -> Option<&'static BuffMeta> {
    DICT.get(&base_id)
}

/// 表示対象かどうか。Hidden なら false。未登録なら true (不明バフとして表示)。
pub fn is_visible(base_id: i32) -> bool {
    match DICT.get(&base_id) {
        Some(meta) => meta.priority != DisplayPriority::Hidden,
        None => true,
    }
}
