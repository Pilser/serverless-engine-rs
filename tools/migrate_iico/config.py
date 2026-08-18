"""Configuration for the iico -> engine migration.

Edit values here (or override via environment variables / CLI flags).
"""

import os


def _env(key: str, default: str) -> str:
    return os.environ.get(key, default)


# ---- source: self-hosted supabase on the iico machine ----------------------
SRC_HOST = _env("IICO_HOST", "192.168.88.101")
SRC_USER = _env("IICO_SSH_USER", "iico")
SRC_SSH_PASS = _env("IICO_SSH_PASS", "12345678")
SRC_DB_CONTAINER = _env("IICO_DB_CONTAINER", "supabase-db")
SRC_DB_USER = _env("IICO_DB_USER", "supabase_admin")
SRC_DB_NAME = _env("IICO_DB_NAME", "postgres")

# ---- destination: the serverless engine ------------------------------------
ENGINE_BASE = _env("ENGINE_BASE", "http://127.0.0.1:7070")
# Fresh board (HelixDB tenant) for the graph-first migration.
BOARD = _env("BOARD", "b_8se1o3ansrkd0000")
OWNER_TOKEN = _env("OWNER_TOKEN", BOARD)  # board id bearer = owner role

# ---- migration behaviour ----------------------------------------------------
# Bulk batches are transaction-wrapped in the engine; Helix write transactions
# cap out around 20-30 conditional writes per request, so keep batches small.
BATCH_SIZE = int(_env("MIGRATE_BATCH_SIZE", "20"))
SKIP_EXISTING_USERS = _env("SKIP_EXISTING_USERS", "1") == "1"

# ---- graph edge map ---------------------------------------------------------
# table -> { ref_column: target_table }. Every `*_id` ref is KEPT as the source
# UUID (no email rewrite — the frontend joins by UUID and the graph model turns
# refs into RELATED_* edges). After a table loads, run `srv graph sync` to
# materialize the edges; this map documents the expected relationships for
# verification (every ref value must exist in the target table's id set).
GRAPH_EDGES = {
    "learners": {"class_id": "classes", "user_id": "profiles"},
    "attendance": {"class_id": "classes", "learner_id": "learners"},
    "teacher_assignments": {"teacher_id": "profiles", "class_id": "classes", "subject_id": "subjects"},
    "guardians": {"user_id": "profiles"},
    "parent_learner_links": {"parent_user_id": "profiles", "learner_id": "learners"},
    "dormitory_residents": {"dormitory_id": "dormitories", "learner_id": "learners"},
    "term_results": {"learner_id": "learners", "subject_id": "subjects"},
    "report_cards": {"learner_id": "learners"},
    "class_timetables": {"class_id": "classes", "subject_id": "subjects", "teacher_id": "profiles"},
    "scheme_of_work": {"teacher_id": "profiles", "class_id": "classes", "subject_id": "subjects"},
    "quran_progress": {"teacher_id": "profiles", "learner_id": "learners"},
    "syllabus_coverage": {"teacher_id": "profiles", "class_id": "classes", "subject_id": "subjects"},
    "fee_assignments": {"learner_id": "learners"},
    "fee_payments": {"learner_id": "learners"},
    "payments": {"learner_id": "learners", "recorded_by": "profiles"},
    "employees": {"profile_id": "profiles"},
    "staff_attendance": {"user_id": "profiles"},
    "salaries": {"user_id": "profiles"},
    "payroll_lines": {"employee_id": "employees"},
    "employee_advances": {"employee_id": "employees"},
    "assets": {"assigned_to_staff": "profiles", "category_id": "asset_categories"},
    "asset_custodians": {"employee_id": "employees", "asset_id": "assets"},
    "clinic_visits": {"learner_id": "learners", "staff_id": "profiles", "recorded_by": "profiles"},
    "health_visits": {"learner_id": "learners", "recorded_by": "profiles"},
    "medical_incidents": {"learner_id": "learners", "recorded_by": "profiles"},
    "discipline_cases": {"learner_id": "learners"},
    "akhlaaq_reports": {"learner_id": "learners", "teacher_id": "profiles"},
    "dormitory_attendance": {"dormitory_id": "dormitories", "learner_id": "learners"},
    "appointments": {"visitor_id": "visitors", "learner_id": "learners", "host_staff_id": "profiles"},
    "visitors": {"created_by": "profiles"},
    "in_app_notifications": {"created_by": "profiles", "user_id": "profiles"},
    "notifications": {"user_id": "profiles"},
    "direct_messages": {"to_user": "profiles"},
    "staff_letters": {"to_user": "profiles"},
    "leave_requests": {"user_id": "profiles"},
    "account_appeals": {"user_id": "profiles"},
    "advance_requests": {"user_id": "profiles"},
    "academic_warnings": {"learner_id": "learners"},
    "bursar_override_requests": {"learner_id": "learners"},
    "bursar_rules": {"class_id": "classes"},
    "budget_requests": {"requester_id": "profiles"},
    "donations": {"donor_id": "profiles"},
    "exam_timetable": {"invigilator_id": "profiles"},
    "email_schedules": {"created_by": "profiles"},
    "email_logs": {"template_id": "email_templates"},
    "device_employee_mapping": {"user_id": "profiles", "learner_id": "learners"},
    "digital_homework": {"class_id": "classes", "subject_id": "subjects", "teacher_id": "profiles"},
    "lesson_observations": {"teacher_id": "profiles"},
    "lesson_plans": {"teacher_id": "profiles", "class_id": "classes", "subject_id": "subjects"},
    "lesson_register": {"teacher_id": "profiles", "class_id": "classes", "subject_id": "subjects"},
    "library_members": {"profile_id": "profiles"},
    "library_books": {"created_by": "profiles"},
    "meal_records": {"recorded_by": "profiles"},
    "print_orders": {"created_by": "profiles"},
    "print_order_permissions": {"user_id": "profiles"},
    "scheduled_notifications": {"created_by": "profiles"},
    "tailor_tasks": {"recorded_by": "profiles"},
    "user_signatures": {"user_id": "profiles"},
    "quran_progress": {"teacher_id": "profiles", "learner_id": "learners"},
}

# Columns dropped before load (engine uses its own seq/id model).
# `id` is kept (the engine client treats payload.id as the app's logical id),
# but timestamps from the source are kept as-is in payload.
DROP_COLUMNS = {
    "profiles": ["updated_at"],
    "classes": ["updated_at"],
    "fee_structures": ["updated_at"],
}

# Tables that already exist in the engine with a stale strict schema (from an
# earlier manual import) that would reject real source rows. They are dropped
# and recreated permissively before loading.
RECREATE_TABLES = {"learners"}
