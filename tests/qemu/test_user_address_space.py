"""User Address Space foundation QEMU tests."""

import re

from tests.qemu.harness import QemuTestCase, SUCCESS_MARKER


class UserAddressSpaceTests(QemuTestCase):
    def test_uas_q001_private_root_contract_is_reported(self) -> None:
        self.assertIn("[User Address Space]", self.output)
        self.assertRegex(self.output, r"Smoke root\s+: 0x[0-9a-f]{16}")
        self.assertRegex(self.output, r"Owned PT frames\s+: 0x0*1")
        self.assertIn("Kernel mappings   : supervisor-only shared", self.output)
        self.assertIn("User mappings     : private U-bit subtree", self.output)
        self.assertIn("W^X policy        : enforced by type", self.output)
        self.assertIn("Release smoke     : verified", self.output)

    def test_uas_q002_smoke_precedes_boot_completion(self) -> None:
        report = self.output.index("[User Address Space]")
        complete = self.output.index(SUCCESS_MARKER)
        self.assertLess(report, complete)

    def test_uas_q003_has_no_address_space_failure(self) -> None:
        self.assertNotIn("[ERROR] User Address Space", self.output)
        self.assertNotIn("[PANIC]", self.output)

    def test_uas_q004_capability_authorized_user_image_is_published(self) -> None:
        self.assertIn("[Capability User Image]", self.output)
        self.assertRegex(self.output, r"Text virtual\s+: 0x0*1000")
        self.assertRegex(self.output, r"Stack virtual\s+: 0x0*3000")
        self.assertRegex(self.output, r"Initial stack top\s+: 0x0*4000")
        self.assertRegex(self.output, r"Guard page\s+: 0x0*2000")
        self.assertIn("Text permissions  : read-execute", self.output)
        self.assertIn("Stack permissions : read-write", self.output)
        self.assertIn("Guard mapping     : absent", self.output)
        self.assertIn("Mapping references: two held", self.output)
        self.assertIn("Shared PT path    : verified", self.output)

    def test_uas_q005_user_image_precedes_kernel_thread_dispatch(self) -> None:
        mapping = self.output.index("[Capability User Image]")
        dispatch = self.output.index("[Kernel Thread Dispatch]")
        self.assertLess(mapping, dispatch)

    def test_uas_q006_has_no_user_mapping_failure(self) -> None:
        self.assertNotIn("[ERROR] User Mapping", self.output)
        self.assertNotIn("[PANIC]", self.output)

    def test_uas_q007_text_and_stack_use_distinct_memory_objects(self) -> None:
        text = re.search(r"Text memory\s+: (0x[0-9a-f]+)", self.output)
        stack = re.search(r"Stack memory\s+: (0x[0-9a-f]+)", self.output)
        self.assertIsNotNone(text)
        self.assertIsNotNone(stack)
        self.assertNotEqual(text.group(1), stack.group(1))

    def test_uas_q011_user_mode_dispatch_returns_through_ecall(self) -> None:
        self.assertIn("[User-mode Dispatch]", self.output)
        self.assertIn("U-mode entered    : yes", self.output)
        self.assertIn("Exit mechanism    : user ecall", self.output)
        self.assertIn("Final state       : Stopped", self.output)
        self.assertIn("Kernel return     : verified", self.output)

    def test_uas_q012_user_address_space_is_inactive_but_resident_after_exit(
        self,
    ) -> None:
        active = re.search(r"Active hart mask\s*: (0x[0-9a-f]+)", self.output)
        resident = re.search(r"Resident hart mask\s*: (0x[0-9a-f]+)", self.output)
        current_asid = re.search(r"Current ASID\s*: (0x[0-9a-f]+)", self.output)
        self.assertIsNotNone(active)
        self.assertIsNotNone(resident)
        self.assertIsNotNone(current_asid)
        self.assertEqual(int(active.group(1), 16), 0)
        resident_mask = int(resident.group(1), 16)
        self.assertNotEqual(resident_mask, 0)
        self.assertEqual(resident_mask & (resident_mask - 1), 0)
        self.assertEqual(int(current_asid.group(1), 16), 0)

    def test_uas_q013_user_dispatch_has_no_unexpected_or_fatal_trap(self) -> None:
        self.assertNotIn("[Unexpected U-mode Trap]", self.output)
        self.assertNotIn("[Fatal S-mode Trap]", self.output)
        self.assertNotIn("[PANIC]", self.output)

    def test_uas_q014_two_user_threads_yield_and_exit(self) -> None:
        self.assertIn("[User Scheduling]", self.output)
        self.assertRegex(self.output, r"User threads\s*: 0x0*2")
        self.assertRegex(self.output, r"Completed yields\s*: 0x0*6")
        self.assertIn("Same Process/AS   : verified", self.output)
        self.assertIn("Round robin       : verified", self.output)
        self.assertIn("Final states      : Stopped / Stopped", self.output)

    def test_uas_q015_user_switches_leave_no_running_publication(self) -> None:
        scheduling = self.output[self.output.index("[User Scheduling]") :]
        self.assertRegex(scheduling, r"Runnable threads\s*: 0x0+")
        self.assertRegex(scheduling, r"Running threads\s*: 0x0+")
        self.assertRegex(scheduling, r"Active hart mask\s*: 0x0+")

    def test_uas_q016_repeated_user_dispatch_has_no_fatal_path(self) -> None:
        self.assertNotIn("[Unexpected U-mode Trap]", self.output)
        self.assertNotIn("[Fatal S-mode Trap]", self.output)
        self.assertNotIn("[PANIC]", self.output)

    def test_uas_q017_user_exit_reaches_quiescent_stopping_state(self) -> None:
        self.assertIn("[Process Termination]", self.output)
        self.assertIn("Reason            : exit", self.output)
        self.assertIn("Process state     : Stopping", self.output)
        self.assertIn("Progress          : Quiescent", self.output)
        self.assertRegex(self.output, r"Live threads\s+: 0x0+")
        self.assertRegex(self.output, r"Active hart mask\s+: 0x0+")

    def test_uas_q018_termination_does_not_panic(self) -> None:
        self.assertNotIn("[Unexpected U-mode Trap]", self.output)
        self.assertNotIn("[Fatal S-mode Trap]", self.output)
        self.assertNotIn("[PANIC]", self.output)
