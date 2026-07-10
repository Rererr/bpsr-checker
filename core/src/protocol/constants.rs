pub const SERVICE_UUID: u64 = 0x63335342;
pub const SOCIAL_NTF_SERVICE_ID: u64 = 0x254C89A3;
pub const SOCIAL_NTF_NOTIFY_METHOD_ID: u32 = 1;

pub mod packet {
    pub const COMPRESSION_FLAG: u16 = 0x8000;
    pub const TYPE_MASK: u16 = 0x7FFF;

    #[inline]
    pub fn extract_type(packet_type: u16) -> u16 {
        packet_type & TYPE_MASK
    }
}

pub mod packet_layout {
    pub const SERVER_SIGNATURE_OFFSET: usize = 5;
}

pub mod entity {
    pub const TYPE_MASK: u16 = 0xFFFF;

    #[inline]
    pub fn get_player_uid(uuid: i64) -> i64 {
        uuid >> 16
    }
}

pub mod server_detection {
    pub const SERVER_SIGNATURE: &[u8] = &[0x00, 0x63, 0x33, 0x53, 0x42, 0x00];
    pub const LOGIN_RETURN_SIGNATURE_1: &[u8] =
        &[0x00, 0x00, 0x00, 0x62, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01];
    pub const LOGIN_RETURN_SIGNATURE_2: &[u8] = &[0x00, 0x00, 0x00, 0x00, 0x0a, 0x4e];
    pub const LOGIN_RETURN_SIGNATURE_SIZE: usize = 0x62;
}

pub mod attr_type {
    pub const ATTR_NAME: i32 = 0x01;
    pub const ATTR_ID: i32 = 0x0A;
    pub const ATTR_HP: i32 = 0x2C2E;
    pub const ATTR_MAX_HP: i32 = 0x2C38;
    pub const ATTR_PROFESSION_ID: i32 = 0xDC;
    pub const ATTR_FIGHT_POINT: i32 = 0x272E;
    pub const ATTR_SEASON_LEVEL: i32 = 0x2756;
    pub const ATTR_SEASON_STRENGTH: i32 = 0x2CB0;
    pub const ATTR_POS: i32 = 0x34;
    // 装備中スキルの一覧（SkillLevelInfo の連結列）。装備していればイマジンの奥義/絶技スキルも
    // 載るため、発動に依存しない「漏れの無いイマジン表示」の一次情報候補。過去 probe(2026-07-03)は
    // 「スキルLvl差分のみ」と観測したが、それは delta 経路のみの観測だった可能性が高く、
    // EnterScene/AOI appear のフルスナップショットでは全装備スキルが載る想定（実機検証中）。
    pub const ATTR_SKILL_LEVEL_ID_LIST: i32 = 0x74; // 116
    // 装備データ（EquipNine{slot, equip_id} の連結列）。
    // イマジンのアイテム構成IDが載るかの実機検証用。
    pub const ATTR_EQUIP_DATA: i32 = 0xC8; // 200

    // 召喚エンティティ関連（バトルイマジン検知に使う。EAttrType の実機観測値準拠）。
    // 召喚は AttrSkillId=召喚元スキル(イマジンの正体)・AttrTopSummonerId=オーナー(プレイヤー) を持つ。
    pub const ATTR_SUMMONER_ID: i32 = 0x5A; // 90  直接の召喚主
    pub const ATTR_TOP_SUMMONER_ID: i32 = 0x5B; // 91  最上位の召喚主（＝プレイヤー）
    pub const ATTR_SKILL_ID: i32 = 0x64; // 100 現在スキル/召喚元スキルID
    pub const ATTR_SKILL_REMODEL_LEVEL: i32 = 0x79; // 121 スキル改造(ティア)レベル

    // 自キャラ戦闘ステータス（EnterScene の PlayerEnt.attrs から取得。2026-06-19 実機 probe +
    // ゲーム内ステータス画面ツールチップで全項目を値一致確認）。
    // 各 stat は {id, id+1, id+2}（合計/基礎/補正）の3連で届くため先頭 id（合計）を使う。
    // ％系は「表示%」の id を採用（ゲーム内パネル表示と一致。別途あるレーティング id は不採用）。
    // 整数系
    pub const ATTR_ATTACK_POWER: i32 = 0x2C42; // 11330 物理攻撃力 (AttrAttack)
    pub const ATTR_MAGIC_ATTACK: i32 = 0x2C4C; // 11340 魔法攻撃力 (AttrMAttack)
    pub const ATTR_DEFENSE_POWER: i32 = 0x2C56; // 11350 物理防御力 (AttrDefense)
    pub const ATTR_MAGIC_DEFENSE: i32 = 0x2C60; // 11360 魔法防御力 (AttrMDefense)
    pub const ATTR_ENDURANCE: i32 = 0x2B20; // 11040 耐久力
    pub const ATTR_STRENGTH: i32 = 0x2B02; // 11010 筋力
    pub const ATTR_INTELLIGENCE: i32 = 0x2B0C; // 11020 知力
    pub const ATTR_AGILITY: i32 = 0x2B16; // 11030 敏捷
    // 割合系（値/100 = %・「表示%(万分比)」の id を採用。別途あるレーティング id は不採用）
    pub const ATTR_CRIT: i32 = 0x2DBE; // 11710 会心 (AttrCrit, 2485=24.85%)
    pub const ATTR_ATTACK_SPEED: i32 = 0x2DC8; // 11720 攻撃速度 (AttrAttackSpeedPCT)
    pub const ATTR_CAST_SPEED: i32 = 0x2DD2; // 11730 詠唱速度 (AttrCastSpeedPCT)
    pub const ATTR_HASTE: i32 = 0x2E9A; // 11930 ファスト/急速 (AttrHastePct)
    pub const ATTR_LUCKY: i32 = 0x2E04; // 11780 幸運 (AttrLuckyStrikeProb, 633=6.33%)
    pub const ATTR_DEXTERITY: i32 = 0x2EA4; // 11940 器用さ (AttrMasteryPct=精通)
    pub const ATTR_VERSATILITY: i32 = 0x2EAE; // 11950 万能 (AttrVersatilityPct=全能)
    pub const ATTR_CRIT_DMG: i32 = 0x30DE; // 12510 会心ダメージ (AttrCritDamage, 5710=57.1%)
    pub const ATTR_RESIST: i32 = 0x30E8; // 12520 レジスト (AttrCritDamageRes=暴击伤害抵抗)
    pub const ATTR_LUCKY_DMG: i32 = 0x30F2; // 12530 幸運の一撃ダメージ倍率 (AttrLuckDamInc)
    // 能力スコア(46169)=ATTR_FIGHT_POINT(0x272E)・幻夢強度=ATTR_SEASON_STRENGTH(0x2CB0)。
    // 会心率/幸運率は命中データからの実測値を別途使う（[[self-stats-overlay-progress]]）。
}

pub mod damage {
    pub const CRIT_BIT: i32 = 0b00000001;
}
