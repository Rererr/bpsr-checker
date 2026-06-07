#[derive(Debug, Clone)]
pub enum EventType {
    Damage,
    Heal,
    Miss,
}

#[derive(Debug, Clone)]
pub struct CombatEvent {
    pub timestamp_ms: u64,
    pub event_type: EventType,
    pub attacker_uid: i64,
    pub target_uid: i64,
    pub skill_id: i32,
    pub value: i64,
    pub is_crit: bool,
    pub is_lucky: bool,
    pub is_attacker_player: bool,
    pub is_target_boss: bool,
}
