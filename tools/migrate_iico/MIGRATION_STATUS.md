# IICO → HelixDB Migration Status

> **LIVE checklist.** This supersedes the turso-era status doc. The storage
> backend is **HelixDB** (one tenant per board; records = nodes, `*_id` refs =
> edges). Board: **`b_7z5lr8zaurkd0000`**. See `MIGRATION_STRATEGY.md` for the
> graph-first plan and `README.md` for the pipeline API.

## Migration tools

- `tools/migrate_iico/extract.py` — SSH + psql JSON dump from source Supabase
- `tools/migrate_iico/transform.py` — null-strip, drop columns (keeps source UUIDs)
- `tools/migrate_iico/loader.py` — create table + bulk upsert + verify refs + graph sync
- `tools/migrate_iico/migrate.py` — orchestrate (checkpointed in `<workdir>/done.json`)

## How to run

```bash
# One table (extract → transform → load → verify refs → graph sync)
python3 migrate.py --workdir /tmp/migrate_iico --tables <table>

# Test mode (first N rows only)
python3 migrate.py --workdir /tmp/migrate_iico --tables <table> --limit 3

# Force re-run (ignore done.json checkpoint)
python3 migrate.py --workdir /tmp/migrate_iico --tables <table> --force
```

Loads are **idempotent** (`?upsert=1` on `unique_key=$.id`), so interrupted runs
resume safely. Large tables should run detached in tmux:

```bash
tmux new-session -d -s migrate \
  -c /home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine/tools/migrate_iico \
  "python3 migrate.py --workdir /tmp/migrate_iico --tables <tables> 2>&1 | tee /tmp/migrate_completion.log"
```

## Current state (2026-08-18)

The migration was **interrupted by the blocking-client-on-async-runtime hang**
(now fixed in `crates/mcp/src/transport.rs` + `crates/server/src/transport/rest.rs`
+ `crates/server/src/jobs.rs`). Many tables were left **partially loaded**
(`eng=3` = a `--limit 3` test, or a load cut off mid-batch). A completion run is
in progress.

### Verified complete (source == engine)

`profiles` 343, `learners` 294, `classes` 7, `guardians` 42, `fee_structures` 23,
`dormitories` 12, `dormitory_residents` 206, `visit_reasons` 370,
`inventory_items` 238, `inventory_stock` 237, `inventory_transactions` 5,
`employees` 45, `fee_assignments` 36, `fee_payments` 4, `finance_accounts` 21,
`site_settings` 11, `student_account_credentials` 294, `suppliers` 6,
`store_orders` 17, `library_books` 12, `library_members` 25, `meal_records` 294,
`schools` 1, `school_calendar` 5, `staff_attendance` 2, `asset_categories` 9,
`inventory_categories` 21, `archive_categories` 4, `grade_boundaries` 9,
`grading_scales` 1, `exam_series` 3, `promotion_rules` 7, `purchase_orders` 5,
`payroll_runs` 1, `salary_payments` 2, `salary_records` 2, `news` 3,
`payments` 1, `donors` 1, `projects` 4, `division_rules` 5, `discipline_cases` 1,
`academic_warnings` 1, `user_warnings` 1, `vehicle_logs` 2, `testimonials` 1,
`testimonial_likes` 1, `testimonial_comments` 1, `health_visits` 1,
`emergency_reentry_slips` 3, `budget_settings` 1, `liquidity_requests` 2,
`advance_requests` 2, `temp_workers` 4, `user_signatures` 3,
`compliance_contacts` 2, `compliance_notices` 1, `compliance_portals` 3,
`governance_meetings` 6, `communications` 1, `attendance_settings` 1,
`email_settings` 1, `appointments` 1, `leave_requests` 1, `print_orders` 3,
`print_order_permissions` 2, `notification_templates` 5, `notification_logs` 18,
`washing_machine_usage` 0, `tailor_tasks` 0, `tailor_task_materials` 0,
`school_infrastructure` 0, `school_sanitation` 0, `purchase_documents` 0,
`expense_requests` 0, `exchange_rates` 0, `journal_entries` 0,
`bursar_rules` 0, `bursar_override_requests` 0, `budget_requests` 0,
`donations` 0, `petty_cash_invoices` 0, `petty_cash_runs` 0,
`payment_notifications` 0, `approval_steps` 0.

### In progress / remaining (source != engine)

| Table | Source | Engine | Note |
|-------|--------|--------|------|
| assets | 1886 | partial | completing |
| attendance | 520 | partial | completing |
| term_results | 1957 | partial | completing |
| report_cards | 468 | partial | completing |
| class_timetables | 132 | partial | completing |
| student_account_credentials | 294 | partial | completing |
| syllabus_coverage | 35 | partial | completing |
| curriculum_topics | 38 | partial | completing |
| curriculum_plans | 8 | partial | completing |
| email_notification_events | 55 | partial | completing |
| email_queue | 27 | partial | completing |
| lesson_plans | 6 | partial | completing |
| lesson_register | 5 | partial | completing |
| scheme_of_work | 6 | partial | completing |
| subjects | 13 | 12 | 1 missing |
| visitor_visits | 6 | partial | completing |
| in_app_notifications | 12051 | — | needs load (slow) |
| user_roles | 344 | — | needs load |
| user_permissions | 72 | — | needs load |
| email_logs | 169 | — | needs load |
| teacher_assignments | 46 | — | needs load |
| learner_documents | 43 | — | needs load |
| staff_letters | 18 | — | needs load |
| teacher_documents | 1 | — | needs load |
| learner_sponsorships | 1 | — | needs load |
| temp_worker_logs | 2 | — | needs load |
| audit_log | 134811 | — | **SKIPPED** (audit, not needed by app) |

## Known issues

- **`next_seq` O(n²)** (`crates/engine/src/crud.rs`): each insert scans the whole
  table for the max seq. Fine for small tables; slow for large ones
  (in_app_notifications ~12k rows). Consider a per-table sequence counter.
- **`find_unique_holder` full-table scan** (`crates/engine/src/crud.rs`): each
  insert scans all rows for unique-key conflicts. Same O(n²) concern.
- **Daemon unresponsive during bulk load**: the engine `Mutex` is held during a
  bulk insert, so `/healthz` and other requests block until the batch finishes.
  Expected under load; not a hang.
