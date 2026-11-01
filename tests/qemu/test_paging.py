"""Paging QEMU tests."""

from tests.qemu.harness import QemuTestCase


class PagingTests(QemuTestCase):
    def test_vm_q003_sv39_remains_active_after_activation(self) -> None:
        self.assertIn("[Sv39 Paging]", self.output)
        self.assertIn("Permissions       : text=R-X rodata=R-- data/bss=RW-", self.output)
        self.assertIn("Translation       : active (identity mapped)", self.output)
