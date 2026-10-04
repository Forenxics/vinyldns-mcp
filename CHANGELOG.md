# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
