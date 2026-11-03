"""Supervisor timer QEMU tests."""

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER, hex_field


class SupervisorTimerTests(QemuTestCase):
    def test_timer_q001_interrupt_is_delivered(self) -> None:
        self.assertIn("[Supervisor Timer]", self.output)
        self.assertGreater(hex_field(self.output, "Timebase frequency"), 0)
        self.assertEqual(hex_field(self.output, "Tick rate"), 100)
        self.assertGreaterEqual(hex_field(self.output, "Observed ticks"), 1)
        self.assertIn("Reschedule request: published", self.output)

    def test_timer_q002_boot_continues_after_timer_trap(self) -> None:
        self.assertLess(self.output.index("[Supervisor Timer]"), self.output.index(SUCCESS_MARKER))
        self.assertNotIn("[Fatal S-mode Trap]", self.output)
        self.assertNotIn("[Boot Failure]", self.output)
        self.assertNotIn("[PANIC]", self.output)
