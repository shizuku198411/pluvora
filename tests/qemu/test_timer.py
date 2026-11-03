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

    def test_timer_q003_preempts_and_resumes_kernel_thread(self) -> None:
        timer = self.output.index("[Supervisor Timer]")
        thread = self.output.index("[Kernel Thread Dispatch]")
        complete = self.output.index(SUCCESS_MARKER)
        self.assertLess(timer, thread)
        self.assertLess(thread, complete)
        self.assertIn("Timer preemption  : yes", self.output)
        self.assertIn("Resume mechanism  : sret", self.output)
