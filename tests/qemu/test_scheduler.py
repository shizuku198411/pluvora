"""Scheduler foundation QEMU tests."""

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER, hex_field


class SchedulerFoundationTests(QemuTestCase):
    def test_sched_q001_all_online_harts_are_published(self) -> None:
        self.assertEqual(hex_field(self.output, "Online hart mask"), 0xF)

    def test_sched_q002_boot_scheduler_starts_empty(self) -> None:
        self.assertEqual(hex_field(self.output, "Runnable threads"), 0)
        self.assertEqual(hex_field(self.output, "Running threads"), 0)

    def test_sched_q003_scheduler_initializes_before_completion(self) -> None:
        self.assertLess(self.output.index("[Scheduler]"), self.output.index(SUCCESS_MARKER))
        self.assertNotIn("[Boot Failure]", self.output)
        self.assertNotIn("[Fatal S-mode Trap]", self.output)

    def test_sched_q004_runtime_loops_are_published(self) -> None:
        self.assertIn("[Scheduler Runtime]", self.output)
        self.assertIn("Ready publication : complete", self.output)
        self.assertIn("Hart loops        : enabled", self.output)
        self.assertIn("Idle policy       : interrupt-driven WFI", self.output)
        self.assertIn("Wake notification : reschedule IPI", self.output)

    def test_sched_q005_runtime_report_precedes_completion(self) -> None:
        runtime = self.output.index("[Scheduler Runtime]")
        complete = self.output.index(SUCCESS_MARKER)
        self.assertLess(runtime, complete)
