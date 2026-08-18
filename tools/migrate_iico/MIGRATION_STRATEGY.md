# IICO → HelixDB Migration Strategy (graph-first)

Supersedes the turso-era `MIGRATION_STATUS.md`. The engine storage backend is now
**HelixDB** (one tenant per board, records = nodes, `*_id` refs = edges). The old
64-table migration was loaded into the turso backend, which is gone; **everything
is re-migrated fresh** into a new Helix board (tenant).

## 1. The frontend check that decided the data model

`deploy/iico` (the real IICO app, Vite/React + Supabase) was inspected to decide
whether to keep the old **UUID→email rewrite** or keep source UUIDs:

- `FeeAssignmentsPanel.tsx`: `classMap = Object.fromEntries(classes.map(c => [c.id, c.name]))` then `classMap[learner.class_id]`
- `AssetDialog.tsx`: `profiles.select("id, full_name")`, `SelectItem value={s.id}`

**Every `*_id` reference is joined in application code by matching the UUID to
the target table's `id` column.** The old email-rewrite was a workaround for the
old engine's user model, and it breaks this pattern.

**Decision: keep source UUIDs as-is.** Do NOT rewrite `*_id` refs to emails.
This preserves the frontend's join-by-UUID behavior and enables the graph model.

## 2. Node / edge model (what becomes what)

| Source table | In the graph | Notes |
|---|---|---|
| `profiles` (users) | Nodes (own table, keep `id` UUID) | `auth` signup handled separately (email+password) |
| `learners`, `classes`, `subjects`, `guardians`, `employees`, `dormitories`, `visitors`, `rooms` | Nodes | entity tables |
| `learners.class_id` | Edge `RELATED_CLASS` | auto via `graph sync learners` |
| `learners.user_id` | Edge `RELATED_USER` | auto via `graph sync learners` |
| `guardians` ↔ `learners` (`parent_learner_links`) | Edge `GUARDIAN_OF` | auto via `graph sync` |
| `teacher_assignments.(teacher,class,subject)` | Edges `TEACHES` / `USES` | auto via `graph sync teacher_assignments` |
| `term_results`, `attendance`, `report_cards`, `fee_payments`, `dormitory_residents`, ... | Nodes with `*_id` refs | facts attached by edges |

Rule: **entities are nodes; `*_id` columns are edges**. `graph sync <table>`
materializes `RELATED_<FIELD>` edges automatically, so no custom edge code is
needed in the loader.

## 3. Migration pipeline (revised)

Reuse `tools/migrate_iico/` with these changes:

1. **Extract** (`extract.py`) — unchanged (SSH + psql JSON dump).
2. **Transform** (`transform.py`) — **remove the UUID→email rewrite**. Keep
   `DROP_COLUMNS` (source timestamps etc.). Null stripping stays.
3. **Load** (`loader.py`) — unchanged mechanics (create permissive table,
   `unique_key: "$.id"`, batch upsert `?upsert=1`), **but target a fresh board
   (tenant)** and keep `id` as the source UUID.
4. **Graph sync** — after a relationship table loads, run
   `srv graph sync <board> <table>` to materialize edges, then verify a sample
   of edges resolve (traverse both directions).
5. **Verify** — per table, beyond count-matching:
   - sample 3-5 real rows and diff fields against source
   - confirm every `*_id` value exists in the target table's `id` set (referential check)
   - confirm `graph sync` created the expected edges and traversal works

## 4. Execution order (manual first, then parallel)

Phase 1 — **manual, to lock the recipe** (lead agent):
1. `profiles` (343) + auth users (separate signup step)
2. `learners` (294) + `classes` (7) + `subjects` (26) — the core network
3. `guardians` (42) + `parent_learner_links` — guardian edges
4. Verify + `graph sync` + update this doc

Phase 2 — **parallel sub-agents**, disjoint table groups, shared checklist in
`SUBAGENT_WORKER.md`. Every table in the source DB is assigned below (full
~180-table inventory from `information_schema`), so nothing is left behind.
Order within each group: entities → relationship/fact tables (refs resolve).

- **G1 academics:** term_results, teacher_assignments, attendance, report_cards,
  scheme_of_work, class_timetables, lesson_observations, lesson_plans,
  lesson_register, quran_progress, syllabus_coverage, digital_homework,
  curriculum_plans, curriculum_topics, exam_series, exam_timetable,
  grade_boundaries, grading_scales, ple_mock_tests, ple_results, uneb_results,
  promotion_evaluations, promotion_results, promotion_rules,
  student_progress_snapshots, academic_warnings, user_warnings,
  school_calendar
- **G2 HR/payroll:** employees, staff_attendance, salaries, salary_payments,
  salary_records, payroll_lines, payroll_runs, employee_advances,
  leave_requests, device_employee_mapping, staff_performance_logs,
  temp_workers, temp_worker_logs, user_roles, user_permissions, user_signatures
- **G3 finance:** fee_structures, fee_assignments, fee_payments, payments,
  payment_notifications, budget_requests, budget_settings, donations, donors,
  bursar_override_requests, bursar_rules, finance_accounts, journal_entries,
  exchange_rates, liquidity_requests, expense_requests, petty_cash_invoices,
  petty_cash_runs, approval_steps, purchase_orders, purchase_documents,
  goods_received, suppliers, store_orders
- **G4 library/inventory/assets:** library_books, library_fines, library_loans,
  library_members, library_reservations, inventory_categories,
  inventory_items, inventory_stock, inventory_transactions,
  inventory_gate_passes, inventory_gate_handovers (gate_handovers), assets,
  asset_categories, asset_custodians, archive_categories, document_registry,
  hostel_issuances
- **G5 discipline/health/visitors/orphanage:** discipline_cases,
  akhlaaq_reports, clinic_visits, health_visits, medical_incidents,
  medication_logs, medication_register, prescriptions, pharmacy_emergencies,
  pharmacy_inventory_batches, pharmacy_items, pharmacy_notifications,
  pharmacy_reorder_requests, visitors, visitor_categories, visitor_visits,
  visit_reasons, appointments, emergency_reentry_slips, exit_passes,
  gate_handovers, student_gate_logs, vehicle_logs, dormitories,
  dormitory_attendance, dormitory_residents, guardian_of_links,
  learner_sponsorships, learner_essentials, learner_medical, learner_documents
- **G6 comms/notifications/governance:** in_app_notifications, notifications,
  notification_logs, notification_templates, scheduled_notifications,
  email_logs, email_notification_events, email_queue, email_schedules,
  email_settings, direct_messages, staff_letters, communications,
  governance_meetings, governance_members, governance_policies, meeting_attendance,
  meeting_minutes, ministry_guidelines, national_standards, compliance_contacts,
  compliance_notices, compliance_portals, news, site_settings, whatsapp_settings,
  audit_log, division_rules, reviews_ratings, testimonial_comments,
  testimonial_likes, testimonials
- **G7 misc/schools:** schools, school_infrastructure, school_sanitation,
  classroom_blocks, accounts, iple_candidates, iple_center_registration,
  iple_oral_examinations, iple_subject_scores, tailor_tasks,
  tailor_task_materials, washing_machine_usage, print_orders,
  print_order_permissions, student_account_credentials, meal_records,
  attendance_settings, incident_reports, donor_donations, staff_letters,
  salary_slips

> Group boundaries are approximate (names like `guardian_of_links` vs
> `parent_learner_links` differ per source). Each agent should list the actual
> tables in its group from `information_schema` before starting, and report any
> table not covered so it gets assigned.

Each agent gets: the board id, the table list, the loader command, the
referential-check snippet, and the `graph sync` + traverse verification, and
must update a per-table row in this doc's status table.

## 5. Status

> **Board:** `b_7z5lr8zaurkd0000` (HelixDB tenant). Migration is **in progress** —
> see `MIGRATION_STATUS.md` for the live per-table checklist. The engine's
> `next_seq`/`find_unique_holder` do full-table scans per row (O(n²)); large
> tables load slowly, so the run is detached in tmux and idempotent (`?upsert=1`
> on `unique_key=$.id`), so interrupted runs resume safely.

## 6. Board / tenant

- Board (tenant) id: **`b_7z5lr8zaurkd0000`** (created at migration start)
- `config.py`: `BOARD` set to the board id; `ENGINE_BASE` = daemon URL
- Owner token = board id (owner role)

## 7. Known engine considerations

- `unique_key: "$.id"` on every table (source UUIDs are unique)
- `next_seq` per table is fine at these sizes (<2.5k rows/table)
- The graph FTS text index builds lazily on first `records search`; `_search`
  blobs are written on insert, so search works (fallback scan) immediately
- Source `_users.json` (auth.users) import: use `--users` signup step with
  password hashes (bcrypt `$2a$`), unchanged
