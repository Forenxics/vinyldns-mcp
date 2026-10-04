# Project status

_Updated at the end of every working session._

## Current state: v0.1.0, feature-complete for the first release; not yet published

**Last session: 2026-10-04**

### Done this session
- Confirmed that no VinylDNS MCP server existed.
- Built `vinyldns-mcp` in Rust (`rmcp` 3.5): 15 read tools, plus 8 write and
  pending-change tools behind a plan → confirm workflow with elicitation-based
  human confirmation.
- Verified request signing three ways (AWS test vector, an independent port of
  the VinylDNS server algorithm, and a live server).
- Tests: 18 unit and 12 end-to-end tests pass; clippy reports no warnings with
  `-D warnings`. The live smoke test passed against VinylDNS 0.20.2 (quickstart):
  create, update and delete all reached `Complete`, and a batch change completed.
- Wrote the README, configuration, tools, security and development docs, the
  changelog, and this tracking.

### Next steps
1. Owner creates an empty private repository `Forenxics/vinyldns-mcp`; then push `main` and tag `v0.1.0` (task 15).
   Until then, the code is backed up on branch `claude/laughing-franklin-1mgqpl` of `forenxics/vinyldns`.
2. Try it in Claude Code or Claude Desktop with a real user (task 17).
3. v0.2.0 (agreed): release binaries (16), admin and zone tools (19, 20), HTTP transport (22),
   live DNS cross-check (23).

### Decisions
- Private repository `Forenxics/vinyldns-mcp`.
- SemVer and Keep a Changelog.
- v0.2.0 scope: tasks 16, 19, 20, 22, 23.

### Known limitations
- stdio transport only (one user per process).
- Plans live in memory and are lost when the server restarts. This is intended:
  a stale plan should not survive a restart.
- No zone, group, or batch approval administration yet.
