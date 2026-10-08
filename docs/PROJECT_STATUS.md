# Project status

_Updated at the end of every working session._

## Current state: v0.1.0 released; v0.2.0 in progress (release workflow merged; admin and zone tools awaiting review)

**Last session: 2026-10-08 (session 3: admin and zone tools)**

### Done this session
- Merged PR #1 (release workflow and CI fixes) after all 5 CI checks passed.
- Added the admin tier (`VINYLDNS_MCP_ENABLE_ADMIN`, which also needs writes) with 8 plan tools:
  - batch review: approve and reject;
  - zone management: connect, update, sync, delete;
  - zone ACL rules: add and delete.
  Also added 2 read tools: `list_deleted_zones` and `list_backend_ids`.
- Safety details:
  - zone updates carry over every field VinylDNS would otherwise clear;
  - zone delete requires the zone name to be typed again;
  - TSIG keys are redacted from previews;
  - ACL rule delete sends the exact stored rule.
- Tests: 24 unit and 22 end-to-end tests pass; clippy reports no warnings; the build works on Rust 1.89.
  - Live (VinylDNS 0.20.2 quickstart): connect, ACL add and remove, update (ACL and connection kept;
    a record write afterwards completed), delete, and a batch that went to manual review was
    approved by the support user. A regular user's approval got 403, as expected.
  - Sync was correctly refused because the zone had just synced; this led to a clearer 403 hint.
- Docs: README tools table, TOOLS, CONFIGURATION, SECURITY, DEVELOPMENT, CHANGELOG, `--help`.

### Earlier (session 2: release pipeline)
- Release workflow for 5 targets, with `SHA256SUMS.txt` and notes from the changelog; `--help`;
  workflow lint in CI; `docs/RELEASING.md`.

### Earlier (session 1)
- v0.1.0: 15 read tools, 8 write and pending-change tools with plan/confirm and
  elicitation; tested with unit, end-to-end and live tests; published to the
  private repository `Forenxics/vinyldns-mcp`.

### Next steps
1. Review and merge the admin and zone tools PR (branch `feature/admin-zone-tools`) once CI is green.
2. Owner: create tag `v0.1.0` on GitHub (task 16c).
3. Finish v0.2.0: live DNS cross-check (23) and HTTP transport (22). Then release v0.2.0,
   which is the first real run of the release workflow.
4. Try it in Claude Code or Claude Desktop (task 17).

### Decisions
- Private repository `Forenxics/vinyldns-mcp`.
- SemVer and Keep a Changelog.
- v0.2.0 scope: tasks 16, 19, 20, 22, 23.

### Known limitations
- stdio transport only (one user per process).
- Plans live in memory and are lost when the server restarts. This is intended:
  a stale plan should not survive a restart.
- No group management tools yet (task 21).
- Zone connections (TSIG keys) can be set when connecting a zone, but not changed by `plan_update_zone`.
