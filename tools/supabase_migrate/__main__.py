"""CLI: translate a Supabase migration folder into serverless-engine boards.

Usage:
  python -m supabase_migrate schema   MIGRATIONS_DIR   -> print table/column model
  python -m supabase_migrate boards   MIGRATIONS_DIR [--tables a,b,c] [--mcp URL]
  python -m supabase_migrate data     MIGRATIONS_DIR --table T [--file dump.json]
"""
from __future__ import annotations

import argparse
import json
import sys

from .genconfig import build_update_payload, build_validate_rules, build_recipes
from .importsql import extract_inserts, table_inserts_to_records
from .mcp_client import McpClient, McpError
from .sqlparse import load_migrations

CHAINED = {"_", "-", "--"}


def _pick_tables(model, arg: str | None) -> list[str]:
    if arg:
        return [t.strip() for t in arg.split(",") if t.strip()]
    return sorted(model.tables.keys())


def cmd_schema(args) -> int:
    model = load_migrations(args.migrations)
    out: dict = {"enums": model.enums}
    tables: dict = {}
    for name in sorted(model.tables.keys()):
        tbl = model.tables[name]
        tables[name] = {
            "columns": list(tbl.order),
            "pks": tbl.pks,
            "unique_keys": tbl.unique_keys,
            "checks": tbl.checks,
            "col_types": {c: tbl.columns[c].pg_type for c in tbl.order},
            "not_null": [c for c in tbl.order if tbl.columns[c].not_null],
            "enums": {c: tbl.columns[c].enum_vals for c in tbl.order if tbl.columns[c].is_enum},
            "fks": {c: tbl.columns[c].references for c in tbl.order if tbl.columns[c].references},
        }
    out["tables"] = tables
    print(json.dumps(out, indent=2))
    return 0


def cmd_boards(args) -> int:
    """Create ONE tenant board, then a table per SQL table inside it."""
    model = load_migrations(args.migrations)
    client = McpClient(args.mcp)
    tables = _pick_tables(model, args.tables)

    # Create the tenant board once.
    try:
        res = client.tool_call("apps.create", {"title": args.tenant or "iico_tenant"})
        board = res.get("board")
    except McpError as e:
        print(json.dumps({"error": f"create tenant board: {e}"}))
        return 1

    report: dict = {"tenant": board, "tables": [], "recipes": [], "errors": []}
    for name in tables:
        tbl = model.tables[name]
        # table config: schema + unique_key (+ validate on the table)
        tconfig = build_update_payload(tbl)
        validate = build_validate_rules(tbl)
        if validate:
            tconfig["validate"] = validate
        try:
            client.tool_call(
                "tables.create",
                {"board": board, "table": name, **tconfig},
            )
            report["tables"].append({"table": name})
        except McpError as e:
            report["errors"].append({"table": name, "step": "create_table", "error": str(e)})
            continue
        # recipes
        for recipe in build_recipes(tbl):
            try:
                client.tool_call(
                    "recipes.add",
                    {"board": board, "name": recipe["name"], "when": recipe["when"], "actions": recipe["actions"]},
                )
                report["recipes"].append({"table": name, "recipe": recipe["name"]})
            except McpError as e:
                report["errors"].append({"table": name, "step": "recipe", "error": str(e)})
    print(json.dumps(report, indent=2))
    return 0


def cmd_data(args) -> int:
    model = load_migrations(args.migrations)
    if args.table not in model.tables:
        print(json.dumps({"error": f"table '{args.table}' not in schema"}))
        return 1
    tbl = model.tables[args.table]
    client = McpClient(args.mcp)

    # Collect INSERTs across all migration files.
    found = 0
    imported = 0
    errors: list[dict] = []
    from pathlib import Path

    for p in sorted(Path(args.migrations).glob("*.sql")):
        if p.name.startswith("00000000000000_consolidated"):
            continue  # concatenation of the real migrations; avoid double-import
        sql = p.read_text(errors="replace")
        for tname, columns, rows in extract_inserts(sql):
            if tname != args.table:
                continue
            found += len(rows)
            for rec in table_inserts_to_records(tbl, columns, rows):
                try:
                    client.tool_call("records.submit", {"board": args.board, "table": args.table, "payload": rec})
                    imported += 1
                except McpError as e:
                    errors.append({"file": p.name, "error": str(e)})
    print(json.dumps({"table": args.table, "rows_found": found, "imported": imported, "errors": errors[:20], "error_count": len(errors)}))
    return 0


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="supabase_migrate")
    sub = p.add_subparsers(dest="cmd", required=True)

    ps = sub.add_parser("schema", help="Print parsed schema model as JSON")
    ps.add_argument("migrations")
    ps.set_defaults(fn=cmd_schema)

    pb = sub.add_parser("boards", help="Create one tenant board + a table per SQL table")
    pb.add_argument("migrations")
    pb.add_argument("--tables")
    pb.add_argument("--tenant", help="Tenant board title (default iico_tenant)")
    pb.add_argument("--mcp", default="http://127.0.0.1:7070/mcp")
    pb.set_defaults(fn=cmd_boards)

    pd = sub.add_parser("data", help="Import static INSERT data into a tenant board table")
    pd.add_argument("migrations")
    pd.add_argument("--table", required=True)
    pd.add_argument("--board", required=True, help="Tenant board id")
    pd.add_argument("--mcp", default="http://127.0.0.1:7070/mcp")
    pd.set_defaults(fn=cmd_data)

    args = p.parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
