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

    // 自キャラ戦闘ステータス（probe 実測 + ゲーム内パネル照合で確定）。
    // 各 stat は {id, id+1, id+2}（合計/基礎/補正）の3連で届くため先頭 id を使う。
    pub const ATTR_ATTACK_POWER: i32 = 0x32; // 攻撃力（整数）
    pub const ATTR_DEFENSE_POWER: i32 = 0x33; // 防御力（整数）
    pub const ATTR_ENDURANCE: i32 = 0x2B20; // 11040 耐久力（整数）
    pub const ATTR_DEXTERITY: i32 = 0x2B84; // 11140 器用さ（整数）
    pub const ATTR_ATTACK_SPEED: i32 = 0x2DC8; // 11720 攻撃速度（値/100 = %）
    pub const ATTR_HASTE: i32 = 0x2E9A; // 11930 ファスト/迅速（値/100 = %）
    pub const ATTR_LUCKY: i32 = 0x3188; // 12680 幸運（値/100 = %）
    // 会心率はパケットに送られないため、命中データからの実測値を別途使う。
}

pub mod damage {
    pub const CRIT_BIT: i32 = 0b00000001;
}
