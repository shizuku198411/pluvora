#!/usr/bin/env python3
"""Run Pluvora's current QEMU integration tests with no external packages."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import time
import unittest


PROJECT_ROOT = Path(__file__).resolve().parents[2]
KERNEL_IMAGE = (
    PROJECT_ROOT
    / "kernel/target/riscv64imac-unknown-none-elf/release/Pluvora-kernel"
)
OPENSBI_IMAGE = (
    PROJECT_ROOT / "opensbi/build/platform/generic/firmware/fw_jump.bin"
)
BOOT_TIMEOUT_SECONDS = 15.0
SUCCESS_MARKER = "Early bootstrap completed successfully"


class QemuBootError(RuntimeError):
    pass


def qemu_command() -> list[str]:
    return [
        "qemu-system-riscv64",
        "-machine",
        "virt",
        "-m",
        "256M",
        "-smp",
        "4",
        "-nographic",
        "-no-reboot",
        "-bios",
        str(OPENSBI_IMAGE),
        "-kernel",
        str(KERNEL_IMAGE),
    ]


def boot_kernel() -> str:
    for image in (KERNEL_IMAGE, OPENSBI_IMAGE):
        if not image.is_file():
            raise QemuBootError(f"required image does not exist: {image}")

    process = subprocess.Popen(
        qemu_command(),
        cwd=PROJECT_ROOT,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    assert process.stdout is not None

    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + BOOT_TIMEOUT_SECONDS
    output = bytearray()

    try:
        while time.monotonic() < deadline:
            if process.poll() is not None:
                remaining = process.stdout.read()
                if remaining:
                    output.extend(remaining)
                break

            for key, _ in selector.select(timeout=0.1):
                chunk = os.read(key.fd, 4096)
                if chunk:
                    output.extend(chunk)

            decoded = output.decode("utf-8", errors="replace")
            if SUCCESS_MARKER in decoded:
                return decoded.replace("\r", "")

        decoded = output.decode("utf-8", errors="replace").replace("\r", "")
        if process.poll() is not None:
            raise QemuBootError(
                f"QEMU exited before the success marker (status={process.returncode})\n{decoded}"
            )
        raise QemuBootError(
            f"QEMU timed out after {BOOT_TIMEOUT_SECONDS:.0f}s\n{decoded}"
        )
    finally:
        selector.close()
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=2)


class PluvoraQemuTests(unittest.TestCase):
    output: str

    @classmethod
    def setUpClass(cls) -> None:
        cls.output = boot_kernel()

    def test_boot_005_bootstrap_completes(self) -> None:
        self.assertIn(SUCCESS_MARKER, self.output)
        self.assertNotIn("[PANIC]", self.output)
        self.assertNotIn("[Boot Failure]", self.output)
        self.assertNotIn("[Fatal S-mode Trap]", self.output)

    def test_boot_008_four_harts_are_discovered(self) -> None:
        self.assertIn("Discovered count  : 0x0000000000000004", self.output)
        for hart_id in range(4):
            expected = (
                f"hart: index=0x{hart_id:016x}, id=0x{hart_id:016x}"
            )
            self.assertIn(expected, self.output)

    def test_vm_q003_sv39_remains_active_after_activation(self) -> None:
        self.assertIn("[Sv39 Paging]", self.output)
        self.assertIn("Permissions       : text=R-X rodata=R-- data/bss=RW-", self.output)
        self.assertIn("Translation       : active (identity mapped)", self.output)

    def test_trap_q003_trap_entry_is_installed(self) -> None:
        self.assertIn("[S-mode traps]", self.output)
        self.assertRegex(self.output, r"stvec\s*:\s*0x[0-9a-f]{16}")
        self.assertRegex(self.output, r"Mode\s*:\s*direct")
        self.assertLess(self.output.index("[S-mode traps]"), self.output.index(SUCCESS_MARKER))


def filtered_suite(filter_text: str | None) -> unittest.TestSuite:
    tests = unittest.defaultTestLoader.loadTestsFromTestCase(PluvoraQemuTests)
    if not filter_text:
        return tests

    normalized = filter_text.lower().replace("-", "_")
    selected = [test for test in tests if normalized in test.id().lower()]
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
