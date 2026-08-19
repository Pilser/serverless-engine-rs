# Owner/Admin Portal — Implementation Plan

## Overview

Add an "Owner" button on the public site that opens a **dedicated full-screen chat** (no website content) for admin/owners. They can upload files to a temp directory, and the agent receives the exact file path so it can copy/process the file.

---

## 1. Database Changes

### Files: `zw-db/src/migrations.rs`, `zw-db/src/users.rs`

Add `role` and `password_hash` columns to the `users` table:

```sql
ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'public';
ALTER TABLE users ADD COLUMN password_hash TEXT NOT NULL DEFAULT '';
```

Update `UserRow` struct:
```rust
pub struct UserRow {
    pub id: String,
    pub external_id: Option<String>,
    pub name: String,
    pub role: String,         // "public" | "admin" | "owner"
    pub created_at: String,
}
```

Add methods to `DbPool`:
- `verify_password(phone, password) -> Option<UserRow>` — verify bcrypt hash
- `set_user_password(phone, password) -> Result` — set bcrypt hash
- `set_user_role(phone, role) -> Result` — update role

---

## 2. Configuration (`config.toml` + `zw-core/src/config.rs`)

Add `accounts` section under `ch_web`:

```toml
[zerowrapper.ch_web.accounts]
"256757337153" = { role = "owner", password = "admin123" }
"256700000000" = { role = "admin", password = "admin456" }
```

Add `WebChannelAccounts` struct in config, load and seed into DB on startup.

---

## 3. Backend API Changes (`crates/ch-web/src/lib.rs`)

### New Endpoints

**`POST /api/admin/login`**
- Body: `{phone, password}`
- Look up user by phone, verify bcrypt password hash
- Return: `{ok: true, phone, name, role}` or `{error: "Invalid credentials"}`

**`POST /api/upload`**
- Accept multipart/form-data with a file field
- Save to `./data/uploads/{original_filename}_{YYYYMMDD_HHMMSS}.{ext}`
- Return: `{filename, path, size, mime_type}`

### Modified

- `GET /api/login` — now returns `role` field
- `POST /api/register` — only allows `role = "public"` (rejects if phone is in admin/owner list)
- WebSocket handler — accept `role` and `attachments` from frontend JSON
- `format_prompt` in `zeroclaw_bridge.rs` — include `role` and `attachments` in the prompt envelope

---

## 2. Frontend Changes

### New Components
- **`OwnerLogin.astro`** — modal overlay with phone + password fields
- **`OwnerChat.astro`** — full-screen chat-only view (no hero, FAQ, or website content)

### New JS Modules
- **`owner.js`** — owner login flow, modal management, mode switching
- **`owner-chat.js`** — dedicated chat logic for owner/admin (separate from public chat)
- **`file-upload.js`** — file upload handling (upload via `POST /api/upload`, display attachment, include in message)

### Modified Components
- **`Header.astro`** — add "Owner" button
- **`ChatForm.astro`** — add file upload button (shown only in owner mode)
- **`ChatApp.astro`** — conditionally render owner chat vs public chat

### Modified JS Modules
- **`connection.js`** — support `role` and `attachments` in send()
- **`main.js`** — handle owner mode switching
- **`dom.js`** — add new DOM element references

---

## 3. File Upload Flow

```
User clicks upload → file picker → POST /api/upload (multipart)
  → Backend saves to ./data/uploads/{original_filename}_{YYYYMMDD_HHMMSS}.{ext}
  → Returns {filename, path, size, mime_type}
  → Frontend shows attachment chip
  → User types message + sends
  → WebSocket sends: {session_id, user_id, role, content, attachments: [{filename, path, mime_type}]}
  → Backend passes to agent in prompt envelope:
    {"customer": "name", "message": "text", "channel": "web", "role": "owner", "attachments": [...]}
  → Agent sees exact file path, can copy/process it
```

---

## 5. Frontend UI Flow

1. Public user sees current site (hero, FAQ, chat)
2. "Owner" button in header
3. Click → modal overlay with phone + password fields
4. On success → page transitions to full-screen chat-only view
5. Chat has file upload button + text input
6. "Back to Public" button to return
7. Owner/admin sessions are isolated from public sessions

---

## 6. Implementation Order

1. **DB**: add `role` + `password_hash` columns, update `UserRow`
2. **Config**: add `accounts` section, seed on startup
3. **Backend**: `POST /api/admin/login` endpoint
4. **Backend**: `POST /api/upload` endpoint
5. **Backend**: modify `format_prompt` to include `role` + `attachments`
6. **Backend**: modify WebSocket handler to accept `role` + `attachments`
7. **Frontend**: add "Owner" button to Header
8. **Frontend**: create owner login modal
9. **Frontend**: create owner chat view
10. **Frontend**: add file upload button + handling
11. **Frontend**: modify connection.js for role/attachments

---

## 7. File Upload Flow (Detailed)

```
User clicks upload → file picker → POST /api/upload (multipart)
  → Backend saves to ./data/uploads/{original_filename}_{YYYYMMDD_HHMMSS}.{ext}
  → Returns {filename, path, size, mime_type}
  → Frontend shows attachment chip
  → User types message + sends
  → WebSocket sends: {session_id, user_id, role, content, attachments: [{filename, path, mime_type}]}
  → Backend passes to agent in prompt envelope:
    {"customer": "name", "message": "text", "channel": "web", "role": "owner", "attachments": [...]}
  → Agent sees exact file path, can copy/process it
```

---

## 8. Session Isolation

Owner/admin sessions are separate from public sessions:
- Session ID format for owner: `web.<agent>.<user_id>.owner`
- This ensures history is isolated between public and owner/admin chats

---

## 9. Key Design Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Auth method | Phone + password | Reuses existing phone-based identity, adds password for security |
| File storage | `./data/uploads/` | Simple, local, agent has filesystem access |
| Role in prompt | `{"role": "owner"}` | Agent can adapt behavior based on role |
| Owner chat UI | Full-screen, no website | Clean separation from public site |
| Session isolation | Different session ID suffix | Prevents history mixing |
| File info delivery | In prompt envelope | Agent gets exact path immediately |</think>

<｜DSML｜tool_calls>
<｜DSML｜invoke name="write">
<｜DSML｜parameter name="filePath" string="true">/home/pilser/AA-Projects/Whatsapp4Agents/zerowrapper/OWNER_PORTAL_PLAN.md