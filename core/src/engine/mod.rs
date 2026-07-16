pub mod buff_dictionary;
pub mod buff_source;
pub mod buff_tracker;
pub mod calculator;
pub mod class;
pub mod combat_stats;
pub mod consumables;
pub mod demo;
pub mod encounter;
pub mod entity;
pub mod event;
pub mod history;
pub mod imagine_overrides;
pub mod imagine_skills;
pub mod monster_names;
pub mod name_cache;
pub mod processor;
pub mod runtime_settings;
pub mod selected_uid;
pub mod skill_names;

/// テスト専用の補助。`imagine_skills::NAMES` / `imagine_overrides::STORE` はプロセス全体で
/// 共有される static のため、それらを破壊的に触るテスト（`imagine_skills.rs` の
/// `dev_rename_entry` 系、`imagine_overrides.rs`、`entity.rs` の各テスト）が並列実行で
/// 互いのリネーム/上書き途中の状態を観測しないよう、この共通ロックで直列化する。
/// 呼び出し側は返り値をテスト関数のスコープ終わりまで束縛保持すること
/// （束縛しないと即ドロップされ排他にならない）。
#[cfg(test)]
pub(crate) mod imagine_test_support {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    pub(crate) fn guard() -> MutexGuard<'static, ()> {
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
}
