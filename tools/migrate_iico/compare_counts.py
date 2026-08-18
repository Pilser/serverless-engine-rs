#!/usr/bin/env python3
"""Compare local source row counts vs live engine row counts, table by table.

Outputs three groups:
  MATCH   — source == engine (complete)
  PARTIAL — engine > 0 but < source (load was cut off / --limit test)
  MISSING — engine == 0 or table absent (not loaded)

Only row counts are read from disk; never the bulk JSON contents. Run against
the daemon (see CONTINUATION_GUIDE.md), e.g.:

    python3 compare_counts.py                    # all tables in data/raw
    python3 compare_counts.py --only partial     # just incomplete ones
"""

import argparse
import glob
import json
import os
import sys
import urllib.request

from config import BOARD, ENGINE_BASE, OWNER_TOKEN

HERE = os.path.dirname(os.path.abspath(__file__))
RAW_DIR = os.path.join(HERE, "data", "raw")
REQ_TIMEOUT = 120


def source_count(table: str) -> int:
    """len() of the raw dump — cheap, no content loaded."""
    with open(os.path.join(RAW_DIR, f"{table}.json")) as f:
        return len(json.load(f))


def engine_count(table: str) -> int:
    """GET /tables/<t>/aggregate?op=count on the live daemon."""
    url = f"{ENGINE_BASE}/api/srv/{BOARD}/tables/{table}/aggregate?op=count"
    req = urllib.request.Request(
        url,
        headers={"Authorization": f"Bearer {OWNER_TOKEN}"},
    )
    try:
        with urllib.request.urlopen(req, timeout=REQ_TIMEOUT) as resp:
            data = json.loads(resp.read().decode())
        rows = data.get("aggregate", [])
        return rows[0].get("value", 0) if rows else 0
    except urllib.error.HTTPError as e:
        if e.code in (400, 404):
            return -1  # table absent in the engine
        raise RuntimeError(f"{table}: HTTP {e.code}: {e.read().decode()[:200]}")
    except urllib.error.URLError as e:
        raise RuntimeError(f"{table}: network error: {e.reason}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", choices=["match", "partial", "missing", "all"], default="all")
    args = ap.parse_args()

    tables = sorted(os.path.basename(p)[: -len(".json")] for p in glob.glob(os.path.join(RAW_DIR, "*.json")))
    if not tables:
        print(f"no raw dumps found in {RAW_DIR}", file=sys.stderr)
        sys.exit(1)

    match, partial, missing = [], [], []
    for t in tables:
        src = source_count(t)
        try:
            eng = engine_count(t)
        except RuntimeError as e:
            print(f"  [error] {e}", file=sys.stderr)
            continue
        if eng < 0:
            missing.append((t, src))
        elif src == eng:
            match.append((t, src))
        elif eng == 0:
            missing.append((t, src))
        else:
            partial.append((t, src, eng))

    print(f"board: {BOARD}   engine: {ENGINE_BASE}\n")

    if args.only in ("match", "all"):
        print(f"== MATCH ({len(match)}): source == engine ==")
        for t, s in match:
            print(f"  {t}: {s}")

    if args.only in ("partial", "all"):
        print(f"\n== PARTIAL ({len(partial)}): 0 < engine < source — needs --migrate reload ==")
        for t, s, e in sorted(partial, key=lambda x: -x[1]):
            print(f"  {t}: source {s}, engine {e} (missing {s - e})")

    if args.only in ("missing", "all"):
        print(f"\n== MISSING ({len(missing)}): engine 0 or absent — needs load ==")
        for t, s in sorted(missing, key=lambda x: -x[1]):
            print(f"  {t}: source {s}")

    print(f"\nsummary: match={len(match)} partial={len(partial)} missing={len(missing)} total={len(tables)}")


if __name__ == "__main__":
    main()
