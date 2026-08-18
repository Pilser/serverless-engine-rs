"""Parse Postgres SQL migrations into a per-table schema model.

Accumulates columns from CREATE TABLE plus all ALTER TABLE ... ADD COLUMN
statements so the resulting model reflects the final shape of each table.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path

UUID_RE = re.compile(r"gen_random_uuid\(\)|uuid_generate_v4\(\)", re.I)


@dataclass
class Column:
    name: str
    pg_type: str  # normalized lower-case postgres type (e.g. 'uuid', 'text', 'numeric(5,2)')
    not_null: bool = False
    default: str | None = None
    is_enum: bool = False
    enum_vals: list[str] = field(default_factory=list)
    is_pk: bool = False
    references: str | None = None  # 'table(column)'
    check: str | None = None
    unique: bool = False


@dataclass
class Table:
    name: str
    columns: dict[str, Column] = field(default_factory=dict)
    order: list[str] = field(default_factory=list)
    unique_keys: list[list[str]] = field(default_factory=list)
    pks: list[str] = field(default_factory=list)
    checks: list[str] = field(default_factory=list)

    def add_column(self, col: Column) -> None:
        if col.name not in self.columns:
            self.order.append(col.name)
        self.columns[col.name] = col


@dataclass
class SchemaModel:
    enums: dict[str, list[str]] = field(default_factory=dict)
    tables: dict[str, Table] = field(default_factory=dict)

    def table(self, name: str) -> Table:
        if name not in self.tables:
            self.tables[name] = Table(name=name)
        return self.tables[name]


# Strip a leading schema qualifier and quote chars from an identifier.
def clean_id(ident: str) -> str:
    ident = ident.strip()
    if ident.lower() == "public":
        return ""
    if "." in ident:
        parts = ident.split(".")
        if parts[0].strip().lower() == "public":
            ident = ".".join(parts[1:])
    return ident.strip('"').strip()


def split_top_level(text: str, sep: str = ",") -> list[str]:
    """Split on a separator, respecting parens/quotes/single-quoted strings."""
    parts: list[str] = []
    depth = 0
    in_str = False
    cur: list[str] = []
    for ch in text:
        if ch == "'":
            in_str = not in_str
        elif not in_str and ch == "(":
            depth += 1
        elif not in_str and ch == ")":
            depth -= 1
        if not in_str and depth == 0 and ch == sep:
            parts.append("".join(cur).strip())
            cur = []
        else:
            cur.append(ch)
    if cur:
        parts.append("".join(cur).strip())
    return parts


def _type_and_rest(defn: str) -> tuple[str, str]:
    """Split a column definition into (type, remaining constraints)."""
    rest = defn
    depth = 0
    i = 0
    in_str = False
    while i < len(rest):
        ch = rest[i]
        if ch == "'":
            in_str = not in_str
        elif not in_str and ch == "(":
            depth += 1
        elif not in_str and ch == ")":
            depth -= 1
        elif not in_str and depth == 0 and ch.isspace():
            break
        i += 1
    return rest[:i].strip(), rest[i:].strip()


def parse_type_name(t: str) -> str:
    return t.strip().strip('"').lower()


def _strip_default(sql: str) -> str:
    return re.sub(r"^\s*default\s+", "", sql, flags=re.I)


def _parse_default(default_sql: str) -> str:
    d = _strip_default(default_sql).strip()
    d = d.rstrip(",")
    if d == "NULL":
        return None
    return d


def parse_column(defn: str, enums: dict[str, list[str]]) -> Column | None:
    """Parse a single column definition line; returns None for table constraints."""
    defn = defn.strip()
    if not defn:
        return None
    low = defn.lower()
    # Table-level constraints (handled by caller)
    if low.startswith(("primary key", "unique", "foreign key", "check", "constraint", "index")):
        return None

    name, rest = _type_and_rest(defn)
    if not rest and not name:
        return None
    # If 'name rest' has no type at all (name is a type), skip
    colname = clean_id(name)
    if not colname:
        return None

    col = Column(name=colname, pg_type="text")
    # Default is a JSON keyword, not a type
    if rest.lower().startswith("default"):
        col.default = _parse_default(rest)
        col.pg_type = "text"
        return col

    raw_type, tail = _type_and_rest(rest)
    pg = parse_type_name(raw_type)

    if pg in enums:
        col.is_enum = True
        col.enum_vals = enums[pg]
        col.pg_type = "enum"
    else:
        col.pg_type = pg

    # remaining tail constraints
    tail_low = tail.lower()
    if re.search(r"\bnot\s+null\b", tail_low):
        col.not_null = True
    if re.search(r"\bprimary\s+key\b", tail_low):
        col.is_pk = True
        col.not_null = True
    if re.search(r"\bunique\b", tail_low):
        col.unique = True
    if re.search(r"\bdefault\b", tail_low):
        m = re.search(r"\bdefault\b\s+(.*?)(?=\s+(?:check|references|not\s+null|unique|constraint)\b|$)", tail, flags=re.I | re.S)
        if m:
            col.default = _parse_default(m.group(1))
    mref = re.search(r"\breferences\s+([a-z_0-9]+)\s*\(\s*([a-z_0-9]+)\s*\)", tail, flags=re.I)
    if mref:
        col.references = f"{mref.group(1)}({mref.group(2)})"
    mchk = re.search(r"\bcheck\s*\((.*)\)\s*$", tail, flags=re.I | re.S)
    if mchk:
        col.check = mchk.group(1).strip()
    return col


def _extract_enum_names(defn: str) -> list[str]:
    return [v.strip().strip("'") for v in defn.split(",")]


def _table_name_from_create(defn: str) -> str:
    m = re.search(r"\b(table|if\s+not\s+exists)\s+([a-z_0-9.]+)\b", defn, flags=re.I)
    return clean_id(m.group(2)) if m else ""


def parse_sql(sql: str, model: SchemaModel) -> None:
    # CREATE TYPE enums
    for m in re.finditer(r"CREATE\s+TYPE\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-z_0-9_.]+)\s+AS\s+ENUM\s*\((.*?)\)", sql, flags=re.I | re.S):
        ename = clean_id(m.group(1))
        model.enums[ename] = _extract_enum_names(m.group(2))

    # CREATE TABLE blocks
    for m in re.finditer(
        r"CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-z_0-9_.]+)\s*\((.*?)\)\s*;",
        sql,
        flags=re.I | re.S,
    ):
        tname = clean_id(m.group(1))
        if not tname:
            continue
        body = m.group(2)
        tbl = model.table(tname)
        for part in split_top_level(body):
            low = part.lower()
            if low.startswith("constraint"):
                # e.g. CONSTRAINT x UNIQUE(a,b) / CHECK(...)
                inner = re.sub(r"^constraint\s+[a-z_0-9_]+\s+", "", part, flags=re.I)
                _handle_table_constraint(tbl, inner, model)
            elif low.startswith("primary key"):
                _handle_table_constraint(tbl, part, model)
            elif low.startswith("unique"):
                _handle_table_constraint(tbl, part, model)
            elif low.startswith("check"):
                mchk = re.search(r"check\s*\((.*)\)\s*$", part, flags=re.I | re.S)
                if mchk:
                    tbl.checks.append(mchk.group(1).strip())
            elif low.startswith("foreign key"):
                pass
            else:
                col = parse_column(part, model.enums)
                if col is not None:
                    tbl.add_column(col)

        # Collect column-level primary keys.
        for c in tbl.columns.values():
            if c.is_pk and c.name not in tbl.pks:
                tbl.pks.append(c.name)

    # ALTER TABLE ... ADD COLUMN
    for m in re.finditer(
        r"ALTER\s+TABLE\s+(?:ONLY\s+)?([a-z_0-9_.]+)\s+ADD\s+(?:COLUMN\s+)?(IF\s+NOT\s+EXISTS\s+)?([a-z_0-9_]+\s+.+?)\s*[,;]",
        sql,
        flags=re.I | re.S,
    ):
        tname = clean_id(m.group(1))
        if not tname:
            continue
        col = parse_column(m.group(3), model.enums)
        if col is not None:
            model.table(tname).add_column(col)


def _handle_table_constraint(tbl: Table, inner: str, model: SchemaModel) -> None:
    low = inner.lower()
    if low.startswith("primary key"):
        cols = _paren_cols(inner)
        tbl.pks = cols
        for c in cols:
            if c in tbl.columns:
                tbl.columns[c].is_pk = True
                tbl.columns[c].not_null = True
    elif low.startswith("unique"):
        cols = _paren_cols(inner)
        if cols:
            tbl.unique_keys.append(cols)
    elif low.startswith("check"):
        m = re.search(r"check\s*\((.*)\)\s*$", inner, flags=re.I | re.S)
        if m:
            tbl.checks.append(m.group(1).strip())


def _paren_cols(defn: str) -> list[str]:
    m = re.search(r"\((.*)\)", defn, flags=re.S)
    if not m:
        return []
    return [clean_id(x) for x in split_top_level(m.group(1))]


def load_migrations(directory: str) -> SchemaModel:
    model = SchemaModel()
    paths = sorted(Path(directory).glob("*.sql"))
    for p in paths:
        parse_sql(p.read_text(errors="replace"), model)
    return model
