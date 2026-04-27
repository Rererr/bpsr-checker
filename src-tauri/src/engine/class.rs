#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Class {
    Stormblade,
    FrostMage,
    WindKnight,
    VerdantOracle,
    HeavyGuardian,
    Marksman,
    ShieldKnight,
    BeatPerformer,
    Unimplemented,
    #[default]
    Unknown,
}

impl From<i32> for Class {
    fn from(class_id: i32) -> Self {
        match class_id {
            1 => Class::Stormblade,
            2 => Class::FrostMage,
            4 => Class::WindKnight,
            5 => Class::VerdantOracle,
            9 => Class::HeavyGuardian,
            11 => Class::Marksman,
            12 => Class::ShieldKnight,
            13 => Class::BeatPerformer,
            _ => Class::Unimplemented,
        }
    }
}

impl Class {
    pub fn name_en(self) -> &'static str {
        match self {
            Class::Stormblade => "Stormblade",
            Class::FrostMage => "Frost Mage",
            Class::WindKnight => "Wind Knight",
            Class::VerdantOracle => "Verdant Oracle",
            Class::HeavyGuardian => "Heavy Guardian",
            Class::Marksman => "Marksman",
            Class::ShieldKnight => "Shield Knight",
            Class::BeatPerformer => "Beat Performer",
            Class::Unknown => "Unknown Class",
            Class::Unimplemented => "Unimplemented Class",
        }
    }

    pub fn name_ja(self) -> &'static str {
        match self {
            Class::Stormblade => "ストームブレイド",
            Class::FrostMage => "フロストメイジ",
            Class::WindKnight => "ウィンドナイト",
            Class::VerdantOracle => "ヴァーダントオラクル",
            Class::HeavyGuardian => "ヘビーガーディアン",
            Class::Marksman => "マークスマン",
            Class::ShieldKnight => "シールドナイト",
            Class::BeatPerformer => "ビートパフォーマー",
            Class::Unknown => "不明クラス",
            Class::Unimplemented => "未実装クラス",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ClassSpec {
    // Stormblade
    Iaido,
    Moonstrike,
    // Frost Mage
    Icicle,
    Frostbeam,
    // Wind Knight
    Vanguard,
    Skyward,
    // Verdant Oracle
    Smite,
    Lifebind,
    // Heavy Guardian
    Earthfort,
    Block,
    // Marksman
    Wildpack,
    Falconry,
    // Shield Knight
    Recovery,
    Shield,
    // Beat Performer
    Dissonance,
    Concerto,
    #[default]
    Unknown,
}

impl ClassSpec {
    pub fn name(self) -> &'static str {
        match self {
            ClassSpec::Iaido => "Iaido",
            ClassSpec::Moonstrike => "Moonstrike",
            ClassSpec::Icicle => "Icicle",
            ClassSpec::Frostbeam => "Frostbeam",
            ClassSpec::Vanguard => "Vanguard",
            ClassSpec::Skyward => "Skyward",
            ClassSpec::Smite => "Smite",
            ClassSpec::Lifebind => "Lifebind",
            ClassSpec::Earthfort => "Earthfort",
            ClassSpec::Block => "Block",
            ClassSpec::Wildpack => "Wildpack",
            ClassSpec::Falconry => "Falconry",
            ClassSpec::Recovery => "Recovery",
            ClassSpec::Shield => "Shield",
            ClassSpec::Dissonance => "Dissonance",
            ClassSpec::Concerto => "Concerto",
            ClassSpec::Unknown => "Unknown Spec",
        }
    }

    pub fn name_ja(self) -> &'static str {
        match self {
            ClassSpec::Iaido => "居合",
            ClassSpec::Moonstrike => "月閃",
            ClassSpec::Icicle => "氷柱",
            ClassSpec::Frostbeam => "氷光線",
            ClassSpec::Vanguard => "先鋒",
            ClassSpec::Skyward => "翔空",
            ClassSpec::Smite => "裁き",
            ClassSpec::Lifebind => "生命結",
            ClassSpec::Earthfort => "大地砦",
            ClassSpec::Block => "防御",
            ClassSpec::Wildpack => "狼群",
            ClassSpec::Falconry => "鷹匠",
            ClassSpec::Recovery => "回復",
            ClassSpec::Shield => "盾",
            ClassSpec::Dissonance => "不協和音",
            ClassSpec::Concerto => "協奏曲",
            ClassSpec::Unknown => "不明スペック",
        }
    }
}

pub fn get_class_spec_from_skill_id(skill_id: i32) -> ClassSpec {
    match skill_id {
        1714 | 1734 => ClassSpec::Iaido,
        44701 | 179906 => ClassSpec::Moonstrike,
        120901 | 120902 => ClassSpec::Icicle,
        1241 => ClassSpec::Frostbeam,
        1405 | 1418 => ClassSpec::Vanguard,
        1419 => ClassSpec::Skyward,
        1518 | 1541 | 21402 => ClassSpec::Smite,
        20301 => ClassSpec::Lifebind,
        199902 => ClassSpec::Earthfort,
        1930 | 1931 | 1934 | 1935 => ClassSpec::Block,
        220112 | 2203622 => ClassSpec::Falconry,
        2292 | 1700820 | 1700825 | 1700827 => ClassSpec::Wildpack,
        2405 => ClassSpec::Recovery,
        2406 => ClassSpec::Shield,
        2306 => ClassSpec::Dissonance,
        2307 | 2361 | 55302 => ClassSpec::Concerto,
        _ => ClassSpec::Unknown,
    }
}

pub fn get_class_from_spec(class_spec: ClassSpec) -> Class {
    match class_spec {
        ClassSpec::Iaido | ClassSpec::Moonstrike => Class::Stormblade,
        ClassSpec::Icicle | ClassSpec::Frostbeam => Class::FrostMage,
        ClassSpec::Vanguard | ClassSpec::Skyward => Class::WindKnight,
        ClassSpec::Smite | ClassSpec::Lifebind => Class::VerdantOracle,
        ClassSpec::Earthfort | ClassSpec::Block => Class::HeavyGuardian,
        ClassSpec::Wildpack | ClassSpec::Falconry => Class::Marksman,
        ClassSpec::Recovery | ClassSpec::Shield => Class::ShieldKnight,
        ClassSpec::Dissonance | ClassSpec::Concerto => Class::BeatPerformer,
        ClassSpec::Unknown => Class::Unknown,
    }
}
