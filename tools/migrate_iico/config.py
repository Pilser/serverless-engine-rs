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
BOARD = _env("BOARD", "b_v2qyx0enpqkd0000")
OWNER_TOKEN = _env("OWNER_TOKEN", BOARD)  # board id bearer = owner role

# ---- migration behaviour ----------------------------------------------------
BATCH_SIZE = int(_env("MIGRATE_BATCH_SIZE", "100"))
SKIP_EXISTING_USERS = _env("SKIP_EXISTING_USERS", "1") == "1"

# ---- user-reference columns that must be rewritten UUID -> lowercase email --
# table -> list of column names holding an auth.users id.
USER_REF_COLUMNS = {
    "profiles": ["id", "suspended_by"],
    "account_appeals": ["user_id"],
    "advance_requests": ["user_id"],
    "akhlaaq_reports": ["teacher_id"],
    "appointments": ["created_by"],
    "asset_custodians": ["employee_id"],
    "assets": ["assigned_to_staff"],
    "attendance": ["recorded_by"],
    "audit_log": ["user_id"],
    "class_timetables": ["teacher_id"],
    "classes": ["teacher_id"],
    "clinic_visits": ["recorded_by"],
    "device_employee_mapping": ["user_id"],
    "digital_homework": ["teacher_id"],
    "direct_messages": ["to_user"],
    "dormitories": ["matron_staff_id"],
    "dormitory_attendance": ["recorded_by"],
    "email_schedules": ["created_by"],
    "employee_advances": ["employee_id"],
    "employees": ["profile_id"],
    "health_visits": ["recorded_by"],
    "in_app_notifications": ["created_by", "user_id"],
    "learners": ["user_id"],
    "leave_requests": ["user_id"],
    "lesson_observations": ["teacher_id"],
    "lesson_plans": ["teacher_id"],
    "lesson_register": ["teacher_id"],
    "library_books": ["created_by"],
    "library_members": ["profile_id"],
    "meal_records": ["recorded_by"],
    "medical_incidents": ["recorded_by"],
    "notifications": ["user_id"],
    "parent_learner_links": ["parent_user_id"],
    "payments": ["recorded_by"],
    "payroll_lines": ["employee_id"],
    "ple_mock_tests": ["created_by"],
    "print_order_permissions": ["user_id"],
    "print_orders": ["created_by"],
    "quran_progress": ["teacher_id"],
    "salah_attendance": ["recorded_by"],
    "salaries": ["user_id"],
    "scheduled_notifications": ["created_by"],
    "scheme_of_work": ["teacher_id"],
    "staff_attendance": ["user_id"],
    "staff_letters": ["to_user"],
    "syllabus_coverage": ["teacher_id"],
    "tailor_tasks": ["recorded_by"],
    "teacher_assignments": ["teacher_id"],
    "user_signatures": ["user_id"],
    "visitors": ["created_by"],
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
