"""Multi-hart bootstrap QEMU tests."""

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER, hex_field


class HartBootstrapTests(QemuTestCase):
    def test_hart_q001_all_discovered_harts_are_online(self) -> None:
        self.assertEqual(
            hex_field(self.output, "Online count"),
            hex_field(self.output, "Discovered count"),
        )

    def test_hart_q002_each_secondary_receives_one_start_request(self) -> None:
        discovered = hex_field(self.output, "Discovered count")
        self.assertGreater(discovered, 0)
        self.assertEqual(hex_field(self.output, "Start requests"), discovered - 1)

    def test_hart_q003_boot_hart_uses_a_valid_dense_index(self) -> None:
        self.assertLess(
            hex_field(self.output, "Boot hart index"),
            hex_field(self.output, "Discovered count"),
        )

    def test_hart_q004_completion_follows_multi_hart_bootstrap(self) -> None:
        self.assertLess(
            self.output.index("[Multi-hart Bootstrap]"),
            self.output.index(SUCCESS_MARKER),
        )
