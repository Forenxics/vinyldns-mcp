# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Release workflow (`.github/workflows/release.yml`): on a `vX.Y.Z` tag (or a manual
  run), checks that the tag matches `Cargo.toml` and `CHANGELOG.md`, runs the tests,
  builds Linux x86_64 (static musl), Linux ARM64, macOS ARM64 and x86_64, and Windows
  x86_64 binaries, then publishes them with `SHA256SUMS.txt`, release notes from
  the changelog, and build provenance attestations (public repositories).
- `scripts/changelog_section.py` (with tests) to extract a version's release notes.
- `--help` / `-h` flag summarising the configuration variables; unknown arguments
  now exit with status 2.
- CI: runs the changelog script tests and lints the workflows with actionlint.
- Documentation: `docs/RELEASING.md`, and prebuilt-binary install steps in the README.
- Admin tools behind the new `VINYLDNS_MCP_ENABLE_ADMIN` setting (which also needs writes):
  `plan_approve_batch_change` and `plan_reject_batch_change` (support/super users),
  `plan_connect_zone`, `plan_update_zone`, `plan_sync_zone`, `plan_delete_zone`,
  `plan_add_zone_acl_rule` and `plan_delete_zone_acl_rule`. All use plan → `confirm_change`.
  - Zone updates carry over every field you do not change (connections, ACL rules, backend,
    schedule), because VinylDNS clears omitted fields.
  - Zone deletion requires the zone name to be typed again, and its preview says the
    records stay on the DNS server.
  - TSIG keys are redacted from all previews.
  - ACL rule removal sends the exact stored rule.
- Read tools `list_deleted_zones` and `list_backend_ids`.
- `check_connection` reports whether admin tools are enabled.
- `scripts/smoke_test_admin.py`: live test of the admin tools against the VinylDNS quickstart.
- DNS cross-check tools `check_record_set_dns` and `check_zone_dns` (read-only). They query the
  zone's authoritative nameservers directly and report, per record set and nameserver:
  `in_sync`, `ttl_mismatch`, `mismatch` (with the differing values), `missing_in_dns` or `error`.
  They also note nameservers that disagree, changes still in progress, and non-authoritative
  answers. Nameservers come from a parameter, from `VINYLDNS_MCP_DNS_NAMESERVERS`, or from
  the zone's NS records (IPv4 preferred). UDP with TCP fallback; `VINYLDNS_MCP_DNS_TIMEOUT_SECS`.
- `scripts/smoke_test_dns.py`: live DNS cross-check test, including real drift made with nsupdate.

### Changed
- Minimum supported Rust version is now 1.89 (required by `uuid` 1.27). 0.1.0 stated
  1.88 by mistake; the CI MSRV job caught it.
- Clearer hint for HTTP 403: VinylDNS also uses it for actions that aren't allowed right
  now (e.g. syncing a zone shortly after the last sync), not only for missing permissions.
- End-to-end test helpers moved to `tests/common/`.
- GitHub Actions updated to Node 24 versions (`checkout@v7`, `upload-artifact@v7`,
  `download-artifact@v8`, `attest-build-provenance@v4`).

## [0.1.0] - 2026-10-04

### Added
- MCP server over stdio, built on the official `rmcp` 3.5 SDK.
- AWS SigV4 request signing compatible with VinylDNS's `Aws4Authenticator`, verified
  against the AWS test vector, an independent Python port of the server
  algorithm, and a live VinylDNS 0.20.2 instance.
- 15 read tools: connection check, zones, zone changes, record sets (per zone and
  global search), record set changes, batch changes, groups, group members, users.
- Write workflow (opt-in with `VINYLDNS_MCP_ENABLE_WRITES`): `plan_create_record_set`,
  `plan_update_record_set`, `plan_delete_record_set`, `plan_batch_change`,
  `plan_cancel_batch_change`, `confirm_change`, `list_pending_changes`,
  `discard_pending_change`.
- Human confirmation through MCP elicitation, with `auto` / `elicit` / `token` modes.
- Single-use plan tokens with a configurable TTL and a limit on outstanding plans.
- Input validation (record types, TTL, the fields each record type needs, single-record
  CNAMEs) and warnings for conflicting record sets and shared zones without an owner group.
- Updates keep fields you don't change, including `ownerGroupId`.
- MCP tool annotations (read-only, destructive, open-world hints).
- Unit tests, end-to-end MCP tests with a mocked VinylDNS, and a live smoke-test script.
- Documentation: README, configuration, tools, security model, development guide.

[Unreleased]: https://github.com/forenxics/vinyldns-mcp/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/forenxics/vinyldns-mcp/releases/tag/v0.1.0
