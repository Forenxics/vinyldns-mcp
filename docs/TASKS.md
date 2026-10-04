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
| 16 | v0.2.0: release workflow with prebuilt binaries for Linux/macOS/Windows on tag | Waiting (scheduled for v0.2.0) | |
| 17 | Test with Claude Code / Claude Desktop end to end (elicitation dialog UX) | Waiting | |
| 18 | Optional: Docker image | Waiting | |
| 19 | v0.2.0: admin tools: approve/reject batch changes (support/super users) | Waiting (scheduled for v0.2.0) | |
| 20 | v0.2.0: zone tools: connect/update/sync/delete zone, ACL rules | Waiting (scheduled for v0.2.0) | |
| 21 | Optional: group management tools (create group, add/remove members) | Waiting | |
| 22 | v0.2.0: streamable HTTP transport for shared/remote deployment (needs per-user auth design) | Waiting (scheduled for v0.2.0) | |
| 23 | v0.2.0: live DNS cross-check tool (compare VinylDNS data with what resolvers return) | Waiting (scheduled for v0.2.0) | |
| 24 | Optional: publish to crates.io and MCP registries | Waiting | |
| 25 | Optional: offer the server upstream to the VinylDNS project | Waiting | |
