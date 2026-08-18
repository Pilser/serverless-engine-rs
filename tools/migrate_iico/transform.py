"""Transform source rows for engine load.

Responsibilities:
1. Drop columns listed in DROP_COLUMNS.
2. Strip null values (JSON "absent" == null).
3. Serialize to the bulk-load JSON format.

NOTE: `*_id` UUID references are KEPT as-is (no UUID->email rewrite). The
frontend (`deploy/iico`) joins by matching UUID to the target table's `id`, and
the graph model turns these refs into `RELATED_*` edges via `graph sync`.
"""

import json
import os
import sys

from config import DROP_COLUMNS
from extract import load_table


def flush_print(*a, **k):
    k["flush"] = True
    print(*a, **k)


class Transform:
    def __init__(self, workdir: str, force: bool = False):
        self.workdir = workdir

    def transform_rows(self, table: str, rows: list) -> list:
        drop = DROP_COLUMNS.get(table, [])
        out = []
        for row in rows:
            for col in drop:
                row.pop(col, None)
            # Strip null values: JSON "absent" == null.
            cleaned = {k: v for k, v in row.items() if v is not None}
            out.append(cleaned)
        return out

    def prepare(self, table: str) -> list:
        rows = load_table(self.workdir, table)
        return self.transform_rows(table, rows)


def write_transformed(workdir: str, table: str, rows: list) -> str:
    """Persist transformed rows to `<workdir>/transformed/<table>.json`."""
    os.makedirs(os.path.join(workdir, "transformed"), exist_ok=True)
    path = os.path.join(workdir, "transformed", f"{table}.json")
    with open(path, "w") as f:
        json.dump(rows, f)
    return path


def main() -> None:
    workdir = sys.argv[1] if len(sys.argv) > 1 else "."
    table = sys.argv[2]
    force = "--force" in sys.argv
    t = Transform(workdir, force)
    rows = t.prepare(table)
    path = write_transformed(workdir, table, rows)
    flush_print(json.dumps(rows[:3], indent=1, default=str))
    flush_print(f"... total {len(rows)} rows -> {path}")


if __name__ == "__main__":
    main()
