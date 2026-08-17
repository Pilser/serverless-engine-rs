"""Extract tables from the source Supabase DB over SSH as JSON rows.

Each table is dumped to `<workdir>/raw/<table>.json` (an array of row objects).
Calling `extract` is idempotent: existing dumps are reused unless `--force`.
"""

import json
import os
import subprocess
import sys

from config import (
    SRC_HOST,
    SRC_USER,
    SRC_SSH_PASS,
    SRC_DB_CONTAINER,
    SRC_DB_USER,
    SRC_DB_NAME,
)

SSH_TIMEOUT = 60  # seconds


def log(msg: str) -> None:
    print(msg, flush=True)


def ssh_run(sql: str) -> str:
    """Run a single psql query inside the source container, return stdout."""
    psql = (
        f"docker exec -i {SRC_DB_CONTAINER} psql -U {SRC_DB_USER} -d {SRC_DB_NAME} "
        f"-t -A -c \"{sql}\""
    )
    cmd = [
        "sshpass", "-p", SRC_SSH_PASS,
        "ssh", "-o", "StrictHostKeyChecking=no", "-o", "ConnectTimeout=10",
        f"{SRC_USER}@{SRC_HOST}", psql,
    ]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=SSH_TIMEOUT)
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"ssh/psql timed out after {SSH_TIMEOUT}s")
    if proc.returncode != 0:
        raise RuntimeError(f"ssh/psql failed: {proc.stderr.strip()}")
    return proc.stdout


def table_json(table: str, where: str = "") -> list:
    """Return the table rows as a Python list of dicts."""
    cond = f" where {where}" if where else ""
    sql = (
        f"select coalesce(json_agg(t), '[]') from ("
        f"  select * from public.{table}{cond}"
        f") t"
    )
    out = ssh_run(sql)
    out = out.strip().replace("\r", "")
    return json.loads(out)


def pull_table(workdir: str, table: str, force: bool, where: str = "") -> str:
    """Extract one table to `<workdir>/raw/<table>.json`. Returns the file path."""
    os.makedirs(os.path.join(workdir, "raw"), exist_ok=True)
    path = os.path.join(workdir, "raw", f"{table}.json")
    if os.path.exists(path) and not force:
        log(f"reuse {table}: {path}")
        return path
    log(f"extracting {table} ...")
    rows = table_json(table, where)
    with open(path, "w") as f:
        json.dump(rows, f)
    log(f"extracted {table}: {len(rows)} rows -> {path}")
    return path


def load_table(workdir: str, table: str) -> list:
    """Read a previously extracted table dump."""
    path = os.path.join(workdir, "raw", f"{table}.json")
    with open(path) as f:
        return json.load(f)


def pull_users(workdir: str, force: bool) -> str:
    """Extract auth.users (email, password_hash, role) to `<workdir>/raw/_users.json`."""
    os.makedirs(os.path.join(workdir, "raw"), exist_ok=True)
    path = os.path.join(workdir, "raw", "_users.json")
    if os.path.exists(path) and not force:
        return path
    sql = (
        "select coalesce(json_agg(t), '[]') from ("
        "  select u.email, u.encrypted_password as password_hash,"
        "         coalesce(p.role, 'teacher') as role"
        "  from auth.users u left join public.profiles p on p.email = u.email"
        ") t"
    )
    rows = json.loads(ssh_run(sql).strip().replace("\r", ""))
    with open(path, "w") as f:
        json.dump(rows, f)
    print(f"extracted users: {len(rows)} -> {path}")
    return path


def main() -> None:
    workdir = sys.argv[1] if len(sys.argv) > 1 else "."
    tables = sys.argv[2].split(",") if len(sys.argv) > 2 else []
    force = "--force" in sys.argv
    for t in tables:
        t = t.strip()
        if t == "__users__":
            pull_users(workdir, force)
        else:
            pull_table(workdir, t, force)


if __name__ == "__main__":
    main()
