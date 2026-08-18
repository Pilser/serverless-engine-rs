# Blocking Client on Async Runtime — Root Cause, Fix, and Audit

## TL;DR

The daemon's HTTP handlers were calling a **blocking reqwest client directly on a
tokio async worker thread**. reqwest's `blocking` client spawns its own internal
single-threaded tokio runtime to do the I/O. When that blocking client is invoked
*from inside another* (multi-threaded) tokio runtime, the two runtimes interact
badly and, after roughly **128 requests**, the blocking client's internal runtime
stops dispatching the request to the network. Every subsequent request then sits
stalled for the full configured timeout (30s) before failing with
`error sending request`.

The bug was **not** in any dependency version. It was a usage error:
running a `reqwest::blocking::Client` on an async executor thread.

---

## 1. The Symptom (observed)

Command being run repeatedly:

```bash
srv graph sync <board> learners
```

Observed behaviour:

1. The daemon processes ~127–135 Helix requests normally (each 1–40ms).
2. Then one request stalls for exactly **30000ms** and fails:
   ```
   [helix] http://127.0.0.1:7979/v2/query FAILED after 30000ms: error sending request for url (http://127.0.0.1:7979/v2/query)
   ```
3. All subsequent requests fail the same way (30s each). The CLI appears to hang.
4. During the stall, Helix itself is perfectly responsive — `curl` to the *exact*
   failing query returns `{"rows":[]}` in milliseconds.
5. The daemon's `/healthz` goes unresponsive because the engine `Mutex` is held
   by the stuck worker thread.

### Debug evidence

- `strace -f` on the daemon showed the `reqwest-internal-sync-runtime` thread
  parked in `epoll_wait` for 26.5s **without ever issuing a `connect()`** — the
  request was submitted to the blocking client but never dispatched to the socket.
- The caller thread (a `tokio-rt-worker`) was parked in a futex inside reqwest's
  `wait::timeout`, waiting the full 30s for a response that never came.

---

## 2. The Root Cause (proved with a minimal repro)

A standalone Rust repro (plain `reqwest` + `serde_json`) posted the same JSON to
Helix 300 times in a row:

| Execution context                         | Result                    |
| ----------------------------------------- | ------------------------- |
| Blocking client on a **plain `std::thread`** | 300 / 300 OK, 0ms each |
| Blocking client **inside `tokio::Runtime`**  | **stalls at request #127**, then 30s failures |
| Blocking client inside `tokio::task::spawn_blocking` | 300 / 300 OK, 0ms each |

Conclusion: **calling a `reqwest::blocking::Client` from inside a tokio async
context is the trigger.** The blocking client's internal runtime and the outer
tokio runtime deadlock/stall after a burst of requests.

### Why it happens

- `reqwest::blocking::Client` creates a dedicated background thread
  (`reqwest-internal-sync-runtime`) running its own `current_thread` tokio runtime.
- Calls are handed to that thread via an mpsc channel; the caller then blocks with
  `thread::park_timeout` waiting for a oneshot reply.
- When the caller is itself a tokio worker, the two runtimes' parking/notify
  mechanisms collide. After enough requests the wake-up is lost, the internal
  runtime never polls the request, and the caller times out.

---

## 3. The Fix

Wrap every blocking call so it runs on tokio's **blocking thread pool** via
`tokio::task::spawn_blocking`, instead of on an async worker thread.

```rust
let out = match tokio::task::spawn_blocking(move || {
    crate::handle_jsonrpc(&mcp, &text)   // blocking: engine lock + reqwest::blocking
}).await {
    Ok(out) => out,
    Err(_) => { /* json-rpc internal error */ }
};
```

### Where it was patched

| File | Change |
| ---- | ------ |
| `crates/mcp/src/transport.rs` | `handle_hyper` now runs `handle_jsonrpc` (which locks the engine and uses the blocking Helix client) inside `tokio::task::spawn_blocking`. |

This fixed the reported `srv graph sync` hang: 0 failures, all requests 1–247ms.

---

## 4. Codebase Audit — other occurrences of the same problem

Because this is a serverless engine (external apps can trigger writes, hooks,
recipes, cron jobs), the blocking call can be reached from many async entry
points. Every path below invokes the **blocking** Helix client
(`crates/helixdb/src/client.rs`) or the **blocking** outbound HTTP caller
(`crates/server/src/http_caller.rs`, installed as `engine::http::HttpCaller`) from
inside a tokio async context.

### 4.1 The blocking clients

Two `reqwest::blocking::Client`s exist:

1. `crates/helixdb/src/client.rs` — the database backend (Helix).
   - `Client::post` / `Client::execute` are **synchronous**, no `.await`.
2. `crates/server/src/http_caller.rs` — `ReqwestCaller`, installed via
   `engine::install_http_caller`, used for outbound webhooks/recipes/cron HTTP.
   - `HttpCaller::call` is **synchronous**, no `.await`.

### 4.2 Chain that must NOT run on an async worker

```
async HTTP handler (tokio worker)
   └─ engine.lock()                       // std::sync::Mutex
       └─ graph_sync / insert_record / update_record / delete_record / dispatch_recipes / dispatch_cron / job run
           └─ helixdb::Client::post()     // reqwest::blocking  (BLOCKING)
           └─ engine::http::http_call()   // ReqwestCaller       (BLOCKING)
```

### 4.3 Confirmed problem locations (needs the same fix)

| # | File | Line(s) | Why it's a problem |
|---|------|---------|--------------------|
| 1 | `crates/mcp/src/transport.rs` | `handle_hyper` → `handle_jsonrpc` | **FIXED.** Was running blocking engine on async worker; now wrapped in `spawn_blocking`. |
| 2 | `crates/server/src/transport/rest.rs` | `handle` (async, ~172) → `route` (async, ~364) → many `engine.lock()` calls; webhook `dispatch_recipes` at ~721 | REST handlers run on tokio workers and call the blocking engine (Helix + recipes) directly. |
| 3 | `crates/server/src/jobs.rs` | `spawn_scheduler` (async, ~52) → `run_job` (async, ~74) → `engine::http::http_call` at ~116 | Cron job HTTP actions use the blocking caller on an async worker. |
| 4 | `crates/engine/src/crud.rs` | `insert_record`/`update_record`/`delete_record` → `automation::dispatch` | These are triggered from async REST/MCP handlers; recipes run blocking outbound HTTP. |
| 5 | `crates/engine/src/automation.rs` | `dispatch` (~148) / `dispatch_cron` (~185) → `run_actions` → `apply_action` → `http_call_body` (~563); `srv_http_call` (~278) | Recipe actions perform blocking outbound HTTP. Safe only if the caller thread is a blocking thread. |
| 6 | `crates/server/src/transport/rest_admin.rs` | `server.engine.lock().unwrap()` in every admin handler | Admin endpoints call the blocking engine on async workers (lower risk — fewer requests — but same hazard). |

### 4.4 Already correct (async client used properly)

- `crates/server/src/jobs.rs` — `spawn_webhook_worker` uses `reqwest::Client`
  (async) with `.send().await`. **Correct.**
- `crates/server/src/transport/sse.rs`, `ws.rs` — only read records
  (`records_after`), no blocking outbound HTTP. **Low risk.**

---

## 5. Recommended Fixes (apply to the remaining locations)

### 5.1 Server REST transport (`rest.rs`)

Move the body of `route` / `handle` into a `spawn_blocking` closure, mirroring the
MCP fix. The `route` function takes `Arc<Server>`; clone it into the closure and
call the blocking handler there. Because `route` currently awaits `req.collect()`
for the body first, collect the body in the async fn, then dispatch the
*already-collected* request to `spawn_blocking`.

```rust
let server = server.clone();
let body_bytes = /* collected above */;
let resp = tokio::task::spawn_blocking(move || {
    route_blocking(&server, &method, &segs, &headers, &body_bytes, &params)
}).await.unwrap_or_else(/* 500 fallback */);
```

### 5.2 Scheduler cron HTTP (`jobs.rs` `run_job`)

Wrap the HTTP action portion (the `engine::http::http_call(...)` call and the
subsequent `insert_record`) in `spawn_blocking`:

```rust
let engine = engine.clone();
let job = job.clone();
let message = tokio::task::spawn_blocking(move || {
    // ... existing job logic (http_call, insert_record) ...
}).await.unwrap_or_else(|_| "job failed".to_string());
```

### 5.3 Recipe/automation HTTP (`crud.rs`, `automation.rs`)

These are reached from async REST/MCP handlers. Once the REST transport (5.1)
is fixed to run the whole engine dispatch in `spawn_blocking`, recipe actions run
on blocking threads and are safe. The MCP path is already fixed. No change needed
inside `automation.rs` itself — the fix belongs at the transport boundary.

---

## 6. Rule of thumb (prevent regression)

- Never call a `reqwest::blocking::Client` (or any blocking I/O) from a tokio
  async context.
- Every async HTTP handler that touches the engine must dispatch the blocking work
  with `tokio::task::spawn_blocking`.
- Prefer the **async** `reqwest::Client` for any new outbound HTTP that originates
  in an async context (e.g. the webhook worker already does this correctly).
- Keep `engine` free of blocking transport: it must compile and pass tests with
  zero external I/O deps (in-memory adapters only). The blocking Helix/HTTP
  clients are server-side adapters and must only ever run on blocking threads.

---

## 7. Verification

After the fix, re-run:

```bash
srv graph sync <board> learners
```

Expected: completes without 30s failures. Check the daemon log:

```bash
grep -c FAILED /tmp/daemon_fixed.log   # expect 0
grep 'took' /tmp/daemon_fixed.log | tail   # expect millisecond values, no 30s
```

A standalone repro of the underlying issue (blocking client on tokio) is
available under `/tmp/repro/` (not part of this repo).
