"""Orchestrate iico -> engine migration.

Usage:
  python3 migrate.py --workdir /tmp/migrate --users            # import auth users
  python3 migrate.py --workdir /tmp/migrate --tables a,b,c     # migrate tables
  python3 migrate.py --workdir /tmp/migrate --all               # users + core tables
  python3 migrate.py --workdir /tmp/migrate --tables classes --limit 2   # SMALL TEST

Checkpoint: each table's success is recorded in `<workdir>/done.json`; reruns
skip completed tables unless `--force`.
"""

import argparse
import json
import os
import subprocess
import sys

from config import BATCH_SIZE, BOARD, ENGINE_BASE, OWNER_TOKEN

HERE = os.path.dirname(os.path.abspath(__file__))
CORE_TABLES = [
    "profiles",
    "classes",
    "learners",
    "subjects",
    "guardians",
    "user_roles",
    "term_results",
    "attendance",
    "employees",
    "report_cards",
    "teacher_assignments",
    "parent_learner_links",
    "fee_structures",
    "fee_assignments",
    "payments",
]

# Tables skipped during the core pass (need schemas/recipes to be useful).
SKIP_CORE = {"job_submissions"}


def done_path(workdir: str) -> str:
    return os.path.join(workdir, "done.json")


def read_done(workdir: str) -> set:
    p = done_path(workdir)
    if os.path.exists(p):
        with open(p) as f:
            return set(json.load(f))
    return set()


def write_done(workdir: str, done: set) -> None:
    with open(done_path(workdir), "w") as f:
        json.dump(sorted(done), f, indent=1)


def run(workdir: str, module: str, args: list[str], force: bool = False) -> None:
    cmd = [sys.executable, os.path.join(HERE, module), workdir] + args
    if force:
        cmd.append("--force")
    print(f"  [step] {module} {' '.join(args)}", flush=True)
    subprocess.run(cmd, check=True)


def apply_limit(table: str, limit: int | None, workdir: str) -> None:
    """If --limit N given, replace a table dump with only the first N rows."""
    if limit is None:
        return
    path = os.path.join(workdir, "raw", f"{table}.json")
    with open(path) as f:
        rows = json.load(f)
    if len(rows) > limit:
        with open(path, "w") as f:
            json.dump(rows[:limit], f)
        print(f"  [test] {table}: limited to {limit} rows")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--workdir", default="/tmp/migrate_iico")
    ap.add_argument("--tables", default="")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--users", action="store_true")
    ap.add_argument("--limit", type=int, default=None, help="test mode: N rows/table")
    ap.add_argument("--force", action="store_true")
    args = ap.parse_args()

    os.makedirs(os.path.join(args.workdir, "raw"), exist_ok=True)
    done = read_done(args.workdir)

    tables = [t.strip() for t in args.tables.split(",") if t.strip()]
    if args.all:
        tables = [t for t in CORE_TABLES if t not in SKIP_CORE]

    # ---- users first ----
    if args.users or args.all:
        run(args.workdir, "extract.py", ["__users__"], args.force)
        run(args.workdir, "loader.py", ["__users__"], args.force)

    # ---- tables ----
    for table in tables:
        if table in done and not args.force:
            print(f"skip (done): {table}")
            continue
        print(f"=== {table} ===")
        run(args.workdir, "extract.py", [table], args.force)
        apply_limit(table, args.limit, args.workdir)
        run(args.workdir, "transform.py", [table], args.force)
        run(args.workdir, "loader.py", [table], args.force)
        if args.limit is None:
            done.add(table)
            write_done(args.workdir, done)

    print("\nsummary:")
    print(f"  board:       {BOARD}")
    print(f"  engine base: {ENGINE_BASE}")
    print(f"  completed:   {sorted(done)}")


if __name__ == "__main__":
    main()
