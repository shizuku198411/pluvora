"""S-mode trap setup QEMU tests."""

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER


class TrapTests(QemuTestCase):
    def test_trap_q003_trap_entry_is_installed(self) -> None:
        self.assertIn("[S-mode traps]", self.output)
        self.assertRegex(self.output, r"stvec\s*:\s*0x[0-9a-f]{16}")
        self.assertRegex(self.output, r"Mode\s*:\s*direct")
        self.assertLess(self.output.index("[S-mode traps]"), self.output.index(SUCCESS_MARKER))
