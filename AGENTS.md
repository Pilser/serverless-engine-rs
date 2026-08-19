# serverlessEngine-rs

This is the new, standalone, embeddable **serverless engine** project. It is a separate git repo.

- **We work HERE:** `/home/vox/.AAAPROJECTS/Whatsapp4Agents/serverless-engine/`
- **Reference (behavioral contract to port):** `/home/vox/.AAAPROJECTS/Whatsapp4Agents/zerowrapper/`

## What this project is

A pure-library Rust crate (`crates/engine`) implementing the full serverless platform core
(document store, query/filter/search/aggregate, schema+computed+validate+redact, auth/keys/scopes,
recipes/automation, cron/jobs, webhooks, secrets, TTL, audit, links, files, realtime event model,
two pluggable storage traits `Database` + `ObjectStore`), plus an embeddable `crates/server`, an
`crates/mcp` tool server, and a `crates/cli` (`srv` CLI = MCP client over HTTP).

The existing reference engine in `zerowrapper/` is the reference implementation. **Its tests are the
behavioral contract** the new engine must satisfy.

## Read these FIRST (before any implementation)

All in `docs/` of this repo:
- `serverless-engine-plan.md` — actionable plan: folder layout, crate boundaries, phases, milestones
- `serverless-engine-features.md` — full feature inventory of the reference engine
- `serverless-engine-gap-analysis.md` — gap matrix + the `Database`/`ObjectStore` trait designs
- `cli-mcp-interface-design.md` — the CLI ↔ MCP control-surface spec (Phase 4)

## Architecture rules (critical)

- **Dependency direction:** `server → engine` and `mcp/cli → server`. `engine` depends on nothing
  workspace-local. `server` is the ONLY place DB/object-store/broker drivers live.
- **`engine` must compile and pass tests with ZERO external I/O deps** — only in-memory adapters.
  This keeps it embeddable (single process) or distributable (behind `server`).
- The query IR (`FilterCond`/`SrvFilter`/`Agg`) and both storage traits live in `engine/storage/`.
- One concern per file, max ~200 lines per file.

## RULES

### Build commands (MANDATORY for you and any sub-agents)
- Verify compilation with `cargo check` from the workspace root. **No flags** — never `cargo check -p <crate>`.
- To produce the binary: `cargo build --release` from the workspace root. **No flags** — never `-p`.
- The user runs `cargo test` themselves; do not run tests.
- **Never** run `cargo clean` or delete the `target/` directory. Never touch `target/`.
- Sub-agents MUST follow the same rules: plain `cargo check` / plain `cargo build --release`, no flags,
  no `cargo test`, no `cargo clean`, never touch `target/`.

### Workflow
- When you detect you are given a summary from a recent session, do NOT rush to implement.
  First verify with the user and get a go-ahead.
- Use sub-agents for complex multi-file changes; give each agent **disjoint files** to avoid conflicts.
- Do not implement from memory of the docs alone — read the corresponding `zerowrapper/` source
  and port behavior faithfully.
- **Parallel-implementation mode (current):** sub-agents write code ONLY. Do **NOT** run any
  `cargo` command (no `cargo check`/`build`/`test`). The lead runs ONE integration build after all
  waves land, then a fix pass. Write against the shared API contract in your task prompt and
  self-review for type correctness before finishing.

### Code style
- One concern per file, max ~200 lines per file.
- Explicit imports only (no `use super::*`).
- No comments unless the code is genuinely unclear.
- Follow the module structure in `docs/serverless-engine-plan.md` §3.

  
