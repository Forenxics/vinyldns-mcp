# Project status

_Updated at the end of every working session._

## Current state: v0.2.0 being released (first run of the release workflow)

**Last session: 2026-10-08 (session 4: DNS cross-check and the v0.2.0 release)**

### Done this session
- Merged PR #3 (DNS cross-check) after all 5 CI checks passed, including macOS and Windows
  builds of the new DNS libraries.
- DNS cross-check tools `check_record_set_dns` and `check_zone_dns`: direct authoritative
  queries (UDP with TCP fallback) and canonical comparison, verified live against BIND,
  including real drift made with nsupdate. Nameserver discovery now prefers IPv4.
- Prepared v0.2.0: version bump, dated changelog, and README status. The HTTP transport moved
  to v0.3.0, as decided by the owner.
- Release workflow: a manual run from `main` can now create the tag itself, after every build
  has passed. This is needed because this session cannot push tags, and it lets the owner
  release without local git.

### Earlier (session 3: admin and zone tools)
- Admin tier (`VINYLDNS_MCP_ENABLE_ADMIN`): batch approve/reject, zone connect/update/sync/delete,
  ACL rules; verified live, including manual-review approval by a support user.

### Earlier (session 2: release pipeline)
- Release workflow for 5 targets, with `SHA256SUMS.txt` and notes from the changelog; `--help`;
  workflow lint in CI; `docs/RELEASING.md`.

### Earlier (session 1)
- v0.1.0: 15 read tools, 8 write and pending-change tools with plan/confirm and
  elicitation; tested with unit, end-to-end and live tests; published to the
  private repository `Forenxics/vinyldns-mcp`.

### Next steps
1. Check the v0.2.0 release run: all 5 targets build, and the release page lists the archives
   and `SHA256SUMS.txt`. Then mark tasks 16 and 16e Completed.
2. Owner: create tag `v0.1.0` on commit `b88d355` (task 16c), so the changelog's 0.1.0 link works.
3. Try it in Claude Code or Claude Desktop with a real user (task 17).
4. v0.3.0: HTTP transport (task 22). This needs a per-user authentication design.

### Decisions
- Private repository `Forenxics/vinyldns-mcp`.
- SemVer and Keep a Changelog.
- v0.2.0 scope: tasks 16, 19, 20, 23. The HTTP transport (22) moved to v0.3.0 on 2026-10-08.
- DNS cross-check queries authoritative servers directly (not the system resolver), so caches cannot hide drift.

### Known limitations
- stdio transport only (one user per process).
- Plans live in memory and are lost when the server restarts. This is intended:
  a stale plan should not survive a restart.
- No group management tools yet (task 21).
- Zone connections (TSIG keys) can be set when connecting a zone, but not changed by `plan_update_zone`.
- The DNS cross-check only goes from VinylDNS to DNS; records that exist only in DNS are found by `plan_sync_zone`.
