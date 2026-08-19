#!/usr/bin/env python3
"""Rewrite absolute source Supabase storage URLs inside engine records to the
engine's own /file endpoint, so rows point at where the object bytes WILL live
on the board's object store (and serve immediately for already-pushed files).

Rewrite rule:
  https://<host>/supabase/storage/v1/object/public/<bucket>/<path>
    -> /api/srv/<board>/file?table=<bucket>&file=<urlencoded path>

Scans these source-tables (from data/raw/ analysis) for URL-bearing fields:
  user_signatures.image_url, store_orders.*_signature_url,
  print_orders.file_url, learners.photo_url, staff_letters.body (embedded).

The rewrite is idempotent (already-rewritten URLs are skipped).

Usage:
  python3 rewrite_storage_urls.py            # dry run
  python3 rewrite_storage_urls.py --apply    # patch the engine
"""

import json
import os
import re
import sys
import urllib.parse
import urllib.request

ENGINE = os.environ.get("ENGINE_BASE", "http://127.0.0.1:7070")
BOARD = os.environ.get("BOARD", "b_7z5lr8zaurkd0000")
TOKEN = os.environ.get("OWNER_TOKEN", BOARD)

URL_RE = re.compile(r"https?://[^/\s]+/supabase/storage/v1/object/public/([^?\s\"']+)")
SRV_PREFIX = f"/api/srv/{BOARD}/file"


def _request(method: str, path: str, body: dict | None = None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        f"{ENGINE}{path}",
        data=data,
        method=method,
        headers={"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read())


def rewrite_url(s: str) -> str:
    def repl(m):
        path = m.group(1)
        bucket, _, rest = path.partition("/")
        return f"{SRV_PREFIX}?table={urllib.parse.quote(bucket)}&file={urllib.parse.quote(rest)}"
    return URL_RE.sub(repl, s)


def find_rows(table: str, limit: int = 2000) -> list[dict]:
    q = f"/api/srv/{BOARD}/tables/{urllib.parse.quote(table)}/query?limit={limit}"
    d = _request("GET", q)
    return d.get("records", [])


def patch_row(table: str, seq: int, patch: dict) -> bool:
    try:
        _request("PATCH", f"/api/srv/{BOARD}/tables/{urllib.parse.quote(table)}/records/{seq}", patch)
        return True
    except Exception as e:
        print(f"    !! seq {seq}: {e}", flush=True)
        return False


def main():
    apply = "--apply" in sys.argv
    fields = {
        "user_signatures": ["image_url"],
        "store_orders": ["applicant_signature_url", "director_signature_url", "authorizer_signature_url"],
        "print_orders": ["file_url"],
        "learners": ["photo_url"],
        "staff_letters": ["body"],
    }
    print(f"== rewrite storage URLs -> {SRV_PREFIX}  (apply={apply})", flush=True)
    total = fixed = 0
    for table, flds in fields.items():
        rows = find_rows(table)
        print(f"== {table}: {len(rows)} rows", flush=True)
        for r in rows:
            p = r.get("payload", {})
            seq = r.get("seq")
            newp = dict(p)
            changed = False
            for f in flds:
                v = newp.get(f)
                if isinstance(v, str) and "supabase/storage" in v:
                    nv = rewrite_url(v)
                    if nv != v:
                        newp[f] = nv
                        changed = True
            if changed:
                total += 1
                if apply:
                    ok = patch_row(table, seq, newp)
                    fixed += 1 if ok else 0
                    print(f"  + seq {seq}: rewrote {[f for f in flds if newp.get(f)!=p.get(f)]}", flush=True)
                else:
                    print(f"  ~ seq {seq}: would rewrite {[f for f in flds if newp.get(f)!=p.get(f)]}", flush=True)
    print(f"== done: {total} rows to rewrite, {fixed} patched" + ("" if apply else " (dry run — pass --apply)"), flush=True)


if __name__ == "__main__":
    main()
