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
        (55412,   BuffMeta::new(Buff,   Normal)),  // コンクエスト Zeal Crusade Lv12 (自オーラ。BuffTable準拠でバフ表示)
        (55417,   BuffMeta::new(Buff,   Normal)),  // コンクエスト Zeal Crusade Lv17 (自オーラ。BuffTable準拠でバフ表示)
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
        (2110111, BuffMeta::new(Buff,   High)),    // 付与 Enchantment (BuffTable: Buff/High)
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
        (500113,  BuffMeta::new(Buff, Low)),       // 無敵復活 Revive invincibility (BuffTable: Low)

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
        (2206540, BuffMeta::new(Buff,     Hidden)), // 剛勇無畏 (永続の内部保持バフ・非表示。表示は2206542)
        (2206542, BuffMeta::new(Buff,     High)),   // 剛勇無畏 (10秒タイマー＋スタック。これを表示)

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
        (2201271, BuffMeta::new(Buff,     High)),   // 絶境逢生 Survival Instinct (BuffTable: High)
        (2201361, BuffMeta::new(Buff,     Normal)), // レジスト意識
        (2201461, BuffMeta::new(Buff,     Normal)), // 剛身強化
        (2201491, BuffMeta::new(Recovery, Normal)), // 回復
        (2201651, BuffMeta::new(Buff,     Normal)), // 堅固な壁

        // ── 風騎士 / ウィンドナイト ──────────────────────────────
        (2203161, BuffMeta::new(Buff,     High)),   // 迅速
        (2205121, BuffMeta::new(Buff,     Normal)), // 追風逐影 Wind Chaser (BuffTable: Normal)
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

        // ── S3 追加分（BPSR-ZDPS BuffTable の S2→S3 差分から機械生成: BuffType→category,
        //    BuffPriority→priority。名前は公式 loc 突合の JA、無ければ EN） ──────────
        (510544, BuffMeta::new(Debuff, Alert)), // 静止
        (683118, BuffMeta::new(Buff, High)), // 団結の力
        (683331, BuffMeta::new(Buff, High)), // 団結の力
        (683858, BuffMeta::new(Buff, High)), // 贈るッポ！
        (683859, BuffMeta::new(Buff, High)), // 贈るッポ！
        (683861, BuffMeta::new(Buff, High)), // ナイトワルツ
        (702003, BuffMeta::new(Buff, Normal)), // 激高の楽章
        (702004, BuffMeta::new(Buff, Normal)), // 激高の楽章
        (702005, BuffMeta::new(Buff, Normal)), // 激高の楽章
        (702015, BuffMeta::new(Buff, Normal)), // 不調和結晶
        (705104, BuffMeta::new(Debuff, Normal)), // 威嚇の怒号
        (705105, BuffMeta::new(Buff, Normal)), // 威嚇の怒号
        (821060, BuffMeta::new(Buff, High)), // 幻花晶の庇護
        (821074, BuffMeta::new(Debuff, High)), // 幻覚
        (821078, BuffMeta::new(Debuff, High)), // 幻覚の侵蝕
        (829121, BuffMeta::new(Debuff, High)), // 弱体化
        (829124, BuffMeta::new(Debuff, Normal)), // Bleed
        (829129, BuffMeta::new(Debuff, High)), // シンギュラリティの傷痕
        (829130, BuffMeta::new(Debuff, High)), // 傷口の残留
        (829201, BuffMeta::new(Debuff, Alert)), // 時間延長レールガン
        (829210, BuffMeta::new(Debuff, Alert)), // 巨大時間延長レールガン
        (829224, BuffMeta::new(Debuff, High)), // 機械の契約
        (829243, BuffMeta::new(Debuff, Low)), // エングラム制限
        (829265, BuffMeta::new(Debuff, High)), // Slow
        (829324, BuffMeta::new(Debuff, Alert)), // 累刑宣告
        (829352, BuffMeta::new(Buff, Low)), // 悖逆の機骸・始の弱点研究-ノーマル
        (829353, BuffMeta::new(Buff, Low)), // 悖逆の機骸・始の弱点研究-ハード
        (829354, BuffMeta::new(Buff, Low)), // 悖逆の機骸・始の弱点研究-ナイトメア
        (829355, BuffMeta::new(Buff, Low)), // 悖逆の機骸・継の弱点研究-ノーマル
        (829356, BuffMeta::new(Buff, Low)), // 悖逆の機骸・継の弱点研究-ハード
        (829357, BuffMeta::new(Buff, Low)), // 悖逆の機骸・継の弱点研究-ナイトメア
        (829358, BuffMeta::new(Buff, Low)), // 悖逆の機骸・終の弱点研究-ノーマル
        (829359, BuffMeta::new(Buff, Low)), // 悖逆の機骸・終の弱点研究-ハード
        (829360, BuffMeta::new(Buff, Low)), // 悖逆の機骸・終の弱点研究-ナイトメア
        (829370, BuffMeta::new(Buff, High)), // 連戦の決意
        (829372, BuffMeta::new(Debuff, Alert)), // マーク1
        (829373, BuffMeta::new(Debuff, Alert)), // マーク2
        (829374, BuffMeta::new(Debuff, Alert)), // マーク3
        (829375, BuffMeta::new(Debuff, Alert)), // マーク4
        (841519, BuffMeta::new(Debuff, High)), // 虚蝕束縛
        (841520, BuffMeta::new(Buff, High)), // 怒り
        (841522, BuffMeta::new(Debuff, High)), // 被抑圧
        (841526, BuffMeta::new(Debuff, High)), // 虚蝕裂傷
        (842304, BuffMeta::new(Buff, High)), // 決意
        (873605, BuffMeta::new(Debuff, High)), // イタズラ爆弾
        (873607, BuffMeta::new(Debuff, High)), // 虚蝕の残影
        (873610, BuffMeta::new(Debuff, Normal)), // 蝕体の刻印
        (873618, BuffMeta::new(Debuff, High)), // 衰弱
        (873631, BuffMeta::new(Debuff, Normal)), // 重傷
        (883315, BuffMeta::new(Buff, Normal)), // オーバーヒート
        (883342, BuffMeta::new(Debuff, Normal)), // 炎ダメージ悪化
        (883344, BuffMeta::new(Debuff, High)), // Slow
        (883373, BuffMeta::new(Debuff, High)), // 重荷状態
        (883399, BuffMeta::new(Debuff, Normal)), // 灼熱の煉獄
        (883602, BuffMeta::new(Buff, Alert)), // 双響・氷
        (883603, BuffMeta::new(Buff, Alert)), // 双響・水
        (883615, BuffMeta::new(Debuff, High)), // ナッポのイタズラ
        (883650, BuffMeta::new(Debuff, High)), // 破綻の音律
        (883702, BuffMeta::new(Debuff, High)), // 導電
        (883705, BuffMeta::new(Debuff, Normal)), // 縛糸
        (883721, BuffMeta::new(Buff, Low)), // 潮汐の加護
        (883803, BuffMeta::new(Debuff, High)), // 裂傷
        (883804, BuffMeta::new(Debuff, High)), // あばら骨折
        (883805, BuffMeta::new(Debuff, High)), // 呼吸困難
        (883811, BuffMeta::new(Debuff, High)), // ひっかき傷
        (883819, BuffMeta::new(Debuff, High)), // 気流交錯
        (883826, BuffMeta::new(Debuff, High)), // 風隠の庇護
        (883830, BuffMeta::new(Debuff, High)), // 乱気流
        (883831, BuffMeta::new(Debuff, High)), // 裂傷
        (883833, BuffMeta::new(Buff, Low)), // 加速
        (884114, BuffMeta::new(Debuff, High)), // プロトコル崩壊
        (884140, BuffMeta::new(Debuff, Alert)), // 錯乱の烙印
        (972049, BuffMeta::new(Buff, High)), // ナツ・火の契約
        (972050, BuffMeta::new(Buff, High)), // ルーシィ・星霊契約
        (974318, BuffMeta::new(Buff, Low)), // 滅竜の力
        (974322, BuffMeta::new(Debuff, Normal)), // 落石
        (974342, BuffMeta::new(Buff, Normal)), // 毒霧拡散
        (974343, BuffMeta::new(Debuff, Normal)), // 滅竜の力
        (974344, BuffMeta::new(Debuff, Normal)), // 滅竜の力
        (974345, BuffMeta::new(Debuff, High)), // 汚染
        (974346, BuffMeta::new(Debuff, High)), // 汚染
        (990204, BuffMeta::new(Debuff, High)), // Frozen
        (995179, BuffMeta::new(Debuff, High)), // スタン
        (995186, BuffMeta::new(Debuff, Normal)), // Lightning Strike
        (995187, BuffMeta::new(Debuff, Normal)), // 風の守護
        (995191, BuffMeta::new(Buff, Normal)), // 力の封印
        (997330, BuffMeta::new(Buff, Normal)), // 巨像の余威
        (997382, BuffMeta::new(Debuff, Normal)), // スタン
        (997397, BuffMeta::new(Debuff, High)), // 迷妄状態
        (2032211, BuffMeta::new(Buff, Low)), // 料理
        (2032212, BuffMeta::new(Buff, Low)), // 料理
        (2032213, BuffMeta::new(Buff, Low)), // 料理
        (2032214, BuffMeta::new(Buff, Low)), // 料理
        (2032215, BuffMeta::new(Buff, Low)), // 料理
        (2032216, BuffMeta::new(Buff, Low)), // 料理
        (2032221, BuffMeta::new(Buff, Low)), // 料理
        (2032222, BuffMeta::new(Buff, Low)), // 料理
        (2032223, BuffMeta::new(Buff, Low)), // 料理
        (2032224, BuffMeta::new(Buff, Low)), // 料理
        (2032225, BuffMeta::new(Buff, Low)), // 料理
        (2032226, BuffMeta::new(Buff, Low)), // 料理
        (2032231, BuffMeta::new(Buff, Low)), // 料理
        (2032232, BuffMeta::new(Buff, Low)), // 料理
        (2032233, BuffMeta::new(Buff, Low)), // 料理
        (2032234, BuffMeta::new(Buff, Low)), // 料理
        (2032235, BuffMeta::new(Buff, Low)), // 料理
        (2032236, BuffMeta::new(Buff, Low)), // 料理
        (2032241, BuffMeta::new(Buff, Low)), // 料理
        (2032242, BuffMeta::new(Buff, Low)), // 料理
        (2032243, BuffMeta::new(Buff, Low)), // 料理
        (2032244, BuffMeta::new(Buff, Low)), // 料理
        (2032245, BuffMeta::new(Buff, Low)), // 料理
        (2032246, BuffMeta::new(Buff, Low)), // 料理
        (2032251, BuffMeta::new(Buff, Low)), // 料理
        (2032252, BuffMeta::new(Buff, Low)), // 料理
        (2032253, BuffMeta::new(Buff, Low)), // 料理
        (2032254, BuffMeta::new(Buff, Low)), // 料理
        (2032255, BuffMeta::new(Buff, Low)), // 料理
        (2032256, BuffMeta::new(Buff, Low)), // 料理
        (2032261, BuffMeta::new(Buff, Low)), // 料理
        (2032262, BuffMeta::new(Buff, Low)), // 料理
        (2032263, BuffMeta::new(Buff, Low)), // 料理
        (2032264, BuffMeta::new(Buff, Low)), // 料理
        (2032271, BuffMeta::new(Buff, Low)), // 料理
        (2032272, BuffMeta::new(Buff, Low)), // 料理
        (2032273, BuffMeta::new(Buff, Low)), // 料理
        (2032274, BuffMeta::new(Buff, Low)), // 料理
        (2032281, BuffMeta::new(Buff, Low)), // 料理
        (2032282, BuffMeta::new(Buff, Low)), // 料理
        (2032283, BuffMeta::new(Buff, Low)), // 料理
        (2032284, BuffMeta::new(Buff, Low)), // 料理
        (2033017, BuffMeta::new(Buff, Low)), // 薬剤
        (2033018, BuffMeta::new(Buff, Low)), // 薬剤
        (2033019, BuffMeta::new(Buff, Low)), // 薬剤
        (2033027, BuffMeta::new(Buff, Low)), // 薬剤
        (2033028, BuffMeta::new(Buff, Low)), // 薬剤
        (2033029, BuffMeta::new(Buff, Low)), // 薬剤
        (2033037, BuffMeta::new(Buff, Low)), // 薬剤
        (2033038, BuffMeta::new(Buff, Low)), // 薬剤
        (2033039, BuffMeta::new(Buff, Low)), // 薬剤
        (2033047, BuffMeta::new(Buff, Low)), // 薬剤
        (2033048, BuffMeta::new(Buff, Low)), // 薬剤
        (2033049, BuffMeta::new(Buff, Low)), // 薬剤
        (2033057, BuffMeta::new(Buff, Low)), // 薬剤
        (2033058, BuffMeta::new(Buff, Low)), // 薬剤
        (2033059, BuffMeta::new(Buff, Low)), // 薬剤
        (2033067, BuffMeta::new(Buff, Low)), // 薬剤
        (2033068, BuffMeta::new(Buff, Low)), // 薬剤
        (2033069, BuffMeta::new(Buff, Low)), // 薬剤
        (2033077, BuffMeta::new(Buff, Low)), // 薬剤
        (2033078, BuffMeta::new(Buff, Low)), // 薬剤
        (2033079, BuffMeta::new(Buff, Low)), // 薬剤
        (2033087, BuffMeta::new(Buff, Low)), // 薬剤
        (2033088, BuffMeta::new(Buff, Low)), // 薬剤
        (2033089, BuffMeta::new(Buff, Low)), // 薬剤
        (2033097, BuffMeta::new(Buff, Low)), // 薬剤
        (2033098, BuffMeta::new(Buff, Low)), // 薬剤
        (2033099, BuffMeta::new(Buff, Low)), // 薬剤
        (2033107, BuffMeta::new(Buff, Low)), // 薬剤
        (2033108, BuffMeta::new(Buff, Low)), // 薬剤
        (2033109, BuffMeta::new(Buff, Low)), // 薬剤
        (2033117, BuffMeta::new(Buff, Low)), // 薬剤
        (2033118, BuffMeta::new(Buff, Low)), // 薬剤
        (2033119, BuffMeta::new(Buff, Low)), // 薬剤
        (2033127, BuffMeta::new(Buff, Low)), // 薬剤
        (2033128, BuffMeta::new(Buff, Low)), // 薬剤
        (2033129, BuffMeta::new(Buff, Low)), // 薬剤
        (2033137, BuffMeta::new(Buff, Low)), // 薬剤
        (2033138, BuffMeta::new(Buff, Low)), // 薬剤
        (2033139, BuffMeta::new(Buff, Low)), // 薬剤
        (2033147, BuffMeta::new(Buff, Low)), // 薬剤
        (2033148, BuffMeta::new(Buff, Low)), // 薬剤
        (2033149, BuffMeta::new(Buff, Low)), // 薬剤
        (2033157, BuffMeta::new(Buff, Low)), // 薬剤
        (2033158, BuffMeta::new(Buff, Low)), // 薬剤
        (2033159, BuffMeta::new(Buff, Low)), // 薬剤
        (2033167, BuffMeta::new(Buff, Low)), // 薬剤
        (2033168, BuffMeta::new(Buff, Low)), // 薬剤
        (2033169, BuffMeta::new(Buff, Low)), // 薬剤
        (2033177, BuffMeta::new(Buff, Low)), // 薬剤
        (2033178, BuffMeta::new(Buff, Low)), // 薬剤
        (2033179, BuffMeta::new(Buff, Low)), // 薬剤
        (2033187, BuffMeta::new(Buff, Low)), // 薬剤
        (2033188, BuffMeta::new(Buff, Low)), // 薬剤
        (2033189, BuffMeta::new(Buff, Low)), // 薬剤
        (2110049, BuffMeta::new(Debuff, Normal)), // 機械の故障
        (2110137, BuffMeta::new(Buff, High)), // 虚蝕の力
        (2110138, BuffMeta::new(Buff, High)), // 風雷の力
        (2110139, BuffMeta::new(Buff, High)), // ナッポの祝福
        (2110140, BuffMeta::new(Buff, High)), // 機械の力
        (2110141, BuffMeta::new(Buff, High)), // 器用さの王
        (2110142, BuffMeta::new(Buff, High)), // 高速連斬
        (2110143, BuffMeta::new(Buff, High)), // 機能強化
        (2110144, BuffMeta::new(Buff, High)), // たくましき肉体
        (2110145, BuffMeta::new(Buff, High)), // 致命のコンボ
        (2110149, BuffMeta::new(Buff, High)), // トロールの力
        (2110161, BuffMeta::new(Buff, High)), // Celestial Spirit Mage!
        (2110162, BuffMeta::new(Buff, High)), // Fire Dragon Slayer!
        (2110164, BuffMeta::new(Buff, High)), // 竜の力
        (2110165, BuffMeta::new(Buff, High)), // 火竜の咆哮
        (2110167, BuffMeta::new(Debuff, Normal)), // 闘志薄弱
        (2110171, BuffMeta::new(Buff, High)), // ルーシィ・イミテーション
        (2110172, BuffMeta::new(Buff, High)), // ナツ・イミテーション
        (2110305, BuffMeta::new(Debuff, Normal)), // 回復禁止
        (2110306, BuffMeta::new(Buff, Normal)), // Electro Shield
        (2110307, BuffMeta::new(Buff, Normal)), // 聖盾防護
        (2208621, BuffMeta::new(Buff, High)), // Flame Dance
        (2208651, BuffMeta::new(Buff, High)), // 炎舞血脈
        (2409071, BuffMeta::new(Buff, Normal)), // 激怒全開
        (2409081, BuffMeta::new(Buff, Normal)), // 双炎型
        (3050441, BuffMeta::new(Buff, High)), // Lucky Moonblade
        (3051431, BuffMeta::new(Buff, High)), // 特殊攻撃強化
        (3053411, BuffMeta::new(Buff, High)), // Photon Energy Enhancement
        (3053412, BuffMeta::new(Buff, High)), // Photon Energy Enhancement
        (3056411, BuffMeta::new(Buff, High)), // 蒼月の慰め
        (3057411, BuffMeta::new(Buff, High)), // Harmonic Anthem Enhancement
        (3060141, BuffMeta::new(Buff, Normal)), // Crimson Lotus Awakening
        (3060191, BuffMeta::new(Buff, Normal)), // Explosive Strike
        (3060241, BuffMeta::new(Buff, Normal)), // Blazing Element
        (3060401, BuffMeta::new(Buff, Normal)), // Flame Soul Recovery
        (3060411, BuffMeta::new(Buff, High)), // Axe Wind Hunt
        (3060421, BuffMeta::new(Buff, High)), // Formless Illusion
        (3060441, BuffMeta::new(Buff, High)), // Peerless Flame Dance
        (3210181, BuffMeta::new(Buff, High)), // Wind Thunder Combo
        (3210191, BuffMeta::new(Buff, High)), // Arachnocrab Power
        (3210201, BuffMeta::new(Buff, High)), // All-Out Assault
        (3210202, BuffMeta::new(Buff, Normal)), // Elemental Smackdown
        (3210211, BuffMeta::new(Buff, Normal)), // Eye Power
        (3210221, BuffMeta::new(Buff, High)), // Invigorated
        (3210222, BuffMeta::new(Buff, High)), // 絶境逢生
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
