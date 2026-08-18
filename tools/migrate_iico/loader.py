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


def load_table(
    workdir: str,
    table: str,
    count_check: bool = True,
    migrate: bool = False,
    batch_size: int | None = None,
) -> None:
    path = os.path.join(workdir, "transformed", f"{table}.json")
    with open(path) as f:
        rows = json.load(f)
    if migrate:
        # Clean reload: drop any partial data, then fast-import (skips per-row
        # unique/recipe/hook/audit work; run graph sync afterwards).
        if table_exists(table):
            api("DELETE", f"/tables/{table}")
            flush_print(f"dropped table: {table} (clean reload)")
        ensure_table(table)
    else:
        ensure_table(table)
    bs = batch_size or BATCH_SIZE
    total = 0
    for i in range(0, len(rows), bs):
        batch = rows[i : i + bs]
        q = "?migrate=1" if migrate else "?upsert=1"
        resp = api("POST", f"/tables/{table}/bulk{q}", {"records": batch})
        seqs = resp.get("seqs", [])
        total += len(seqs)
        flush_print(f"  {table}: loaded {total}/{len(rows)}")
        time.sleep(0.05)
    if count_check and total != len(rows):
        raise RuntimeError(f"{table}: loaded {total} but source has {len(rows)}")
    flush_print(f"loaded {table}: {total} rows")


def verify_references(workdir: str, table: str) -> None:
    """Referential check: every `*_id` value in `table` must exist in the
    target table's `id` set (per GRAPH_EDGES). Fail loudly on orphans so a
    silent-hallucination migration is caught here, not in the app."""
    from config import GRAPH_EDGES

    refs = GRAPH_EDGES.get(table, {})
    if not refs:
        return
    path = os.path.join(workdir, "transformed", f"{table}.json")
    with open(path) as f:
        rows = json.load(f)
    for col, target in refs.items():
        # Load the target's id set from its transformed dump (source truth).
        tpath = os.path.join(workdir, "transformed", f"{target}.json")
        if not os.path.exists(tpath):
            flush_print(f"  [verify] {table}.{col} -> {target}: target dump missing, skipped")
            continue
        with open(tpath) as f:
            target_ids = {str(r.get("id")) for r in json.load(f) if r.get("id")}
        orphans = {str(r[col]) for r in rows if r.get(col) and str(r[col]) not in target_ids}
        if orphans:
            sample = sorted(orphans)[:5]
            raise RuntimeError(
                f"{table}.{col}: {len(orphans)} orphan refs not in {target}.id (e.g. {sample})"
            )
        flush_print(f"  [verify] {table}.{col} -> {target}: {len(rows)} refs all resolve")


def graph_sync(board: str, table: str) -> None:
    """Materialize RELATED_* edges for a table's `*_id` refs via the engine's
    graph sync (idempotent). Returns after the sync call; edges are created in
    the board's HelixDB tenant."""
    from config import GRAPH_EDGES

    if table not in GRAPH_EDGES:
        return
    url = f"{ENGINE_BASE}/mcp"
    body = json.dumps({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "graph.sync", "arguments": {"board": board, "table": table}},
    }).encode()
    req = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=REQ_TIMEOUT) as resp:
            result = json.loads(resp.read().decode())
        text = result.get("result", {}).get("content", [{}])[0].get("text", "")
        flush_print(f"  [graph] sync {table}: {text[:200]}")
    except Exception as e:  # noqa: BLE001
        flush_print(f"  [graph] sync {table} FAILED: {e}")


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
    migrate = "--migrate" in sys.argv
    batch_size = None
    for a in sys.argv:
        if a.startswith("--batch="):
            batch_size = int(a.split("=", 1)[1])
    if table == "__users__":
        path = os.path.join(workdir, "raw", "_users.json")
        with open(path) as f:
            load_users(workdir, json.load(f))
    elif table.startswith("__verify__"):
        verify_references(workdir, table[len("__verify__"):])
    elif table.startswith("__sync__"):
        graph_sync(BOARD, table[len("__sync__"):])
    else:
        load_table(workdir, table, migrate=migrate, batch_size=batch_size)


if __name__ == "__main__":
    main()
