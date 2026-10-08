#!/usr/bin/env python3
"""Tests for changelog_section.py. Run: python3 scripts/test_changelog_section.py"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
from changelog_section import section  # noqa: E402

SAMPLE = """# Changelog

## [Unreleased]
### Added
- next thing

## [0.2.0] - 2026-10-05
### Added
- release binaries

## [0.1.0] - 2026-10-04
### Added
- first

[Unreleased]: https://example/compare/v0.2.0...HEAD
[0.2.0]: https://example/releases/tag/v0.2.0
"""


class SectionTest(unittest.TestCase):
    def test_middle_section(self):
        self.assertEqual(section(SAMPLE, "v0.2.0"), "### Added\n- release binaries")

    def test_last_section_stops_at_link_references(self):
        self.assertEqual(section(SAMPLE, "0.1.0"), "### Added\n- first")

    def test_missing_version(self):
        self.assertEqual(section(SAMPLE, "9.9.9"), "")

    def test_prefix_does_not_match_longer_version(self):
        self.assertEqual(section(SAMPLE.replace("[0.2.0] -", "[0.2.0-rc.1] -"), "0.2.0"), "")

    def test_real_changelog_has_current_release(self):
        root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        with open(os.path.join(root, "CHANGELOG.md"), encoding="utf-8") as f:
            self.assertIn("MCP server over stdio", section(f.read(), "0.1.0"))


if __name__ == "__main__":
    unittest.main()
