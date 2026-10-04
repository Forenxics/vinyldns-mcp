#!/usr/bin/env python3
"""Print the CHANGELOG.md section for one version (used as GitHub release notes).

Usage: changelog_section.py VERSION [CHANGELOG.md]
       VERSION may have a leading "v" (e.g. v0.2.0).
Exits with status 1 if the version has no section or the section is empty,
so a release cannot be published without release notes.
"""
import re
import sys


def section(changelog: str, version: str) -> str:
    version = version.removeprefix("v")
    heading = re.compile(rf"^## \[{re.escape(version)}\](?:\s|$)")
    lines, out, inside = changelog.splitlines(), [], False
    for line in lines:
        if inside and (line.startswith("## ") or re.match(r"^\[[^\]]+\]: ", line)):
            break
        if inside:
            out.append(line)
        elif heading.match(line):
            inside = True
    return "\n".join(out).strip()


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    path = sys.argv[2] if len(sys.argv) > 2 else "CHANGELOG.md"
    with open(path, encoding="utf-8") as f:
        notes = section(f.read(), sys.argv[1])
    if not notes:
        print(f"error: no non-empty section for {sys.argv[1]} in {path}", file=sys.stderr)
        return 1
    print(notes)
    return 0


if __name__ == "__main__":
    sys.exit(main())
