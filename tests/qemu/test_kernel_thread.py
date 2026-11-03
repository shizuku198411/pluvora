"""Kernel Thread dispatch and context handoff QEMU tests."""

from harness import QemuTestCase, SUCCESS_MARKER


class KernelThreadDispatchTests(QemuTestCase):
    def test_kthread_q001_entry_runs_and_returns(self) -> None:
        self.assertIn("[Kernel Thread Dispatch]", self.output)
        self.assertIn("Entry executed    : yes", self.output)
        self.assertIn("Yield completed   : yes", self.output)
        self.assertIn("Resume completed  : yes", self.output)
        self.assertIn("Final state       : blocked", self.output)
        self.assertIn("Boot context      : restored", self.output)
        self.assertLess(
            self.output.index("[Kernel Thread Dispatch]"),
            self.output.index(SUCCESS_MARKER),
        )

    def test_kthread_q002_has_no_fatal_failure(self) -> None:
        self.assertNotIn("[Fatal S-mode Trap]", self.output)
        self.assertNotIn("[PANIC]", self.output)
