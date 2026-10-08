# Project status

_Updated at the end of every working session._

## Current state: v0.1.0 released; v0.2.0 in progress (release binaries done, awaiting first real run)

**Last session: 2026-10-04 (second session: release pipeline)**

### Done this session
- Added `.github/workflows/release.yml`: tag-triggered or manual. It checks the tag
  against `Cargo.toml` and `CHANGELOG.md`, runs the tests, builds 5 targets (Linux
  x86_64 static musl, Linux ARM64, macOS ARM64 and x86_64, Windows x86_64), and
  publishes archives, `SHA256SUMS.txt`, notes from the changelog, and provenance
  attestations (public repositories only).
- Checked locally: both Linux targets cross-compile; packaging and checksums work;
  the static binary passed the full live smoke test against VinylDNS 0.20.2 and
  makes HTTPS connections using the system certificate store. actionlint
  (with shellcheck) reports no problems.
- Not checkable here: the macOS and Windows builds, and the publish job. These run
  for the first time at the next tag.
- Added the `--help` flag; CI now lints workflows and tests the changelog script.
- First CI run on `main`: Linux, macOS and Windows passed; the MSRV job failed (uuid needs
  Rust 1.89). Fixed: MSRV is now 1.89 (checked locally), and the actions use Node 24 versions.
- Docs: new `docs/RELEASING.md`; install-from-release steps in the README.

### Earlier (session 1)
- v0.1.0: 15 read tools, 8 write and pending-change tools with plan/confirm and
  elicitation; tested with unit, end-to-end and live tests; published to the
  private repository `Forenxics/vinyldns-mcp`.

### Next steps
1. Owner: create tag `v0.1.0` on GitHub (task 16c). Binaries start at v0.2.0,
   because the v0.1.0 commit predates the release scripts.
2. Check that CI is green after this push (MSRV fix, workflow lint job).
3. Continue v0.2.0: admin and zone tools (19, 20), live DNS cross-check (23),
   HTTP transport (22). Then release v0.2.0, which is the first real run of the
   release workflow.
4. Try it in Claude Code or Claude Desktop (task 17).

### Decisions
- Private repository `Forenxics/vinyldns-mcp`.
- SemVer and Keep a Changelog.
- v0.2.0 scope: tasks 16, 19, 20, 22, 23.

### Known limitations
- stdio transport only (one user per process).
- Plans live in memory and are lost when the server restarts. This is intended:
  a stale plan should not survive a restart.
- No zone, group, or batch approval administration yet.
