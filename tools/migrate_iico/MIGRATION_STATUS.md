# IICO Migration Status

## Overview
Migration of IICO data from Supabase to the serverless engine. This document tracks the progress of migrating tables from the source Supabase instance to the engine.

## Migration Tools
- `tools/migrate_iico/extract.py` - Extract tables from source Supabase via SSH
- `tools/migrate_iico/transform.py` - Transform data (UUID→email rewrite, null stripping)
- `tools/migrate_iico/loader.py` - Load data into engine via REST API
- `tools/migrate_iico/migrate.py` - Orchestrate the migration pipeline

## Current Migration Status

| # | Table | Source Rows | Engine Rows | Status | Notes |
|---|-------|-------------|-------------|--------|-------|
| 1 | profiles | 343 | 343 | ✅ Done |
| 2 | user_roles | 344 | 344 | ✅ Done |
| 3 | learners | 294 | 294 | ✅ Done |
| 3 | term_results | 2334 | 2334 | ✅ Done |
| 4 | classes | 7 | 7 | ✅ Done |
| 5 | subjects | 26 | 26 | ✅ Done |
| 6 | guardians | 42 | 42 | ✅ Done |
| 7 | attendance | 520 | 520 | ✅ Done |
| 8 | report_cards | 468 | 468 | ✅ Done |
| 9 | employees | 44 | 44 | ✅ Done |
| 9 | fee_payments | 2 | 2 | ✅ Done |
| 10 | teacher_assignments | 53 | 53 | ✅ Done |
| 9 | parent_learner_links | 0 | 0 | ✅ Done (empty) |

**Total tables migrated: All tables synced (64 tables in engine as of 2026-08-17).** The original 14 core tables plus all remaining USER_REF_COLUMNS tables and additional large tables (fee_structures, scheme_of_work, assets 1886, in_app_notifications 12035, dormitory_residents 206, visit_reasons 370, student_account_credentials 294, emis_learner_export 286, inventory_details/items/stock ~238, email_logs 166, and more) have been migrated from Supabase to the engine.

### Remaining Tables to Migrate
None. All source tables with data have been migrated and verified. The `in_app_notifications` table (12,035 rows) was fully synced via a background tmux run with a larger timeout and batch size.

**To add these tables to the migration, use:**
```bash
python3 migrate.py --workdir /tmp/migrate_iico --tables <table_name>
```

### How to Add a New Table

1. **Extract**: `python3 migrate.py --workdir /tmp/migrate_iico --tables <table_name>`
2. **Transform**: The script automatically transforms UUID→email for user-ref columns
3. **Load**: `python3 migrate.py --workdir /tmp/migrate_iico --tables <table_name>`
4. **Verify**: Check engine counts match source

### Migration Checklist

- [x] profiles (343 rows)
- [x] user_roles (344 rows)
- [x] learners (294 rows)
- [x] term_results (2334 rows)
- [x] classes (7 rows)
- [x] subjects (26 rows)
- [x] guardians (42 rows)
- [x] attendance (520 rows)
- [x] report_cards (468 rows)
- [x] employees (44 rows)
- [x] fee_payments (2 rows)
- [x] teacher_assignments (53 rows)
- [x] profiles (343 rows, re-migrated)
- [x] user_roles (344 rows, re-migrated)
- [x] learners (294 rows, re-migrated)
- [x] term_results (2334 rows, re-migrated)
- [x] fee_structures (23 rows)
- [x] scheme_of_work (6 rows)
- [x] in_app_notifications (12035 rows)
- [x] all other USER_REF_COLUMNS tables
- [x] additional large tables (visit_reasons, student_account_credentials, emis_learner_export, inventory_*, email_logs)

### Engine Status

All 14 core tables now exist on the engine with the correct schema and data. The engine has been rebuilt with the performance fixes (FTS removal, transaction batching, indexed unique_key lookups).

### Known Issues

- **`next_seq` O(n²) performance**: Each insert does a full table scan. For small tables (≤500 rows), this is acceptable (~0.5s per 10 rows). For larger tables, consider adding a sequence counter.
- **`find_unique_holder` full-table scan**: Each insert scans all rows to check for unique key conflicts. This is being addressed in a separate engine fix.
- **Spatial/FTS index**: The `wb_records` FTS index caused ~4s/insert time. This has been removed/off.

### Engine Fixes Required (for the fixing agent)

1. **`find_unique_holder`** in `crates/engine/src/crud.rs` - Change from full-table scan to indexed query
2. **`next_seq`** in `crates/engine/src/crud.rs` - Use a sequence counter instead of full table scan
3. **FTS index** in `wb_records` - Remove or disable the Tantivy FTS index

### How to Continue Migration

To add more tables:

1. Add the table to `USER_REF_COLUMNS` in `config.py` if it has user-reference columns
2. Add the table to `CORE_TABLES` in `migrate.py` if you want it included in `--all`
3. Run: `python3 migrate.py --workdir /tmp/migrate_iico --tables <table_name>`
4. Verify counts match source using: `curl http://127.0.0.1:7070/api/srv/<BOARD>/tables/<table>/aggregate?op=count`

### Migration Commands

```bash
# Test small data (3 rows)
python3 migrate.py --workdir /tmp/migrate_iico --tables <table> --limit 3

# Migrate a single table
python3 migrate.py --workdir /tmp/migrate_iico --tables <table_name>

# Migrate all core tables
python3 migrate.py --workdir /tmp/migrate_iico --all

# Migrate all tables (core + additional)
python3 migrate.py --workdir /tmp/migrate_iico --all --users
```
<tool_call>
<function=bash>
<parameter=command>
cd /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine/tools/migrate_iico; ls -la