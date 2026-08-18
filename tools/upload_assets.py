"""Upload a built frontend dist/ tree to the engine tenant as static assets."""
from __future__ import annotations

import argparse
import mimetypes
import os
import urllib.request

from pathlib import Path


def main() -> int:
    p = argparse.ArgumentParser(description="Upload a dist/ tree to engine assets")
    p.add_argument("--dist", required=True, help="path to built dist directory")
    p.add_argument("--base", required=True, help="engine asset base, e.g. http://127.0.0.1:7070/api/srv/<board>")
    p.add_argument("--token", required=True, help="owner board id (Bearer token = board id) or API key")
    args = p.parse_args()

    dist = Path(args.dist)
    if not dist.is_dir():
        print(f"dist dir not found: {dist}")
        return 1

    uploaded = 0
    errors: list[str] = []
    for root, _dirs, files in os.walk(dist):
        for fname in files:
            path = Path(root) / fname
            rel = path.relative_to(dist).as_posix()
            ctype = mimetypes.guess_type(str(path))[0] or "application/octet-stream"
            data = path.read_bytes()
            url = f"{args.base}/assets/{rel}"
            req = urllib.request.Request(
                url,
                data=data,
                method="PUT",
                headers={
                    "Authorization": f"Bearer {args.token}",
                    "Content-Type": ctype,
                },
            )
            try:
                with urllib.request.urlopen(req) as resp:
                    if resp.status >= 400:
                        errors.append(f"{rel}: HTTP {resp.status}")
                    else:
                        uploaded += 1
                        if uploaded <= 5 or rel.endswith("index.html"):
                            print(f"  {rel} ({ctype})")
            except Exception as e:  # noqa: BLE001
                errors.append(f"{rel}: {e}")

    print(f"\nuploaded: {uploaded}  errors: {len(errors)}")
    for e in errors[:20]:
        print("  ERR", e)
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
