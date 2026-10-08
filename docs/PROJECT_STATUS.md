# Project status

_Updated at the end of every working session._

## Current state: v0.1.0 released; v0.2.0 nearly done (DNS cross-check awaiting review; HTTP transport left)

**Last session: 2026-10-08 (session 4: DNS cross-check)**

### Done this session
- Merged PR #2 (admin and zone tools) after all 5 CI checks passed.
- Added the read-only DNS cross-check: `check_record_set_dns` and `check_zone_dns`.
  - Queries go directly to the authoritative nameservers (non-recursive, UDP with TCP
    fallback), and records on both sides are compared in canonical form.
  - Statuses: `in_sync`, `ttl_mismatch`, `mismatch`, `missing_in_dns`, `error`, `skipped` (SOA).
  - Notes cover nameservers that disagree, changes in progress, and non-authoritative answers.
- New settings: `VINYLDNS_MCP_DNS_NAMESERVERS` and `VINYLDNS_MCP_DNS_TIMEOUT_SECS`.
- Bug found and fixed while testing: nameserver discovery picked IPv6 addresses first, which
  fails on IPv4-only hosts. IPv4 is now preferred.
- Tests: 57 pass, plus 1 opt-in network test. Clippy reports no warnings; builds on Rust 1.89;
  the Linux release cross-builds still work.
  - The end-to-end tests run a fake authoritative DNS server (UDP and TCP) inside the test.
  - Live against the quickstart's BIND: the whole `ok.` zone checked as 14 in sync and SOA
    skipped. Real drift made with nsupdate was reported as `mismatch`, with the differing
    values and TTL.
  - Public-DNS discovery could only be partly checked here: the NS lookup worked, but this
    sandbox intercepts outbound DNS.
- Docs: README, TOOLS, CONFIGURATION, SECURITY, DEVELOPMENT, CHANGELOG, `--help`.

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
1. Review and merge the DNS cross-check PR (branch `feature/dns-cross-check`) once CI is green.
2. Owner: create tag `v0.1.0` on GitHub (task 16c).
3. Last v0.2.0 item: HTTP transport (22), which needs a per-user authentication design. Then
   release v0.2.0, the first real run of the release workflow. Alternatively, release v0.2.0
   now and move the HTTP transport to v0.3.0.
4. Try it in Claude Code or Claude Desktop (task 17).

### Decisions
- Private repository `Forenxics/vinyldns-mcp`.
- SemVer and Keep a Changelog.
- v0.2.0 scope: tasks 16, 19, 20, 22, 23.
- DNS cross-check queries authoritative servers directly (not the system resolver), so caches cannot hide drift.

### Known limitations
- stdio transport only (one user per process).
- Plans live in memory and are lost when the server restarts. This is intended:
  a stale plan should not survive a restart.
- No group management tools yet (task 21).
- Zone connections (TSIG keys) can be set when connecting a zone, but not changed by `plan_update_zone`.
- The DNS cross-check only goes from VinylDNS to DNS; records that exist only in DNS are found by `plan_sync_zone`.
