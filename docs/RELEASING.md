# Releasing

Releases are built and published by `.github/workflows/release.yml`.
Versions follow [Semantic Versioning](https://semver.org), and release notes
come from [`CHANGELOG.md`](../CHANGELOG.md) (Keep a Changelog format).

## Published artifacts

| Archive | Platform | Notes |
|---|---|---|
| `vinyldns-mcp-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz` | Linux x86_64 | Fully static; runs on any distribution |
| `vinyldns-mcp-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz` | Linux ARM64 | Needs glibc 2.35+ (Ubuntu 22.04+, Debian 12+, RHEL 10+) |
| `vinyldns-mcp-vX.Y.Z-aarch64-apple-darwin.tar.gz` | macOS, Apple silicon | Unsigned (see below) |
| `vinyldns-mcp-vX.Y.Z-x86_64-apple-darwin.tar.gz` | macOS, Intel | Unsigned (see below) |
| `vinyldns-mcp-vX.Y.Z-x86_64-pc-windows-msvc.zip` | Windows x86_64 | Unsigned (see below) |
| `SHA256SUMS.txt` | – | SHA-256 checksums of all archives |

Each archive contains the binary, `README.md`, `LICENSE` and `CHANGELOG.md`.
In public repositories, each archive also gets a
[build provenance attestation](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations),
which can be checked with `gh attestation verify <archive> --repo Forenxics/vinyldns-mcp`.
(Attestations are skipped in private repositories, where they need GitHub
Enterprise Cloud.)

## Release checklist

1. Make sure `main` is green in CI.
2. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`, add a
   new empty `## [Unreleased]` above it, and update the link references at
   the bottom.
3. Set `version = "X.Y.Z"` in `Cargo.toml` and run `cargo build`, so that
   `Cargo.lock` picks up the new version.
4. Update `docs/TASKS.md` and `docs/PROJECT_STATUS.md`.
5. Commit (`Release vX.Y.Z`) and push to `main`.
6. Create the tag, using **one** of:
   - `git tag -a vX.Y.Z -m "vinyldns-mcp vX.Y.Z" && git push origin vX.Y.Z`
   - on GitHub: **Releases → Draft a new release → choose a tag → create `vX.Y.Z`
     on `main`**, then **Publish**. If the tag push doesn't start the workflow,
     run it manually (next point).
   - **Actions → Release → Run workflow**, with an existing tag.
7. Watch the **Release** workflow. It:
   1. checks that the tag matches `Cargo.toml` and that `CHANGELOG.md` has a
      section for it (otherwise it stops before building anything);
   2. runs the test suite;
   3. builds the five targets and runs `--version` on the ones the runner can
      execute;
   4. packages them, writes `SHA256SUMS.txt`, and attests provenance (public
      repositories only);
   5. creates the GitHub release with the changelog section as its notes, or
      uploads the assets to the release if it already exists.

Tags with a pre-release suffix (`v0.2.0-rc.1`) are published as pre-releases.
They also need a matching `Cargo.toml` version and changelog section.

Re-running the workflow for the same tag is safe: the assets are replaced.

## Unsigned binaries

The binaries are not code-signed yet.

- **macOS:** after extracting, run `xattr -d com.apple.quarantine vinyldns-mcp` (or
  allow it under System Settings → Privacy & Security).
- **Windows:** SmartScreen may warn on first run. Check the checksum, then choose
  **More info → Run anyway**.

## Checking the release pipeline locally

```sh
actionlint .github/workflows/*.yml                 # pip install actionlint-py
python3 scripts/test_changelog_section.py
python3 scripts/changelog_section.py vX.Y.Z        # preview the release notes
# Linux cross builds (Debian/Ubuntu: apt install musl-tools gcc-aarch64-linux-gnu)
CC_x86_64_unknown_linux_musl=musl-gcc cargo build --release --target x86_64-unknown-linux-musl
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc cargo build --release --target aarch64-unknown-linux-gnu
```
