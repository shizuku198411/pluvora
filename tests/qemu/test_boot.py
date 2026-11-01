"""Boot-sequence QEMU tests."""

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER, hex_field


class BootTests(QemuTestCase):
    def test_boot_005_bootstrap_completes(self) -> None:
        self.assertIn(SUCCESS_MARKER, self.output)
        self.assertNotIn("[PANIC]", self.output)
        self.assertNotIn("[Boot Failure]", self.output)
        self.assertNotIn("[Fatal S-mode Trap]", self.output)

    def test_boot_008_four_harts_are_discovered(self) -> None:
        self.assertEqual(hex_field(self.output, "Discovered count"), 4)
        for hart_id in range(4):
            expected = f"hart: index=0x{hart_id:016x}, id=0x{hart_id:016x}"
            self.assertIn(expected, self.output)
