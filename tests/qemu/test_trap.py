"""S-mode trap setup QEMU tests."""

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER


class TrapTests(QemuTestCase):
    def test_trap_q003_trap_entry_is_installed(self) -> None:
        self.assertIn("[S-mode traps]", self.output)
        self.assertRegex(self.output, r"stvec\s*:\s*0x[0-9a-f]{16}")
        self.assertRegex(self.output, r"Mode\s*:\s*direct")
        self.assertLess(self.output.index("[S-mode traps]"), self.output.index(SUCCESS_MARKER))

    def test_trap_q004_user_trap_boundary_is_idle(self) -> None:
        self.assertIn("[User Trap Boundary]", self.output)
        self.assertRegex(self.output, r"Idle sscratch\s+: 0x0+")
        self.assertIn("U-mode entry      : armed kernel stack", self.output)
        self.assertIn("Trap stack pivot  : sscratch exchange", self.output)
        self.assertIn("Handler scratch   : cleared", self.output)
        self.assertIn(
            "User exceptions   : termination policy classified", self.output
        )
        self.assertIn("Runnable users    : disabled", self.output)

    def test_trap_q005_supervisor_traps_still_complete(self) -> None:
        boundary = self.output.index("[User Trap Boundary]")
        timer = self.output.index("[Supervisor Timer]")
        complete = self.output.index(SUCCESS_MARKER)
        self.assertLess(boundary, timer)
        self.assertLess(timer, complete)

    def test_trap_q006_has_no_unexpected_user_trap(self) -> None:
        self.assertNotIn("[Unexpected U-mode Trap]", self.output)
        self.assertNotIn("[PANIC]", self.output)
