"""Shared QEMU process and output helpers."""

from __future__ import annotations

import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import time
import unittest

PROJECT_ROOT = Path(__file__).resolve().parents[2]
KERNEL_IMAGE = PROJECT_ROOT / "kernel/target/riscv64imac-unknown-none-elf/release/Pluvora-kernel"
OPENSBI_IMAGE = PROJECT_ROOT / "opensbi/build/platform/generic/firmware/fw_jump.bin"
BOOT_TIMEOUT_SECONDS = 15.0
SUCCESS_MARKER = "Early bootstrap completed successfully"
_cached_output: str | None = None


class QemuBootError(RuntimeError):
    """Raised when the kernel cannot reach the expected boot milestone."""


def qemu_command() -> list[str]:
    return [
        "qemu-system-riscv64", "-machine", "virt", "-m", "256M", "-smp", "4",
        "-nographic", "-no-reboot", "-bios", str(OPENSBI_IMAGE), "-kernel", str(KERNEL_IMAGE),
    ]


def boot_kernel() -> str:
    for image in (KERNEL_IMAGE, OPENSBI_IMAGE):
        if not image.is_file():
            raise QemuBootError(f"required image does not exist: {image}")

    process = subprocess.Popen(
        qemu_command(), cwd=PROJECT_ROOT, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True,
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
        raise QemuBootError(f"QEMU timed out after {BOOT_TIMEOUT_SECONDS:.0f}s\n{decoded}")
    finally:
        selector.close()
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=2)


def kernel_output() -> str:
    global _cached_output
    if _cached_output is None:
        _cached_output = boot_kernel()
    return _cached_output


def hex_field(output: str, label: str) -> int:
    match = re.search(
        rf"^\s*{re.escape(label)}\s*:\s*0x([0-9a-fA-F]+)\s*$", output, re.MULTILINE,
    )
    if match is None:
        raise AssertionError(f"field not found in QEMU output: {label}")
    return int(match.group(1), 16)


class QemuTestCase(unittest.TestCase):
    output: str

    @classmethod
    def setUpClass(cls) -> None:
        cls.output = kernel_output()
