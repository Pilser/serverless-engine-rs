"""Parse static INSERT INTO ... VALUES statements from SQL into JSON records."""
from __future__ import annotations

import re

from .sqlparse import Table, clean_id, split_top_level


def _parse_value(v: str):
    v = v.strip()
    if v.lower() == "null":
        return None
    if v.lower() in ("true", "false"):
        return v.lower() == "true"
    if v.startswith("'") and v.endswith("'"):
        inner = v[1:-1]
        return inner.replace("''", "'")
    # numeric / expressions
    if re.fullmatch(r"-?\d+", v):
        return int(v)
    if re.fullmatch(r"-?\d+\.\d+", v):
        return float(v)
    # non-literal (function call, now(), gen_random_uuid(), variable) -> None placeholder
    return None


def extract_inserts(sql: str) -> list[tuple[str, list[str], list[list]]]:
    """Return [(table, columns, rows)] for static INSERT INTO ... VALUES."""
    out: list[tuple[str, list[str], list[list]]] = []
    pattern = re.compile(
        r"INSERT\s+INTO\s+([a-z_0-9.]+)\s*(?:\(([^)]*)\))?\s*VALUES\s*(.*?)\s*;",
        flags=re.I | re.S,
    )
    for m in pattern.finditer(sql):
        tname = clean_id(m.group(1))
        if not tname:
            continue
        col_text = m.group(2)
        columns = [clean_id(c) for c in split_top_level(col_text)] if col_text else []
        values_text = m.group(3)
        rows: list[list] = []
        for rm in re.finditer(r"\(\s*(.*?)\s*\)\s*(?:,|$)", values_text, flags=re.S):
            body = rm.group(1)
            if not body.strip():
                continue
            if "on conflict" in body.lower():
                continue
            cells = split_top_level(body)
            if not cells:
                continue
            vals = [_parse_value(c) for c in cells]
            if vals:
                rows.append(vals)
        if rows:
            out.append((tname, columns, rows))
    return out


def table_inserts_to_records(tbl: Table, columns: list[str], rows: list[list]) -> list[dict]:
    import uuid as _uuid

    from .typemap import coerce_value

    records: list[dict] = []
    for row in rows:
        rec: dict = {}
        for i, colname in enumerate(columns):
            if colname not in tbl.columns:
                continue
            col = tbl.columns[colname]
            raw = row[i] if i < len(row) else None
            # Skip nulls for nullable columns to keep payloads clean.
            if raw is None and not col.not_null:
                continue
            rec[colname] = coerce_value(col, raw)
        # Auto-generate UUID for missing required PK with a uuid default.
        for pk in tbl.pks:
            col = tbl.columns.get(pk)
            if pk not in rec and col is not None and col.default and "uuid" in col.default.lower():
                rec[pk] = str(_uuid.uuid4())
        if rec:
            records.append(rec)
    return records
