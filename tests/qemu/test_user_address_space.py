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
