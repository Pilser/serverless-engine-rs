"""Translate a Table into engine board config + recipes (apps.update / recipes.add payloads)."""
from __future__ import annotations

from .sqlparse import Table
from .typemap import table_schema


def unique_key_for(tbl: Table) -> str | None:
    # Prefer the primary key path; else a single-column unique constraint.
    if len(tbl.pks) == 1:
        return f"$.{tbl.pks[0]}"
    for uk in tbl.unique_keys:
        if len(uk) == 1:
            return f"$.{uk[0]}"
    return None


def build_update_payload(tbl: Table) -> dict:
    payload: dict = {"schema": table_schema(tbl)}
    uk = unique_key_for(tbl)
    if uk:
        payload["unique_key"] = uk
    return payload


def build_validate_rules(tbl: Table) -> list[dict] | None:
    """Translate simple CHECK constraints into engine validate rules.

    Engine validate = array of {when, error}: record is REJECTED when 'when' is true.
    A DB CHECK passes when expr is true, so we reject on the negation.
    """
    rules: list[dict] = []
    for chk in tbl.checks:
        neg = _negate_check(chk)
        if neg:
            rules.append({"when": neg, "error": f"constraint failed: {chk}"})
    return rules or None


def _negate_check(expr: str) -> str | None:
    expr = expr.strip()
    if not expr:
        return None
    # Single comparison -> invert operator.
    simple = {
        ">=": "<", "<=": ">", ">": "<=", "<": ">=", "=": "!=",
        "!=": "=", "<>": "=",
    }
    for op, inv in simple.items():
        if op in expr and " and " not in expr.lower() and " or " not in expr.lower() and " in " not in expr.lower():
            return expr.replace(op, inv, 1)
    # Composite / IN / otherwise -> logical negation.
    return f"not ({expr})"


def build_recipes(tbl: Table) -> list[dict]:
    """Recipes maintaining schema-implied behavior on the engine.

    The engine stamps Record.created_at natively, so no created_at recipe is needed.
    Real Postgres triggers (audit, RLS, FK cascades) are NOT auto-generated and must
    be ported deliberately per-module.
    """
    return []
