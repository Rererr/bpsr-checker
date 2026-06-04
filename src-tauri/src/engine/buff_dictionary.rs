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

#[derive(Clone, Copy)]
pub struct BuffMeta {
    pub category: BuffCategory,
    pub priority: DisplayPriority,
}

impl BuffMeta {
    pub const fn new(category: BuffCategory, priority: DisplayPriority) -> Self {
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

        // ── 共通ステータス・属性球・イマジン本 ──────────────────
        (510022,  BuffMeta::new(Buff,     Normal)), // 幸運アップ
        (510031,  BuffMeta::new(Buff,     High)),   // ファストアップ (属性球)
        (510032,  BuffMeta::new(Buff,     Normal)), // ファストアップ (イマジン本)
        (510041,  BuffMeta::new(Buff,     Normal)), // 器用さアップ (属性球)
        (510042,  BuffMeta::new(Buff,     High)),   // 器用さアップ (イマジン本)
        (510045,  BuffMeta::new(Buff,     High)),   // 決意
        (510099,  BuffMeta::new(Buff,     Normal)), // マスターガーディアン
        (997020,  BuffMeta::new(Buff,     Normal)), // メインステータス+100
        (997021,  BuffMeta::new(Buff,     Normal)), // 防御+300
        (997022,  BuffMeta::new(Buff,     Normal)), // 会心+300
        (997023,  BuffMeta::new(Buff,     Normal)), // ファスト+300
        (997024,  BuffMeta::new(Buff,     Normal)), // 器用さ+300
        (997025,  BuffMeta::new(Buff,     Normal)), // 幸運+300

        // ── 汎用状態異常 A〜E類 ──────────────────────────────────
        (510501,  BuffMeta::new(Debuff,   High)),   // スタン
        (510511,  BuffMeta::new(Debuff,   High)),   // 凍結
        (510521,  BuffMeta::new(Debuff,   High)),   // 目くらまし
        (510541,  BuffMeta::new(Debuff,   Normal)), // 沈黙
        (510542,  BuffMeta::new(Debuff,   Normal)), // スロウ
        (510543,  BuffMeta::new(Debuff,   Normal)), // 静止
        (510571,  BuffMeta::new(Debuff,   Normal)), // 重傷

        // ── 光盾 (シールドファイター 光盾型) ────────────────────
        (2206011, BuffMeta::new(Buff,     Normal)), // 光明の盾
        (2206111, BuffMeta::new(Buff,     High)),   // 審判
        (2206381, BuffMeta::new(Buff,     Normal)), // 清算炎盾
        (2206421, BuffMeta::new(Buff,     Normal)), // 灼熱
        (2206451, BuffMeta::new(Buff,     Normal)), // 防御の極意
        (2206481, BuffMeta::new(Buff,     Normal)), // 神聖攻撃の極意
        (2206540, BuffMeta::new(Buff,     High)),   // 剛勇無畏 (報告対象)
        (2206542, BuffMeta::new(Buff,     High)),   // 剛勇無畏 (別ID)

        // ── ヘビーガード / 砕岩士 (岩盾型) ─────────────────────
        (50029,   BuffMeta::new(Buff,     High)),   // ロックコート
        (50046,   BuffMeta::new(Buff,     High)),   // 岩の加護
        (50050,   BuffMeta::new(Buff,     Normal)), // 護刃の衝撃
        (50052,   BuffMeta::new(Buff,     High)),   // 魔法バリア
        (50057,   BuffMeta::new(Buff,     High)),   // 巨岩躯体
        (50058,   BuffMeta::new(Buff,     High)),   // 勇壮砦壁
        (50062,   BuffMeta::new(Buff,     High)),   // 岩塊撃
        (2201021, BuffMeta::new(Buff,     Normal)), // 砕石の吸引
        (2201191, BuffMeta::new(Buff,     Normal)), // 砂岩の加護
        (2201201, BuffMeta::new(Buff,     Normal)), // 盾連撃
        (2201241, BuffMeta::new(Buff,     Normal)), // 盾残響
        (2201252, BuffMeta::new(Buff,     Normal)), // 破砕怒撃
        (2201271, BuffMeta::new(Buff,     Normal)), // 絶境逢生
        (2201361, BuffMeta::new(Buff,     Normal)), // レジスト意識
        (2201461, BuffMeta::new(Buff,     Normal)), // 剛身強化
        (2201491, BuffMeta::new(Recovery, Normal)), // 回復
        (2201651, BuffMeta::new(Buff,     Normal)), // 堅固な壁

        // ── 風騎士 / ウィンドナイト ──────────────────────────────
        (2203161, BuffMeta::new(Buff,     High)),   // 迅速
        (2205121, BuffMeta::new(Buff,     High)),   // 追風逐影
        (2205131, BuffMeta::new(Buff,     Normal)), // 風螺旋
        (2205161, BuffMeta::new(Buff,     High)),   // 風怒
        (2205261, BuffMeta::new(Buff,     High)),   // 破壊の風雷
        (2205481, BuffMeta::new(Buff,     High)),   // 風起

        // ── フロストレイ / 氷雷 ──────────────────────────────────
        (2204151, BuffMeta::new(Buff,     Normal)), // 連弾
        (2204171, BuffMeta::new(Buff,     Normal)), // 嵐の交会
        (2204191, BuffMeta::new(Buff,     High)),   // 氷河の怒濤
        (2204261, BuffMeta::new(Buff,     Normal)), // 静かなる氷

        // ── ヒーラー (フラワー/自然) ─────────────────────────────
        (2202081, BuffMeta::new(Buff,     Normal)), // 緑の爆発
        (2202131, BuffMeta::new(Buff,     Normal)), // 二重注入
        (2202142, BuffMeta::new(Recovery, Normal)), // ヒーリング
        (2202241, BuffMeta::new(Buff,     Normal)), // 自然の精
        (2202321, BuffMeta::new(Buff,     Normal)), // マジェスティック付与
        (2202322, BuffMeta::new(Buff,     Normal)), // マジェスティック保持
        (2202331, BuffMeta::new(Buff,     Normal)), // 花界昇華
        (2202441, BuffMeta::new(Buff,     Normal)), // 庇護
        (2202561, BuffMeta::new(Buff,     Normal)), // 波動残響
        (2202621, BuffMeta::new(Buff,     Normal)), // 生命開花
        (2202651, BuffMeta::new(Buff,     High)),   // 開花循環
        (2202671, BuffMeta::new(Buff,     High)),   // 無心

        // ── バード / ハーモニックアンセム ────────────────────────
        (55313,   BuffMeta::new(Buff,     Normal)), // ハーモニックアンセム2倍化
        (55327,   BuffMeta::new(Buff,     Normal)), // ラッシュパッション
        (2207461, BuffMeta::new(Buff,     High)),   // ハードディストーション増傷
        (2207462, BuffMeta::new(Buff,     High)),   // ハーモニックアンセム増傷
        (2207521, BuffMeta::new(Buff,     Normal)), // 壮大サウンドウェーブ

        // ── レンジャー / ガンスリンガー ──────────────────────────
        (2203231, BuffMeta::new(Buff,     Normal)), // 集中射撃
        (2203292, BuffMeta::new(Buff,     Normal)), // 閃心強化
        (2203371, BuffMeta::new(Buff,     Normal)), // 怒涛の臣獣
        (2203591, BuffMeta::new(Buff,     Normal)), // 鷹の目

        // ── 雷影 / サンダーストライカー ──────────────────────────
        (2200051, BuffMeta::new(Buff,     High)),   // 彼岸
        (2200111, BuffMeta::new(Buff,     High)),   // 雷燕の印
        (2200112, BuffMeta::new(Buff,     Normal)), // 雷燕
        (2200221, BuffMeta::new(Buff,     High)),   // 雷の力
        (2200241, BuffMeta::new(Buff,     Normal)), // 荒れ狂う光
        (2200342, BuffMeta::new(Buff,     High)),   // 強撃
        (2200401, BuffMeta::new(Buff,     Normal)), // 雷印刀意ダメージ軽減
        (2200602, BuffMeta::new(Buff,     Normal)), // 審罰鎌

        // ── シャーマン / ダークサイド ─────────────────────────────
        (2203031, BuffMeta::new(Debuff,   Normal)), // 傷呪の印
        (2203051, BuffMeta::new(Buff,     Normal)), // 霊能激流
        (2203061, BuffMeta::new(Buff,     Normal)), // 魔狼の咆哮
        (2205031, BuffMeta::new(Debuff,   Normal)), // 傷呪の印 (別ID)

        // ── ナイフダンサー / 格闘 ────────────────────────────────
        (2205081, BuffMeta::new(Buff,     Normal)), // 千瘡百孔
        (2205221, BuffMeta::new(Buff,     Normal)), // 戦闘熟練
        (2205241, BuffMeta::new(Buff,     Normal)), // 怒涛
        (2205371, BuffMeta::new(Buff,     Normal)), // 破追[強化]
        (2205391, BuffMeta::new(Buff,     Normal)), // 気勁加持
        (2205501, BuffMeta::new(Buff,     Normal)), // 螺旋爆炎
        (2205591, BuffMeta::new(Buff,     Normal)), // 追撃之力

        // ── 共通ボス系警告 ────────────────────────────────────────
        (881547,  BuffMeta::new(Debuff,   Alert)),  // 誅雷の烙印
        (881076,  BuffMeta::new(Debuff,   Alert)),  // 魂分裂寸前
        (881080,  BuffMeta::new(Debuff,   High)),   // 魂の夭折
        (800026,  BuffMeta::new(Debuff,   High)),   // 大型爆弾カウントダウン

        // ── ボス由来デバフ ────────────────────────────────────────
        (800010,  BuffMeta::new(Debuff,   Normal)), // ツインマキナの火種
        (802911,  BuffMeta::new(Debuff,   Normal)), // 脆弱
        (803057,  BuffMeta::new(Debuff,   Normal)), // 風属性脆弱
        (803063,  BuffMeta::new(Debuff,   Normal)), // ミーン毒
        (683311,  BuffMeta::new(Debuff,   High)),   // アーマーブレイク
        (821051,  BuffMeta::new(Debuff,   High)),   // 磁化傷

        // ── その他 ────────────────────────────────────────────────
        (683115,  BuffMeta::new(Buff,     High)),   // 団結の力
        (683624,  BuffMeta::new(Recovery, High)),   // HP持続回復
        (3200010, BuffMeta::new(Buff,     High)),   // 剣士マスタリー
        (3200018, BuffMeta::new(Buff,     High)),   // マインドバースト
        (3200023, BuffMeta::new(Buff,     High)),   // 閃心祝福
        (3200024, BuffMeta::new(Buff,     High)),   // 嵐の祝福
        (3210071, BuffMeta::new(Buff,     High)),   // スペシャリスト
        (3210111, BuffMeta::new(Buff,     Normal)), // 暗霧剣士のマスタリー
    ]
    .into_iter()
    .collect()
});

pub fn lookup(base_id: i32) -> Option<&'static BuffMeta> {
    DICT.get(&base_id)
}

/// 表示対象かどうか。Hidden または未登録なら false。
pub fn is_visible(base_id: i32) -> bool {
    match DICT.get(&base_id) {
        Some(meta) => meta.priority != DisplayPriority::Hidden,
        None => false,
    }
}
