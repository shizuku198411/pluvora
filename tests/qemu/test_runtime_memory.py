"""Runtime frame allocator and kernel stack QEMU tests."""

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER, hex_field


class RuntimeMemoryTests(QemuTestCase):
    def test_memory_q001_runtime_allocator_has_free_frames(self) -> None:
        self.assertGreater(hex_field(self.output, "Free frames"), 0)

    def test_memory_q002_kernel_stack_round_trip_preserves_capacity(self) -> None:
        self.assertEqual(hex_field(self.output, "Kernel stack pages"), 4)
        self.assertEqual(hex_field(self.output, "Stack top alignment"), 0)

    def test_memory_q003_runtime_memory_precedes_success(self) -> None:
        self.assertLess(self.output.index("[Runtime Memory]"), self.output.index(SUCCESS_MARKER))
        self.assertNotIn("[Boot Failure]", self.output)
        self.assertNotIn("[Fatal S-mode Trap]", self.output)
