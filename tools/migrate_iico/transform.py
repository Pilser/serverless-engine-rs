"""Transform source rows for engine load.

Responsibilities:
1. Load the `auth.users` map (UUID -> lowercase email) so every user-reference
   column can be rewritten from UUID to the lowercase email the engine keys on.
2. Rewrite `USER_REF_COLUMNS` values on each table's rows (drop unknown UUIDs
   -> None, keep null as null).
3. Drop columns listed in DROP_COLUMNS.
4. Serialize to the bulk-load JSON format.
"""

import json
import os
import sys

from config import DROP_COLUMNS, USER_REF_COLUMNS
from extract import load_table


def flush_print(*a, **k):
    k["flush"] = True
    print(*a, **k)


class Transform:
    def __init__(self, workdir: str, force: bool = False):
        self.workdir = workdir
        self.uuid_to_email = {}
        cache = os.path.join(workdir, "raw", "_uuid_to_email.json")
        if force or not os.path.exists(cache):
            map_ = self._build_uuid_map()
            with open(cache, "w") as f:
                json.dump(map_, f)
        with open(cache) as f:
            self.uuid_to_email = json.load(f)

    def _build_uuid_map(self) -> dict:
        """auth.users: id (uuid) -> lowercase email."""
        from extract import ssh_run

        sql = (
            "select coalesce(json_agg(t), '[]') from ("
            "  select id, email from auth.users where id is not null"
            ") t"
        )
        rows = json.loads(ssh_run(sql).strip().replace("\r", ""))
        return {r["id"]: r["email"].lower() for r in rows if r.get("email")}

    def email_for(self, value):
        """Rewrite a single user-reference value (uuid -> lowercase email)."""
        if value is None:
            return None
        if value in self.uuid_to_email:
            return self.uuid_to_email[value]
        # Not a UUID reference (already an email or other key): lowercase emails.
        s = str(value)
        if "@" in s:
            return s.lower()
        return value

    def transform_rows(self, table: str, rows: list) -> list:
        drop = DROP_COLUMNS.get(table, [])
        ref_cols = USER_REF_COLUMNS.get(table, [])
        out = []
        for row in rows:
            for col in drop:
                row.pop(col, None)
            for col in ref_cols:
                if col in row:
                    row[col] = self.email_for(row.get(col))
            # Strip null values: JSON "absent" == null, and it avoids schema
            # mismatches on tables with strict column types (e.g. room: string).
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
