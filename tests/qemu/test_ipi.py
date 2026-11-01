"""IPI foundation QEMU regression tests."""

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER, hex_field


class IpiFoundationTests(QemuTestCase):
    def test_ipi_q001_all_harts_remain_online_after_ipi_setup(self) -> None:
        self.assertEqual(
            hex_field(self.output, "Online count"),
            hex_field(self.output, "Discovered count"),
        )

    def test_ipi_q002_trap_setup_reaches_boot_completion_without_fatal_trap(self) -> None:
        self.assertIn("[S-mode traps]", self.output)
        self.assertNotIn("[Fatal S-mode Trap]", self.output)
        self.assertLess(
            self.output.index("[S-mode traps]"),
            self.output.index(SUCCESS_MARKER),
        )
