#!/usr/bin/env python3
"""食事/シロップ base_id -> 日本語効果ラベルを生成する。

姉妹リポ BPSR-ZDPS の BuffTable.json（NameDesign=中国語の構造化名 / Desc=英語の効果）と
core/data/json/ConsumableBuffIds.json（対象 base_id 一覧）から、
slint-app/data/ConsumableBuffNames.ja.json（id -> "物攻 +15" 等）を生成する。

NameDesign 例: "S1-物攻Lv1" / "[S2]魔攻Lv2" / "【S3】护甲+精英减伤Lv1——" /
              "S1-元素强度·火Lv1" / "元素抗性·暗Lv3" / "魔法增效强度Lv1"
Desc 例: "Physical Attack +15" / "ATK +90" / "Restores 105 HP per second" /
        "Armor +900, DMG taken from Elites or stronger enemies -5%"

ゲーム更新で ID が増えたら BPSR-ZDPS を更新して本スクリプトを再実行する。
UNKNOWN 行が出たら STAT_MAP / SPECIAL を追記する。

実行: python scripts/gen-consumable-names.py
"""
from __future__ import annotations
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
BUFFTABLE = REPO.parent / "BPSR-ZDPS" / "BPSR-ZDPS" / "Data" / "BuffTable.json"
IDS = REPO / "core" / "data" / "json" / "ConsumableBuffIds.json"
OUT = REPO / "slint-app" / "data" / "ConsumableBuffNames.ja.json"

ELEM = {"火": "火", "冰": "氷", "森": "森", "岩": "岩",
        "风": "風", "雷": "雷", "光": "光", "暗": "闇"}

# 単純スタット core(中) -> 日本語短ラベル
STAT_MAP = {
    "物攻": "物攻",
    "魔攻": "魔攻",
    "护甲": "防御",
    "耐力": "耐久力",
    "生命恢复": "HP回復",
    "物理增效强度": "物理増幅",
    "魔法增效强度": "魔法増幅",
}

# 構造化されない特殊バフは id 直指定でラベルを与える。
SPECIAL = {
    700083: "虚蝕の戦利品",
    700084: "虚蝕の戦利品",
    833097: "食材",
    965208: "豊穣の宴",
    2010003: "豊穣の宴(力知敏)",
    681836: "虚蝕変身",
    900120: "回復禁止",
    900128: "薬禁止",
    997030: "迷心花粉",
    2010064: "沈夢耐性1",
    2010065: "沈夢耐性2",
    2010066: "沈夢耐性3",
    3001161: "虚蝕の力",
}


def clean_nd(nd: str) -> str:
    s = nd.replace("——", "").replace("—", "").strip()
    s = re.sub(r"^S1-", "", s)
    s = re.sub(r"^S2-", "", s)
    s = re.sub(r"^\[S2\]", "", s)
    s = re.sub(r"^【S3】", "", s)
    return s.strip()


def stat_label(core: str):
    """core(中) -> (基本ラベル, 種別) 。種別: flat/hp/elite_up/elite_down/None"""
    m = re.match(r"元素强度·(.)$", core)
    if m:
        return ELEM.get(m.group(1), m.group(1)) + "属性強度", "flat"
    m = re.match(r"元素抗性·(.)$", core)
    if m:
        return ELEM.get(m.group(1), m.group(1)) + "属性耐性", "flat"
    if core in STAT_MAP:
        return STAT_MAP[core], ("hp" if core == "生命恢复" else "flat")
    if core == "物攻+精英增伤":
        return "物攻", "elite_up"
    if core == "魔攻+精英增伤":
        return "魔攻", "elite_up"
    if core == "护甲+精英减伤":
        return "防御", "elite_down"
    return None, None


def primary_value(desc: str) -> str | None:
    m = re.search(r"Restores\s+([\d,]+)\s*HP", desc, re.I)
    if m:
        return m.group(1).replace(",", "")
    m = re.search(r"\+\s*([\d,]+)", desc)
    if m:
        return m.group(1).replace(",", "")
    return None


def elite_pct(desc: str) -> str | None:
    # 2番目以降の節に出る ±N%（精鋭増/減ダメ）
    tail = desc.split(",", 1)[1] if "," in desc else ""
    m = re.search(r"[-–]\s*(\d+)%", tail)
    if m:
        return "-" + m.group(1)
    m = re.search(r"\+\s*(\d+)%", tail)
    if m:
        return "+" + m.group(1)
    return None


def build_label(entry: dict) -> tuple[str | None, str]:
    """(label, status) を返す。status: ok/special/unknown"""
    bid = entry["id"]
    if bid in SPECIAL:
        return SPECIAL[bid], "special"
    nd = entry.get("nd") or ""
    desc = entry.get("desc") or ""
    core = re.sub(r"Lv\d+$", "", clean_nd(nd)).strip()
    base, kind = stat_label(core)
    if base is None:
        return None, "unknown"
    val = primary_value(desc)
    if kind == "hp":
        return (f"{base} +{val}/s" if val else base), "ok"
    if kind in ("elite_up", "elite_down"):
        pct = elite_pct(desc)
        sec = ""
        if pct:
            sec = f" 対精鋭{pct}%" if kind == "elite_up" else f" 対精鋭被{pct}%"
        return (f"{base}+{val}{sec}" if val else base + sec), "ok"
    return (f"{base} +{val}" if val else base), "ok"


def main() -> int:
    buff = json.loads(BUFFTABLE.read_text(encoding="utf-8"))
    ids = json.loads(IDS.read_text(encoding="utf-8"))
    targets = list(ids["food"]) + list(ids["syrup"])

    out: dict[str, str] = {}
    unknown = []
    for bid in targets:
        e = buff.get(str(bid)) or {}
        entry = {"id": bid, "nd": e.get("NameDesign"), "desc": e.get("Desc")}
        label, status = build_label(entry)
        if status == "unknown" or not label:
            unknown.append((bid, e.get("NameDesign"), e.get("Desc")))
            continue
        out[str(bid)] = label

    # id 昇順で安定出力
    ordered = {k: out[k] for k in sorted(out, key=int)}
    OUT.write_text(
        json.dumps(ordered, ensure_ascii=False, indent=1) + "\n", encoding="utf-8"
    )
    print(f"wrote {len(ordered)} labels -> {OUT.relative_to(REPO)}")
    if unknown:
        print(f"\n[WARNING] {len(unknown)} unmapped (add to STAT_MAP/SPECIAL):")
        for bid, nd, desc in unknown:
            print(f"  {bid}: nd={nd!r} desc={desc!r}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
