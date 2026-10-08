# Project timeline and task list

Stages: **Waiting** → **Started** → **In progress** → **Testing** → **Completed**.
When a task finishes, its stage is set to **Completed** and the date is recorded.

## Timeline

| Date | Milestone |
|---|---|
| 2026-10-04 | Feasibility check: no existing VinylDNS MCP server found (web, package registries, MCP directories) |
| 2026-10-04 | Decisions: Rust, read + write with confirmation, separate repository, SemVer + changelog |
| 2026-10-04 | v0.1.0 implemented, tested (unit, end-to-end, live against VinylDNS 0.20.2) and documented |
| 2026-10-04 | v0.2.0 scope agreed: release binaries, admin tools, live DNS cross-check, HTTP transport |
| 2026-10-04 | v0.2.0 started: release workflow (task 16) |
| 2026-10-08 | PR #1 (release workflow, CI fixes) merged; admin and zone tools implemented (tasks 19, 20) |

## Tasks

| # | Task | Stage | Completed |
|---|---|---|---|
| 1 | Research: existing VinylDNS MCP servers or clients | Completed | 2026-10-04 |
| 2 | Choose language and SDK (Rust + `rmcp` 3.5) | Completed | 2026-10-04 |
| 3 | SigV4 signer compatible with VinylDNS, plus Python reference port | Completed | 2026-10-04 |
| 4 | Configuration from environment, with validation and secret redaction | Completed | 2026-10-04 |
| 5 | Signed HTTP client with error hints | Completed | 2026-10-04 |
| 6 | Read tools (15) | Completed | 2026-10-04 |
| 7 | Plan/confirm write workflow and pending-change store | Completed | 2026-10-04 |
| 8 | Elicitation-based human confirmation (`auto`/`elicit`/`token`) | Completed | 2026-10-04 |
| 9 | Write tools: record set create/update/delete, batch submit/cancel | Completed | 2026-10-04 |
| 10 | Unit tests (18) and end-to-end MCP tests (12) | Completed | 2026-10-04 |
| 11 | Live smoke test against the VinylDNS quickstart | Completed | 2026-10-04 |
| 12 | Documentation (README, configuration, tools, security, development) | Completed | 2026-10-04 |
| 13 | CHANGELOG + SemVer process | Completed | 2026-10-04 |
| 14 | CI workflow (fmt, clippy, test, build) | Completed | 2026-10-04 |
| 15 | Create GitHub repository (private `Forenxics/vinyldns-mcp`) and push v0.1.0 | Completed | 2026-10-04 |
| 16 | v0.2.0: release workflow with prebuilt binaries for Linux/macOS/Windows on tag | Testing (Linux builds, packaging and notes checked locally; first real run happens at the v0.2.0 tag) | |
| 16a | `--help` flag and stricter argument handling | Completed | 2026-10-04 |
| 16b | CI: workflow lint (actionlint) and changelog script tests | Completed | 2026-10-04 |
| 16d | Fix CI: MSRV 1.88 → 1.89 (uuid), actions moved to Node 24 versions | Completed | 2026-10-04 |
| 16c | Tag `v0.1.0` on GitHub (the session cannot push tags; owner creates it) | Waiting | |
| 17 | Test with Claude Code / Claude Desktop end to end (elicitation dialog UX) | Waiting | |
| 18 | Optional: Docker image | Waiting | |
| 19 | v0.2.0: admin tools: approve/reject batch changes (support/super users) | Testing (unit, end-to-end and live tests pass; awaiting PR review and CI) | |
| 20 | v0.2.0: zone tools: connect/update/sync/delete zone, ACL rules | Testing (unit, end-to-end and live tests pass; awaiting PR review and CI) | |
| 20a | `VINYLDNS_MCP_ENABLE_ADMIN` setting; `list_deleted_zones`, `list_backend_ids` read tools | Testing | |
| 20b | Live admin smoke test (`scripts/smoke_test_admin.py`) | Completed | 2026-10-08 |
| 21 | Optional: group management tools (create group, add/remove members) | Waiting | |
| 22 | v0.2.0: streamable HTTP transport for shared/remote deployment (needs per-user auth design) | Waiting (scheduled for v0.2.0) | |
| 23 | v0.2.0: live DNS cross-check tool (compare VinylDNS data with what resolvers return) | Waiting (scheduled for v0.2.0) | |
| 24 | Optional: publish to crates.io and MCP registries | Waiting | |
| 25 | Optional: offer the server upstream to the VinylDNS project | Waiting | |
