//! バトルイマジン(装備枠 SlotPositionId 7/8)のスキルID→表示名。
//! ID集合・名称は ImagineSkillNames.json を埋め込む静的テーブル。
//! 日本語(names_ja)を優先し、無ければ英語(names_en)にフォールバックする。
//! 中国語のみ判明のもの・完全に未登録のIDは表示しない（誤表示を避ける安全側デフォルト）。
//!
//! 開発者モード（`BPSR_DEV=1`）向けに、テーブルをその場で書き換える API
//! （[`dev_rename_entry`] / [`dev_save_to_working_tree`]）も提供する。テーブルは
//! `RwLock` で保持し、`imagine_name` の挙動（ja優先→en→None）はリネーム前後で不変。

use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

/// dev編集・書き戻しで発生しうるエラー。
#[derive(Debug, thiserror::Error)]
pub enum ImagineDbError {
    #[error(
        "working tree not available (distributed build has no ImagineSkillNames.json to write back)"
    )]
    WorkingTreeUnavailable,
    #[error("failed to write imagine DB: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to serialize imagine DB: {0}")]
    Serialize(#[from] serde_json::Error),
}

#[derive(serde::Deserialize)]
struct Names {
    #[serde(rename = "_comment")]
    comment: String,
    names_ja: HashMap<String, String>,
    names_en: HashMap<String, String>,
}

/// 書き戻し用の出力形式。フィールド順が JSON のキー順になる（`_comment`→ja→en）。
/// `BTreeMap<i32, _>` で id 昇順の安定出力にする。
#[derive(Serialize)]
struct NamesOut<'a> {
    #[serde(rename = "_comment")]
    comment: &'a str,
    names_ja: BTreeMap<i32, &'a String>,
    names_en: BTreeMap<i32, &'a String>,
}

fn parse_id_map(map: HashMap<String, String>) -> HashMap<i32, String> {
    map.into_iter()
        .filter_map(|(k, v)| k.parse::<i32>().ok().map(|id| (id, v)))
        .collect()
}

struct ImagineTable {
    ja: HashMap<i32, String>,
    en: HashMap<i32, String>,
    /// 埋め込みJSONの `_comment`。dev編集では変更せず書き戻し時に温存する。
    comment: String,
}

fn seed_table() -> ImagineTable {
    let data = include_str!("../../data/json/ImagineSkillNames.json");
    let parsed: Names = serde_json::from_str(data).expect("invalid ImagineSkillNames.json");
    ImagineTable {
        ja: parse_id_map(parsed.names_ja),
        en: parse_id_map(parsed.names_en),
        comment: parsed.comment,
    }
}

static NAMES: LazyLock<RwLock<ImagineTable>> = LazyLock::new(|| RwLock::new(seed_table()));

/// `id` の解決名（ja優先→en→None）。テーブル参照の共通ロジック。
fn resolve_id(table: &ImagineTable, id: i32) -> Option<String> {
    table.ja.get(&id).or_else(|| table.en.get(&id)).cloned()
}

/// `skill_id` に対応するバトルイマジン表示名（未登録なら None）。日本語優先、無ければ英語。
pub fn imagine_name(skill_id: i32) -> Option<String> {
    let table = NAMES.read().unwrap_or_else(|e| e.into_inner());
    resolve_id(&table, skill_id)
}

/// スキルIDを解決名（＝canonical）でグループ化した1件。
/// `main_skill_id` は 3900〜3999 の canonical 範囲内の最小id（無ければ全体の最小id）、
/// `clone_skill_ids` はそれ以外を昇順で並べたもの（分身/召喚スキルID）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagineEntry {
    pub canonical: String,
    /// グループのen名。dev_rename_entryの効果確認・単体テストで参照する。現状UI/main.rsは
    /// 未消費（表示言語追従の名前解決はskill_names側で行うため）。
    pub name_en: Option<String>,
    pub main_skill_id: i32,
    pub clone_skill_ids: Vec<i32>,
}

/// バトルイマジンをグループ化した一覧（canonical 名の昇順）。UIの一覧表示・dev編集の
/// 対象選択に使う。グループ化のキーは「その id の解決名（ja優先→en）」の一致であり、
/// 異なる id が偶然同じ解決名を持つ場合は同一グループへ束ねられる
/// （例: 3905/3939 はどちらも ja="マイティボア" のため同一グループになり、
/// 3939 固有の en 名は `name_en` には現れない＝仕様上の既知の制約）。
pub fn imagine_entries() -> Vec<ImagineEntry> {
    let table = NAMES.read().unwrap_or_else(|e| e.into_inner());

    let all_ids: HashSet<i32> = table.ja.keys().chain(table.en.keys()).copied().collect();
    let mut groups: HashMap<String, Vec<i32>> = HashMap::new();
    for id in all_ids {
        let Some(name) = resolve_id(&table, id) else {
            continue;
        };
        groups.entry(name).or_default().push(id);
    }

    let mut entries: Vec<ImagineEntry> = groups
        .into_iter()
        .map(|(canonical, mut ids)| {
            ids.sort_unstable();
            let main_skill_id = ids
                .iter()
                .copied()
                .find(|id| (3900..=3999).contains(id))
                .unwrap_or(ids[0]);
            let clone_skill_ids: Vec<i32> = ids
                .iter()
                .copied()
                .filter(|&id| id != main_skill_id)
                .collect();
            let name_en = table
                .en
                .get(&main_skill_id)
                .cloned()
                .or_else(|| ids.iter().find_map(|id| table.en.get(id).cloned()));
            ImagineEntry {
                canonical,
                name_en,
                main_skill_id,
                clone_skill_ids,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.canonical.cmp(&b.canonical));
    entries
}

/// `canonical` に解決される全 skill_id（ja/en 両マップ）の名前を書き換える（dev編集）。
/// - `new_name_ja` が `Some` かつ非空のときのみ、既にja側にキーを持つ該当idの値を更新する。
/// - `new_name_en` が `Some` のときのみ（空文字も可）、既にen側にキーを持つ該当idの値を更新する。
///
/// どちらの引数も、存在しない id へ新規キーを追加することはしない（既存キーの値のみ書き換える）。
pub fn dev_rename_entry(canonical: &str, new_name_ja: Option<String>, new_name_en: Option<String>) {
    let mut table = NAMES.write().unwrap_or_else(|e| e.into_inner());

    let ids: HashSet<i32> = table
        .ja
        .keys()
        .chain(table.en.keys())
        .copied()
        .filter(|&id| resolve_id(&table, id).as_deref() == Some(canonical))
        .collect();

    if let Some(name_ja) = new_name_ja.filter(|s| !s.is_empty()) {
        for id in &ids {
            if table.ja.contains_key(id) {
                table.ja.insert(*id, name_ja.clone());
            }
        }
    }
    if let Some(name_en) = new_name_en {
        for id in &ids {
            if table.en.contains_key(id) {
                table.en.insert(*id, name_en.clone());
            }
        }
    }
}

/// ワークツリーの `core/data/json/ImagineSkillNames.json` を現在のテーブルで整形保存する。
/// 配布ビルドでは `CARGO_MANIFEST_DIR`（ビルド時に埋め込まれるパス）が実機に存在しないため
/// [`ImagineDbError::WorkingTreeUnavailable`] を返す。
pub fn dev_save_to_working_tree() -> Result<PathBuf, ImagineDbError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/json/ImagineSkillNames.json");
    if !path_is_working_tree_available(&path) {
        return Err(ImagineDbError::WorkingTreeUnavailable);
    }
    let table = NAMES.read().unwrap_or_else(|e| e.into_inner());
    write_table_to_path(&path, &table.comment, &table.ja, &table.en)?;
    Ok(path)
}

/// `path` の親ディレクトリが実在するか（＝そこへ書き戻し可能か）。
fn path_is_working_tree_available(path: &Path) -> bool {
    path.parent().is_some_and(|p| p.is_dir())
}

/// テーブルを整形JSON（`_comment` 温存・id昇順）へシリアライズする純関数。
/// ファイルI/Oを伴わないため、パスに依存せず単体テストできる。
fn serialize_table(
    comment: &str,
    ja: &HashMap<i32, String>,
    en: &HashMap<i32, String>,
) -> Result<Vec<u8>, ImagineDbError> {
    let out = NamesOut {
        comment,
        names_ja: ja.iter().map(|(k, v)| (*k, v)).collect(),
        names_en: en.iter().map(|(k, v)| (*k, v)).collect(),
    };
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    out.serialize(&mut ser)?;
    Ok(buf)
}

/// `serialize_table` の結果を `path` へ書き出す。`dev_save_to_working_tree` と
/// round-trip テスト（temp path）の両方から使う共通の書き込み経路。
fn write_table_to_path(
    path: &Path,
    comment: &str,
    ja: &HashMap<i32, String>,
    en: &HashMap<i32, String>,
) -> Result<(), ImagineDbError> {
    let bytes = serialize_table(comment, ja, en)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imagine_name_unchanged_for_known_ids() {
        let _guard = crate::engine::imagine_test_support::guard();
        // canonical(ja) と分身/召喚ID、および ja に無く en のみで解決されるIDの両方を確認する。
        assert_eq!(imagine_name(3942), Some("ヴェノミーンの巣".to_string()));
        assert_eq!(imagine_name(1007740), Some("ヴェノミーンの巣".to_string()));
        assert_eq!(imagine_name(1007741), Some("ヴェノミーンの巣".to_string()));
        assert_eq!(imagine_name(3909), Some("Void Foxen".to_string())); // ja未登録・enのみ
        assert_eq!(imagine_name(-1), None); // 完全未登録
    }

    #[test]
    fn imagine_entries_groups_venobzzar_incubator() {
        let _guard = crate::engine::imagine_test_support::guard();
        let entries = imagine_entries();
        let entry = entries
            .iter()
            .find(|e| e.canonical == "ヴェノミーンの巣")
            .expect("ヴェノミーンの巣 group must exist");
        assert_eq!(entry.main_skill_id, 3942);
        assert_eq!(entry.clone_skill_ids, vec![1007740, 1007741]);
        assert_eq!(entry.name_en, Some("Venobzzar Incubator".to_string()));
    }

    #[test]
    fn imagine_entries_groups_duplicate_ja_canonical_by_resolved_name() {
        let _guard = crate::engine::imagine_test_support::guard();
        // 3905/3939 はどちらも ja="マイティボア" で解決されるため同一グループへ束ねられる
        // （既知の仕様上の制約: 3939 固有の en「Great Warhog」は main 側の en に負けて消える）。
        let entries = imagine_entries();
        let entry = entries
            .iter()
            .find(|e| e.canonical == "マイティボア")
            .expect("マイティボア group must exist");
        assert_eq!(entry.main_skill_id, 3905); // 3900-3999 内の最小
        assert_eq!(entry.clone_skill_ids, vec![3939, 102640, 102658, 1008040]);
        assert_eq!(entry.name_en, Some("Boarrier Tyrant".to_string())); // 3905側のen（3939のenは採用されない）
    }

    #[test]
    fn dev_rename_entry_updates_ja_and_en_for_all_group_ids() {
        let _guard = crate::engine::imagine_test_support::guard();
        // 他テストと衝突しない孤立したグループ（ボイス=3950/2900715/2900740）を使う。
        let ids = [3950, 2900715, 2900740];
        for id in ids {
            assert_eq!(imagine_name(id), Some("ボイス".to_string()));
        }

        dev_rename_entry(
            "ボイス",
            Some("ボイス改".to_string()),
            Some("Boyce2".to_string()),
        );
        for id in ids {
            assert_eq!(imagine_name(id), Some("ボイス改".to_string()));
        }
        let entry = imagine_entries()
            .into_iter()
            .find(|e| e.canonical == "ボイス改")
            .expect("renamed group must be found under new canonical");
        assert_eq!(entry.name_en, Some("Boyce2".to_string()));

        // 元に戻す（プロセス内で共有される static のため、他テストへ影響させない）。
        dev_rename_entry(
            "ボイス改",
            Some("ボイス".to_string()),
            Some("Boyce".to_string()),
        );
        for id in ids {
            assert_eq!(imagine_name(id), Some("ボイス".to_string()));
        }
    }

    #[test]
    fn dev_rename_entry_empty_ja_is_noop_but_empty_en_applies() {
        let _guard = crate::engine::imagine_test_support::guard();
        // 孤立した単一ID（ドロシー=3949）。ja=Some("")は無視、en=Some("")は適用される非対称仕様の確認。
        assert_eq!(imagine_name(3949), Some("ドロシー".to_string()));

        dev_rename_entry("ドロシー", Some(String::new()), Some(String::new()));
        // ja側は変更されないため canonical のまま解決される。
        assert_eq!(imagine_name(3949), Some("ドロシー".to_string()));
        let entry = imagine_entries()
            .into_iter()
            .find(|e| e.canonical == "ドロシー")
            .expect("ドロシー group must still exist");
        assert_eq!(entry.name_en, Some(String::new())); // en側は空文字で上書きされている

        // 元に戻す。
        dev_rename_entry("ドロシー", None, Some("Dorothy".to_string()));
        let entry = imagine_entries()
            .into_iter()
            .find(|e| e.canonical == "ドロシー")
            .expect("ドロシー group must still exist");
        assert_eq!(entry.name_en, Some("Dorothy".to_string()));
    }

    #[test]
    fn working_tree_availability_check_reflects_real_directory_presence() {
        assert!(!path_is_working_tree_available(Path::new(
            "Z:/definitely/not/a/real/bpsr-checker/path/ImagineSkillNames.json"
        )));
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/json/ImagineSkillNames.json");
        assert!(path_is_working_tree_available(&real));
    }

    #[test]
    fn serialize_table_round_trip_preserves_comment_and_orders_ids_ascending() {
        let mut ja = HashMap::new();
        ja.insert(42, "テスト名".to_string());
        ja.insert(7, "小さいID".to_string());
        let mut en = HashMap::new();
        en.insert(42, "Test Name".to_string());

        let dir = std::env::temp_dir().join(format!(
            "bpsr_imagine_roundtrip_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("roundtrip.json");

        write_table_to_path(&path, "テストコメント", &ja, &en).expect("write should succeed");

        let data = std::fs::read_to_string(&path).expect("read back");
        let parsed: Names = serde_json::from_str(&data).expect("parse back");
        assert_eq!(parsed.comment, "テストコメント");
        assert_eq!(parsed.names_ja.get("42"), Some(&"テスト名".to_string()));
        assert_eq!(parsed.names_ja.get("7"), Some(&"小さいID".to_string()));
        assert_eq!(parsed.names_en.get("42"), Some(&"Test Name".to_string()));

        // id昇順で安定出力されているか（"7" が "42" より前に出現）。
        let idx7 = data.find("\"7\"").expect("id 7 present");
        let idx42 = data.find("\"42\"").expect("id 42 present");
        assert!(idx7 < idx42);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
