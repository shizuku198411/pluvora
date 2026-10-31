# Contributing to Pluvora

Thank you for your interest in Pluvora. Contributions of code, documentation, tests, design feedback, and bug reports are welcome.

## Before You Start

- Read the [Kernel Design Policy](docs/design/kernel-policy.md).
- Set up the toolchain and learn the repository's build and QEMU workflow in [Developing and Testing Pluvora](docs/development/getting-started.md).
- Search existing issues and pull requests before opening a new one.
- For substantial changes, open a design discussion or proposal issue before implementation.
- Do not use a public issue to report a security vulnerability. Follow [SECURITY.md](SECURITY.md) instead.

Pluvora is in an early stage of development. Interfaces and implementation details may change, but changes must remain consistent with the project's capability-based security model and multi-hart requirements.

## Development Principles

Contributions should:

- Preserve least privilege and isolation by default.
- Avoid introducing implicit authority or privilege amplification.
- Treat multi-hart correctness as a baseline requirement.
- Define synchronization, object lifetime, and failure behavior explicitly.
- Favor clear and verifiable implementations before performance optimizations.
- Keep policy in user space and mechanisms in the kernel wherever practical.

## Submitting a Change

1. Fork the repository and create a focused branch.
2. Make one logically coherent change per pull request.
3. Add or update tests and documentation where applicable.
4. Run all available formatting, build, and test checks.
5. Open a pull request using the repository template.

The standard local checks use the named actions in `.workshop/kernel-dev.yaml`:

```sh
workshop run -- build
workshop run -- run-kernel
```

Run `workshop run -- check-elf` as well when a change affects the linker script, memory layout, or ELF section permissions. The full setup and test procedure is documented in [Developing and Testing Pluvora](docs/development/getting-started.md).

Commit messages should be concise and use the imperative mood when practical, for example:

```text
memory: add physical page allocator
ipc: reject stale endpoint capabilities
docs: clarify TLB shootdown completion
```

## Pull Request Expectations

A pull request should explain:

- What changed and why
- How the change was tested
- Any security or multi-hart implications
- Any known limitations or follow-up work

Keep unrelated formatting and refactoring out of functional changes. Review feedback should be resolved through additional commits; maintainers may squash commits when merging.

## Documentation

Public documentation belongs under `docs/`. Internal notes and work-in-progress material must not be referenced by public documentation.

Design changes should update the relevant public document in the same pull request.

## Code of Conduct

Participation in this project is governed by the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

By contributing to Pluvora, you agree that your contributions are licensed under the [MIT License](LICENSE).
