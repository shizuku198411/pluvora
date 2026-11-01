#!/usr/bin/env python3
"""Discover and run Pluvora's QEMU integration tests."""

from __future__ import annotations

import argparse
from pathlib import Path
import sys
import unittest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(PROJECT_ROOT))

from tests.qemu.harness import QemuBootError

QEMU_TEST_DIR = Path(__file__).resolve().parent


def iter_tests(suite: unittest.TestSuite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from iter_tests(item)
        else:
            yield item


def discovered_suite() -> unittest.TestSuite:
    return unittest.defaultTestLoader.discover(
        start_dir=str(QEMU_TEST_DIR),
        pattern="test_*.py",
        top_level_dir=str(PROJECT_ROOT),
    )


def filtered_suite(filter_text: str | None) -> unittest.TestSuite:
    tests = discovered_suite()
    if not filter_text:
        return tests

    normalized = filter_text.lower().replace("-", "_")
    selected = [test for test in iter_tests(tests) if normalized in test.id().lower()]
    if not selected:
        raise QemuBootError(f"no QEMU test matches filter: {filter_text}")
    return unittest.TestSuite(selected)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("filter", nargs="?", help="substring of one test ID/name")
    args = parser.parse_args()

    try:
        suite = filtered_suite(args.filter)
    except QemuBootError as error:
        print(error, file=sys.stderr)
        return 2

    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
