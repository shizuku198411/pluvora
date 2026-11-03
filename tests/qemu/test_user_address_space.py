"""User Address Space foundation QEMU tests."""

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

    def test_uas_q004_capability_authorized_mapping_is_published(self) -> None:
        self.assertIn("[Capability User Mapping]", self.output)
        self.assertRegex(self.output, r"Virtual address\s+: 0x0*1000")
        self.assertIn("Permissions       : read-write", self.output)
        self.assertIn("Memory authority  : MAP + READ + WRITE", self.output)
        self.assertIn("Address authority : MAP", self.output)
        self.assertIn("Mapping reference : held", self.output)
        self.assertIn("User PTE          : published last", self.output)

    def test_uas_q005_mapping_precedes_kernel_thread_dispatch(self) -> None:
        mapping = self.output.index("[Capability User Mapping]")
        dispatch = self.output.index("[Kernel Thread Dispatch]")
        self.assertLess(mapping, dispatch)

    def test_uas_q006_has_no_user_mapping_failure(self) -> None:
        self.assertNotIn("[ERROR] User Mapping", self.output)
        self.assertNotIn("[PANIC]", self.output)
