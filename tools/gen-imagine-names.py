#!/usr/bin/env python3
"""バトルイマジン表示名テーブル(ImagineSkillNames.json)を拡張再生成する。

背景:
  バトルイマジンは装備枠(SlotPositionId 7/8)の「奥义/绝技」発動スキル(3900番台など)だが、
  戦闘中の実信号は **召喚エンティティ**として現れ、その AttrSkillId(100) は 3900番台ではなく
  分身/召喚スキル(例 1007740=奥义!毒爆, 1007741=奥义!毒爆虚拟体, 2900240=奥义！生命祈愿)を指す。
  これらは **NameDesign が親イマジンと同一**(虚拟体サフィックス除去・！/!正規化で一致)なので、
  名前グルーピングで「分身/召喚スキルID → 親イマジンの表示名」を機械的に導出できる。

入力:
  1) BPSR-ZDPS(MIT・参照可)の Data/SkillTable.json … CN(NameDesign)/EN(Name) フル辞書
  2) 既存 ImagineSkillNames.json … 3900番台の canonical id → JA/EN 表示名(ゲームファイル抽出由来)

出力:
  ImagineSkillNames.json を上書き。canonical(既存)は温存し、名前一致で解決できる分身/召喚IDを追記。
  JA は canonical に JA があるものだけ、EN は canonical に EN があるものだけ付与(imagine_name の
  ja→en フォールバックに合わせる)。名前一致しないIDは追加しない(誤名回避の安全側デフォルト)。

使い方:
  python tools/gen-imagine-names.py [SkillTable.json パス]
  既定パス: ../BPSR-ZDPS/BPSR-ZDPS/Data/SkillTable.json
"""
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "core" / "data" / "json" / "ImagineSkillNames.json"
DEFAULT_SKILLTABLE = REPO.parent / "BPSR-ZDPS" / "BPSR-ZDPS" / "Data" / "SkillTable.json"


def norm(name: str | None) -> str:
    """NameDesign を突合キーへ正規化: 全角！→!、空白除去、分身サフィックス『虚拟体』除去。"""
    if not name:
        return ""
    s = name.replace("！", "!").replace("　", " ")
    s = s.replace("虚拟体", "")
    s = re.sub(r"\s+", "", s)
    return s


def main() -> None:
    skilltable_path = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_SKILLTABLE
    if not skilltable_path.exists():
        sys.exit(f"SkillTable.json が見つかりません: {skilltable_path}")

    skills = json.loads(skilltable_path.read_text(encoding="utf-8"))
    existing = json.loads(OUT.read_text(encoding="utf-8"))
    canon_ja: dict[str, str] = existing["names_ja"]
    canon_en: dict[str, str] = existing["names_en"]

    # canonical id の NameDesign(正規化) → 表示名 を引く辞書を作る。
    norm_to_ja: dict[str, str] = {}
    norm_to_en: dict[str, str] = {}
    missing_canon: list[str] = []
    for cid, jname in canon_ja.items():
        v = skills.get(cid)
        if v is None:
            missing_canon.append(cid)
            continue
        norm_to_ja.setdefault(norm(v.get("NameDesign")), jname)
    for cid, ename in canon_en.items():
        v = skills.get(cid)
        if v is None:
            continue
        norm_to_en.setdefault(norm(v.get("NameDesign")), ename)

    # 既存(canonical)は温存。名前一致する非canonicalスキルを追記する。
    out_ja = dict(canon_ja)
    out_en = dict(canon_en)
    added_ja = added_en = 0
    for sid, v in skills.items():
        if sid in canon_ja or sid in canon_en:
            continue  # canonical はそのまま
        # 装備枠(7/8)以外へスロットされ得るスキルは展開しない。S3 の図鑑ロールスキル
        # (SlotPositionId 21-24)等が NameDesign 一致で紛れると、ロールスキル起動の召喚を
        # 「装備イマジン」と誤認するため（内部変種は SlotPositionId が [] か [0]）。
        slots = v.get("SlotPositionId") or []
        if any(s not in (0, 7, 8) for s in slots):
            continue
        key = norm(v.get("NameDesign"))
        if not key:
            continue
        if key in norm_to_ja and sid not in out_ja:
            out_ja[sid] = norm_to_ja[key]
            added_ja += 1
        if key in norm_to_en and sid not in out_en:
            out_en[sid] = norm_to_en[key]
            added_en += 1

    def sort_num(m: dict[str, str]) -> dict[str, str]:
        return {k: m[k] for k in sorted(m, key=lambda x: int(x))}

    result = {
        "_comment": (
            "バトルイマジン(装備枠 SlotPositionId 7/8)の発動スキルID→表示名。日本語(names_ja)優先、"
            "無ければ英語(names_en)へフォールバック。3900番台の canonical に加え、戦闘中の召喚エンティティが"
            "持つ分身/召喚スキルID(例 1007740/1007741/2900240)も収録する。後者は BPSR-ZDPS(MIT) の"
            " Data/SkillTable.json の NameDesign 名前グルーピング(虚拟体除去・！/!正規化)で canonical と"
            "同名のものを親イマジン名へ紐付けた(tools/gen-imagine-names.py で再生成)。名前一致しないIDは"
            "未収録=DPS一覧に表示しない(誤名回避の安全側デフォルト)。"
        ),
        "names_ja": sort_num(out_ja),
        "names_en": sort_num(out_en),
    }
    OUT.write_text(json.dumps(result, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")

    print(f"SkillTable: {skilltable_path}")
    print(f"canonical: ja={len(canon_ja)} en={len(canon_en)} (SkillTable未収録の canonical={len(missing_canon)})")
    print(f"追加: ja +{added_ja} → {len(out_ja)} / en +{added_en} → {len(out_en)}")
    if missing_canon:
        print(f"  ※ SkillTable に無い canonical(名前展開できず温存のみ): {missing_canon[:10]}{'...' if len(missing_canon) > 10 else ''}")
    print(f"書き出し: {OUT}")


if __name__ == "__main__":
    main()
