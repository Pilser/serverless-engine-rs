#!/usr/bin/env python3
"""Dump Supabase STORAGE files to local disk, then push them into the engine's
per-tenant object store (files namespace) so the app serves them itself.

Mirrors how the DB data was scraped into tools/migrate_iico/data/:

  1. Query the source storage DB (via SSH + psql) for every object:
     bucket_id, name (path), size.
  2. Download each object through the source's public URL
     (https://sised.sc.ug/supabase/storage/v1/object/public/<bucket>/<path>)
     into data/storage/<bucket>/<path>.
  3. Upload every file to the engine board: POST /upload?table=<bucket>
     (the engine stores blobs under <board>/files/ + a record in the
     <bucket> table). The frontend's storage.getPublicUrl(bucket, path)
     then serves it from the engine via /file?table=<bucket>&file=<path>.

Usage:
  python3 dump_storage.py            # dump + push
  python3 dump_storage.py --dump-only
  python3 dump_storage.py --push-only
"""

import argparse
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "data")
STORAGE_DIR = os.path.join(DATA, "storage")

SRC = {
    "host": "192.168.88.101",
    "user": "iico",
    "pass": "12345678",
    "db_container": "supabase-db",
    "db_user": "supabase_admin",
}
PUBLIC_BASE = "https://sised.sc.ug/supabase/storage/v1/object/public"

BOARD = os.environ.get("BOARD", "b_7z5lr8zaurkd0000")
ENGINE = os.environ.get("ENGINE_BASE", "http://127.0.0.1:7070")


def ssh_psql(sql: str) -> str:
    cmd = [
        "sshpass", "-p", SRC["pass"],
        "ssh", "-o", "StrictHostKeyChecking=no", "-o", "ConnectTimeout=10",
        f"{SRC['user']}@{SRC['host']}",
        f"docker exec -i {SRC['db_container']} psql -U {SRC['db_user']} -d postgres -t -A -F'|'",
    ]
    proc = subprocess.run(cmd, input=sql, capture_output=True, text=True, timeout=120)
    if proc.returncode != 0:
        raise RuntimeError(f"psql failed: {proc.stderr.strip()}")
    return proc.stdout


def list_objects() -> list[tuple[str, str, int]]:
    out = ssh_psql("select bucket_id, name, (metadata->>'size')::int from storage.objects")
    rows = []
    for line in out.strip().splitlines():
        parts = line.split("|")
        if len(parts) >= 3 and parts[0]:
            rows.append((parts[0], parts[1], int(parts[2] or 0)))
    return rows


def download(bucket: str, path: str, size: int) -> str:
    """Fetch one object from the source public URL into data/storage.

    The source storage volume was wiped; surviving bytes are only in an edge
    cache behind sised.sc.ug, which 500s intermittently (round-robin across two
    replicas, one of which still serves the file). Retry with a browser UA.
    """
    rel = os.path.join(bucket, path)
    dest = os.path.join(STORAGE_DIR, rel)
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    if os.path.exists(dest) and os.path.getsize(dest) == size:
        return dest  # already dumped
    url = f"{PUBLIC_BASE}/{bucket}/{path}"
    ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120 Safari/537.36"
    for attempt in range(1, 9):
        req = urllib.request.Request(url, headers={"User-Agent": ua})
        try:
            with urllib.request.urlopen(req, timeout=8) as r:
                data = r.read()
            with open(dest, "wb") as f:
                f.write(data)
            return dest
        except Exception:
            if attempt == 8:
                print(f"  !! {rel}: lost on source (not in edge cache)", flush=True)
                return ""
            time.sleep(0.6)
    return ""


def push_to_engine(bucket: str, path: str, local: str) -> bool:
    """Upload a local file to the engine board via POST /upload?table=<bucket>.

    X-Filename is sent RAW (not URL-encoded): the engine stores it verbatim as
    the record's `name`, and the frontend's getPublicUrl() re-encodes it in the
    /file query param so the server-side lookup by `name` matches.
    """
    url = f"{ENGINE}/api/srv/{BOARD}/upload?table={urllib.parse.quote(bucket)}"
    with open(local, "rb") as f:
        body = f.read()
    req = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={
            "X-Filename": path,
            "Content-Type": "application/octet-stream",
            "Authorization": f"Bearer {BOARD}",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            d = json.loads(r.read())
            return d.get("ok", False)
    except Exception as e:
        print(f"  !! push {path}: {e}")
        return False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dump-only", action="store_true")
    ap.add_argument("--push-only", action="store_true")
    args = ap.parse_args()

    print("== listing storage objects from source ==")
    objects = list_objects()
    print(f"  {len(objects)} objects")
    total = sum(s for _, _, s in objects)
    print(f"  total {total/1024/1024:.1f} MB")

    if not args.push_only:
        print("== downloading into data/storage ==", flush=True)
        ok = 0
        with ThreadPoolExecutor(max_workers=8) as ex:
            futs = {ex.submit(download, b, p, s): (b, p) for b, p, s in objects}
            for fut in as_completed(futs):
                if fut.result():
                    ok += 1
                    if ok % 10 == 0:
                        print(f"  downloaded {ok}/{len(objects)}", flush=True)
        print(f"  downloaded {ok}/{len(objects)}", flush=True)

    if not args.dump_only:
        print("== pushing into engine board ==")
        pushed = 0
        for bucket, path, _ in objects:
            rel = os.path.join(bucket, path)
            local = os.path.join(STORAGE_DIR, rel)
            if os.path.exists(local) and push_to_engine(bucket, path, local):
                pushed += 1
                if pushed % 10 == 0:
                    print(f"  pushed {pushed}/{len(objects)}")
        print(f"  pushed {pushed}/{len(objects)}")

    print("== done ==")


if __name__ == "__main__":
    main()
