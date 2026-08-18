"""Map Postgres column types to JSON Schema (Draft 2020-12) + record coercion."""
from __future__ import annotations

import re

from .sqlparse import Column, Table

TEXT_TYPES = {
    "text", "varchar", "char", "bpchar", "citext", "name", "character varying",
    "character", "inet", "cidr", "macaddr", "json", "jsonb", "xml", "tsvector",
    "uuid", "interval",
}
NUMERIC_TYPES = {
    "smallint", "integer", "int", "bigint", "int2", "int4", "int8", "real",
    "double precision", "float", "float4", "float8", "numeric", "decimal",
    "money", "serial", "bigserial", "smallserial",
}
BOOL_TYPES = {"boolean", "bool"}


def base_type(t: str) -> str:
    m = re.match(r"([a-z_ ]+?)\s*\(", t)
    return (m.group(1) if m else t).strip().lower()


def json_schema_type(col: Column) -> str:
    if col.is_enum or base_type(col.pg_type) in TEXT_TYPES:
        return "string"
    if base_type(col.pg_type) in NUMERIC_TYPES:
        return "number"
    if base_type(col.pg_type) in BOOL_TYPES:
        return "boolean"
    if base_type(col.pg_type) in ("date", "timestamp", "timestamptz", "timestamp with time zone", "timestamp without time zone", "time", "timetz"):
        return "string"
    return "string"


def column_schema(col: Column) -> dict:
    js = {"type": json_schema_type(col)}
    if col.is_enum:
        js["enum"] = col.enum_vals
    if col.references:
        js["$comment"] = f"fk:{col.references}"
    return js


def table_schema(tbl: Table, required_override: set[str] | None = None) -> dict:
    properties: dict = {}
    required: list[str] = []
    for name in tbl.order:
        col = tbl.columns[name]
        properties[name] = column_schema(col)
        if col.not_null:
            required.append(name)
    if required_override is not None:
        required = [r for r in required if r in required_override or True]
    schema: dict = {
        "type": "object",
        "properties": properties,
        "additionalProperties": True,
    }
    if required:
        schema["required"] = required
    return schema


def coerce_value(col: Column, v):
    """Coerce a raw DB value (str/None) to the target JSON type."""
    if v is None:
        return None
    bt = base_type(col.pg_type)
    if col.is_enum or bt in TEXT_TYPES:
        return str(v)
    if bt in NUMERIC_TYPES:
        try:
            f = float(v)
            if f == int(f):
                return int(f)
            return f
        except (TypeError, ValueError):
            return str(v)
    if bt in BOOL_TYPES:
        return str(v).strip().lower() in ("true", "t", "1", "yes")
    return str(v)
