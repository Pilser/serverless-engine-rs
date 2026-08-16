# serverlessEngine-rs

Standalone, embeddable serverless engine core (`crates/engine`), plus an
embeddable `crates/server`, an `crates/mcp` MCP tool server, and a `crates/cli`
(`srv`) that drives the MCP server over HTTP.

The `engine` crate is a pure library (zero external I/O deps) with two pluggable
storage traits — `Database` and `ObjectStore` — and in-memory adapters for tests.

Layout: `crates/{engine,server,mcp,cli}`. Design & contract: `docs/`
(`serverless-engine-plan.md`, `-features.md`, `-gap-analysis.md`,
`cli-mcp-interface-design.md`).
