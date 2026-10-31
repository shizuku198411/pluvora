<p>
  <img src="./assets/img/Pluvora_logo_tr.png" alt="Pluvora Logo" width="250">
</p>

# Pluvora

Pluvora is a microkernel for 64-bit RISC-V systems.

It isolates user processes, system services, and device drivers wherever possible, limiting the impact of a compromised component and preventing it from spreading across the entire system. The kernel runs in S-mode and is designed for multi-hart environments from the outset.

## Background

In a large monolithic kernel, many components operate within the same privileged address space. A vulnerability in any one of those components can therefore lead to kernel-level privilege escalation or compromise of the entire system.

Pluvora does not assume that vulnerabilities can be eliminated completely. Instead, it assumes that vulnerabilities and implementation defects may exist, then minimizes their blast radius by restricting the resources and privileges available to each component.

## Objectives

Pluvora primarily aims to strengthen system **confidentiality** and **integrity**.

Its design is built around the following principles:

- **Memory Ownership** — Every memory region has an explicit owner and a clearly defined set of access rights.
- **Capability-Based Access Control** — Access to resources is granted explicitly through capabilities, without allowing privilege amplification.
- **Least Privilege** — Each process and service receives only the capabilities required to perform its role.
- **Revocation** — Previously granted capabilities, mappings, and TLB entries can be invalidated safely.
- **Isolation by Default** — Resources remain inaccessible until the required capabilities are explicitly granted.
- **Small Trusted Computing Base** — The kernel contains only essential mechanisms, while filesystems, network stacks, device drivers, and other services run in user space.

Pluvora is not merely an effort to build a kernel that runs on RISC-V. It is a project to build a compromise-resilient microkernel with memory ownership, capabilities, and isolation as its foundational principles.

See the [Kernel Design Policy](docs/design/kernel-policy.md) for the complete design.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for development and submission guidelines. Please report security vulnerabilities according to [SECURITY.md](SECURITY.md).

## License

Pluvora is distributed under the [MIT License](LICENSE).
