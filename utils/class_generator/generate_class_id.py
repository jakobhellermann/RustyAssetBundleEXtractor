#!/usr/bin/env python3
"""Regenerate src/objects/class_id.rs from AssetRipper/TypeTreeDumps.

Reads every Classes/*.json in a local TypeTreeDumps checkout (each file is a
{id: name} map for one Unity version) and merges them into a single id->name
table. When the same id has different names across versions, the name from the
newest version (highest-sorting filename) wins.

UnknownType = -1 is preserved as a hardcoded entry since it is the Default
variant and not part of the upstream dumps.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DUMPS = HERE / "TypeTreeDumps" / "Classes"
OUT = HERE.parents[1] / "src" / "objects" / "class_id.rs"


def version_key(path: Path) -> tuple:
    """Sort key over filenames like '2017.1.0b1.json', '6000.0.0f1.json'.

    Splits the stem on '.' and treats each part as (int_prefix, suffix). That
    orders 2 < 10 numerically and keeps 'b1' < 'f1' lexically within a part.
    """
    parts = []
    for chunk in path.stem.split("."):
        num = ""
        i = 0
        while i < len(chunk) and chunk[i].isdigit():
            num += chunk[i]
            i += 1
        parts.append((int(num) if num else -1, chunk[i:]))
    return tuple(parts)


def load_entries() -> tuple[dict[int, str], dict[int, list[str]], dict[int, list[str]]]:
    """Return (id -> name, id -> renamed-from list, id -> reassigned-from list).

    Two-pass merge:
      1. Iterate oldest -> newest so later versions overwrite per-id renames.
         For each id, also remember all earlier names it held -- these become
         "Renamed from" annotations.
      2. If the same name ends up on multiple ids (a class was moved to a new
         id at some point), keep only the id that still holds the name in the
         newest version that mentions either id. The dropped ids become
         "Reassigned from" annotations on the surviving entry.
    """
    if not DUMPS.is_dir():
        raise SystemExit(
            f"missing {DUMPS}. Run `just update-class-id` (it clones TypeTreeDumps)."
        )
    files = sorted(DUMPS.glob("*.json"), key=version_key)
    if not files:
        raise SystemExit(f"no JSON files in {DUMPS}")

    merged: dict[int, str] = {}
    prior_names: dict[int, list[str]] = {}
    for f in files:
        data = json.loads(f.read_text())
        for k, v in data.items():
            cid = int(k)
            old = merged.get(cid)
            if old is not None and old != v:
                lst = prior_names.setdefault(cid, [])
                if old not in lst:
                    lst.append(old)
            merged[cid] = v

    # Drop the current name from the prior-names list (a class can flip back
    # and forth across versions; the final name is what survives).
    renamed_from: dict[int, list[str]] = {}
    for cid, names in prior_names.items():
        filtered = [n for n in names if n != merged[cid]]
        if filtered:
            renamed_from[cid] = filtered

    # Find name collisions across ids.
    by_name: dict[str, list[int]] = {}
    for cid, name in merged.items():
        by_name.setdefault(name, []).append(cid)

    reassigned_from: dict[int, list[str]] = {}
    for name, ids in by_name.items():
        if len(ids) < 2:
            continue
        winner = None
        for f in reversed(files):
            data = json.loads(f.read_text())
            for cid in ids:
                if data.get(str(cid)) == name:
                    winner = cid
                    break
            if winner is not None:
                break
        if winner is None:
            winner = max(ids)
        for cid in ids:
            if cid != winner:
                del merged[cid]
                reassigned_from.setdefault(winner, []).append(str(cid))
                # The dropped id may itself have had earlier names; surface them
                # as renames of the surviving entry.
                for n in prior_names.get(cid, []):
                    if n != merged[winner]:
                        renamed_from.setdefault(winner, []).append(n)

    return merged, renamed_from, reassigned_from


def render(
    entries: dict[int, str],
    renamed_from: dict[int, list[str]],
    reassigned_from: dict[int, list[str]],
) -> str:
    items = sorted(entries.items())

    def const_line(cid: int, name: str) -> str:
        comments = []
        if cid in renamed_from:
            quoted = ", ".join(f'"{n}"' for n in renamed_from[cid])
            comments.append(f"    // Renamed from: {quoted}")
        if cid in reassigned_from:
            ids = ", ".join(reassigned_from[cid])
            comments.append(f"    // Reassigned from id: {ids}")
        line = f"    pub const {name}: ClassId = ClassId({cid});"
        return "\n".join(comments + [line])

    consts = ["    pub const UnknownType: ClassId = ClassId(-1);"]
    consts += [const_line(cid, name) for cid, name in items]

    name_map = ['    (ClassId::UnknownType, "UnknownType"),']
    name_map += [f'    (ClassId::{name}, "{name}"),' for cid, name in items]
    name_map[-1] = name_map[-1].rstrip(",")

    return (
        "use std::collections::BTreeMap;\n"
        "\n"
        "#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]\n"
        "pub struct ClassId(pub i32);\n"
        "impl Default for ClassId {\n"
        "    fn default() -> Self {\n"
        "        ClassId::UnknownType\n"
        "    }\n"
        "}\n"
        "\n"
        "impl ClassId {\n"
        "    pub fn name(&self) -> Option<&str> {\n"
        "        CLASS_ID_NAME.get(self).copied()\n"
        "    }\n"
        "}\n"
        "\n"
        "impl std::fmt::Debug for ClassId {\n"
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n"
        "        match self.name() {\n"
        "            Some(name) => f.write_str(name),\n"
        '            None => write!(f, "ClassId({})", self.0),\n'
        "        }\n"
        "    }\n"
        "}\n"
        "\n"
        "#[allow(non_upper_case_globals)]\n"
        "impl ClassId {\n" + "\n".join(consts) + "\n}\n"
        "\n"
        "use std::sync::LazyLock;\n"
        "#[rustfmt::skip]\n"
        "pub static CLASS_ID_NAME: LazyLock<BTreeMap<ClassId, &'static str>> = LazyLock::new(|| [\n"
        + "\n".join(name_map)
        + "\n]\n"
        "    .iter()\n"
        "    .copied()\n"
        "    .collect());\n"
    )


def main() -> int:
    entries, renamed_from, reassigned_from = load_entries()
    OUT.write_text(render(entries, renamed_from, reassigned_from))
    print(f"wrote {len(entries)} entries to {OUT}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
