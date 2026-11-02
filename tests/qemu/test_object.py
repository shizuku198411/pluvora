"""Kernel Object and bootstrap Capability Space QEMU tests."""

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER, hex_field


class KernelObjectTests(QemuTestCase):
    def test_object_q000_bootstrap_capability_space_is_registered(self) -> None:
        self.assertEqual(hex_field(self.output, "Live CSpaces"), 1)
        self.assertGreaterEqual(hex_field(self.output, "CSpace capacity"), 1)

    def test_object_q001_kernel_address_space_is_registered(self) -> None:
        self.assertIn("[Kernel Objects]", self.output)
        self.assertEqual(hex_field(self.output, "Live objects"), 1)
        self.assertEqual(hex_field(self.output, "Live capabilities"), 1)

    def test_object_q002_initial_handle_uses_nonzero_generation(self) -> None:
        self.assertLess(
            hex_field(self.output, "Kernel cap slot"),
            hex_field(self.output, "Capability capacity"),
        )
        self.assertGreater(hex_field(self.output, "Kernel cap generation"), 0)

    def test_object_q003_object_initialization_precedes_completion(self) -> None:
        self.assertLess(
            self.output.index("[Kernel Objects]"),
            self.output.index(SUCCESS_MARKER),
        )
        self.assertNotIn("[Boot Failure]", self.output)
