"""Load transformed rows into the serverless engine.

Modes:
- Data tables: `POST /tables/<t>/bulk` (records array). Table is created first
  if missing.
- Users: `POST /auth/signup` with `password_hash` (dual-mode), skipping users
  that already exist. Passwords come from the `auth_users` dump (see load_users).

Every table is loaded in batches of BATCH_SIZE and the loaded count is compared
to the source row count; mismatches fail loudly.
"""

import json
import os
import sys
import time
import urllib.error
import urllib.request

from config import (
    BATCH_SIZE,
    BOARD,
    ENGINE_BASE,
    OWNER_TOKEN,
    RECREATE_TABLES,
    SKIP_EXISTING_USERS,
)

REQ_TIMEOUT = 60


def flush_print(*a, **k):
    k["flush"] = True
    print(*a, **k)  # seconds


def api(method: str, path: str, body: dict | None = None, query: str = "") -> dict:
    url = f"{ENGINE_BASE}/api/srv/{BOARD}{path}"
    if query:
        url += query
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {OWNER_TOKEN}",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=REQ_TIMEOUT) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        detail = e.read().decode()[:300]
        raise RuntimeError(f"{method} {url} -> {e.code}: {detail}")
    except urllib.error.URLError as e:
        raise RuntimeError(f"{method} {url} -> network error: {e.reason}")


def table_exists(table: str) -> bool:
    try:
        api("GET", f"/tables/{table}")
        return True
    except RuntimeError:
        return False


def ensure_table(table: str) -> None:
    if table in RECREATE_TABLES and table_exists(table):
        api("DELETE", f"/tables/{table}")
        flush_print(f"recreated table: {table} (dropped stale strict schema)")
    if table_exists(table):
        return
    # Permissive schema: accept any fields, nothing required.
    schema = {"type": "object", "additionalProperties": True}
    api("POST", "/tables", {"name": table, "schema": schema, "unique_key": "$.id"})
    flush_print(f"created table: {table} (permissive schema, unique_key=$.id)")


def load_table(workdir: str, table: str, count_check: bool = True) -> None:
    path = os.path.join(workdir, "transformed", f"{table}.json")
    with open(path) as f:
        rows = json.load(f)
    ensure_table(table)
    total = 0
    for i in range(0, len(rows), BATCH_SIZE):
        batch = rows[i : i + BATCH_SIZE]
        resp = api("POST", f"/tables/{table}/bulk?upsert=1", {"records": batch})
        seqs = resp.get("seqs", [])
        total += len(seqs)
        flush_print(f"  {table}: loaded {total}/{len(rows)}")
        time.sleep(0.05)
    if count_check and total != len(rows):
        raise RuntimeError(f"{table}: loaded {total} but source has {len(rows)}")
    flush_print(f"loaded {table}: {total} rows")


def load_users(workdir: str, users: list[dict]) -> None:
    """Import auth.users (email + bcrypt password_hash + role) via dual signup."""
    done = 0
    skipped = 0
    for u in users:
        email = (u.get("email") or "").strip().lower()
        if not email:
            continue
        role = (u.get("role") or "teacher").strip().lower() or "teacher"
        phash = u.get("password_hash")
        body = {"email": email, "role": role}
        if phash:
            body["password_hash"] = phash
        else:
            body["password"] = "MigrationTempPass2026"
        try:
            api("POST", "/auth/signup", body)
            done += 1
        except RuntimeError as e:
            if "already exists" in str(e) and SKIP_EXISTING_USERS:
                skipped += 1
            else:
                raise
        flush_print(f"  users: imported {done}, skipped {skipped} (total {done+skipped}/{len(users)})")
    flush_print(f"users: done={done} skipped={skipped}")


def main() -> None:
    workdir = sys.argv[1]
    table = sys.argv[2]
    if table == "__users__":
        path = os.path.join(workdir, "raw", "_users.json")
        with open(path) as f:
            load_users(workdir, json.load(f))
    else:
        load_table(workdir, table)


if __name__ == "__main__":
    main()
