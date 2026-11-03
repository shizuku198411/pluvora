"""RISC-V context switch QEMU tests."""

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER, hex_field


class ContextSwitchTests(QemuTestCase):
    def test_context_q001_smoke_returns_to_boot_context(self) -> None:
        self.assertIn("[Context Switch]", self.output)
        self.assertEqual(hex_field(self.output, "Smoke stack bytes"), 16 * 1024)
        self.assertLess(self.output.index("[Context Switch]"), self.output.index(SUCCESS_MARKER))
        self.assertNotIn("[PANIC]", self.output)
        self.assertNotIn("[Fatal S-mode Trap]", self.output)
