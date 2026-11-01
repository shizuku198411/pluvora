# Developing and Testing Pluvora

This guide explains how to create the Pluvora development environment, build the kernel, inspect the resulting ELF image, and boot it with QEMU.

Pluvora uses [Canonical Workshop](https://ubuntu.com/workshop) to provide a reproducible containerized toolchain. The Workshop definition in `.workshop/kernel-dev.yaml` installs the required RISC-V tools, builds OpenSBI v1.9, and installs the Rust `riscv64imac-unknown-none-elf` target.

## Prerequisites

Workshop supports Ubuntu, other snap-enabled Linux distributions, and WSL2. It requires LXD 6.8 or newer.

Follow Canonical's [Install Workshop](https://ubuntu.com/workshop/docs/tutorial/part-1-get-started/#tut-install) instructions for the current prerequisites and installation procedure. At the time of writing, the documented snap installation is:

```sh
sudo snap install --channel=6/stable lxd
sudo snap install --classic workshop
```

If LXD is already installed through snap, follow the official documentation to refresh it to the required channel instead. Refer to the LXD documentation when using another installation method.

Confirm that Workshop is available:

```sh
snap info workshop
```

## Get the Source

Clone the repository and enter its root directory:

```sh
git clone https://github.com/shizuku198411/pluvora.git pluvora
cd pluvora
```

Run all remaining `workshop` commands from the repository root. Workshop discovers `.workshop/kernel-dev.yaml` from this directory.

## Launch the Development Environment

Create and start the development environment with verbose output:

```sh
workshop launch --verbose
```

The first launch may take some time because it installs the cross-development packages, clones and builds OpenSBI v1.9, and installs the Rust bare-metal target. Later launches reuse the prepared environment.

Check its state and configuration:

```sh
workshop info
```

The workshop should report a ready state before actions are run.

Project files are mounted into the workshop at `/project`. Changes made in the host checkout are therefore immediately visible inside the environment, and changes made under `/project` are visible on the host.

## Available Actions

Reusable commands are defined under `actions` in `.workshop/kernel-dev.yaml`. Invoke an action with:

```sh
workshop run -- <action>
```

The current actions are:

| Action | Purpose |
| --- | --- |
| `build` | Check formatting, run Clippy with warnings denied, and create the release kernel image |
| `format` | Apply Rust formatting fixes to the kernel source tree |
| `check-elf` | Display the ELF header and program headers with `llvm-readelf` |
| `run-kernel` | Boot the release kernel with OpenSBI and four QEMU harts |
| `qemu-boot-test` | Boot OpenSBI without loading the Pluvora kernel image |
| `test-all` | Run formatting, Clippy, all host-side Rust tests, and all Python-driven QEMU tests |
| `test-rust [FILTER]` | Run all Rust logic tests or only names containing `FILTER` |
| `test-qemu [FILTER]` | Run all QEMU integration tests or only names containing `FILTER` |
| `test-one rust\|qemu [FILTER]` | Select a test category explicitly and optionally apply a name filter |

Workshop parses actions each time `workshop run` is invoked, so changes to an action do not require a refresh of the environment.

If `build` reports formatting differences, apply them with:

```sh
workshop run -- format
```

Review the resulting source changes, then run `workshop run -- build` again.

## Run the Tests

Run every currently implemented check and test:

```sh
workshop run -- test-all
```

Host-side Rust tests cover hardware-independent parsing, range validation, allocation policy, Sv39 encoding, and trap decoding. QEMU tests are driven by Python's standard library and verify the current RV64 boot, paging, and trap smoke-test behavior.

Run one test category:

```sh
workshop run -- test-rust
workshop run -- test-qemu
```

Pass a substring to execute a specific test or group:

```sh
workshop run -- test-rust dtb_002
workshop run -- test-qemu trap_q003
```

The generic action accepts the category as its first argument:

```sh
workshop run -- test-one rust vm_001
workshop run -- test-one qemu boot_005
```

A filter that matches no QEMU test is an error. A QEMU timeout, early emulator exit, kernel panic, boot failure, or fatal trap also fails the relevant test rather than being treated as a successful shutdown.

## Continuous Integration

GitHub Actions runs the same `test-all` Workshop action for every pull request. The workflow is defined in `.github/workflows/test.yml` and uses Canonical's `launch-workshop` action to create an ephemeral `kernel-dev` environment on a GitHub-hosted Ubuntu runner.

The CI job intentionally delegates its checks to the repository's Workshop action:

```sh
workshop run -- test-all
```

This keeps formatting, Clippy, Rust logic tests, the release build, and Python-driven QEMU tests consistent between local development and CI. The external GitHub Actions used by the workflow are pinned to commit SHAs; version comments record the corresponding release tags.

## Build and Check the Kernel

Run the complete build check:

```sh
workshop run -- build
```

The action performs the equivalent of:

```sh
cd kernel
cargo fmt --check
cargo clippy --release -- -D warnings
cargo build --release
```

A successful build produces:

```text
kernel/target/riscv64imac-unknown-none-elf/release/Pluvora-kernel
```

Inspect the ELF metadata after building:

```sh
workshop run -- check-elf
```

Confirm that the ELF reports RISC-V as its machine architecture and that its loadable segments match the intended linker layout and permissions.

## Boot the Kernel

Build the latest source and start QEMU:

```sh
workshop run -- build
workshop run -- run-kernel
```

The `run-kernel` action launches:

- QEMU's RISC-V `virt` machine
- 256 MiB of RAM
- Four harts
- OpenSBI `fw_jump.bin`
- The Pluvora release kernel
- A serial console attached to the current terminal

The kernel should reach its successful bootstrap status after printing the discovered platform and initialized subsystems.

QEMU's `-nographic` console uses these key sequences:

- `Ctrl-A`, then `C`: switch between the serial console and QEMU monitor
- `Ctrl-A`, then `X`: terminate QEMU

## Test OpenSBI Separately

To verify the firmware and QEMU setup without loading Pluvora, run:

```sh
workshop run -- qemu-boot-test
```

An OpenSBI banner and platform information confirm that the firmware can start on the configured QEMU machine. Exit with `Ctrl-A`, then `X`.

## Recommended Development Loop

Use the following loop for ordinary changes:

1. Edit source files in the host checkout or under `/project` in the workshop.
2. Use `workshop run -- test-rust <FILTER>` while iterating on one logic test.
3. Run `workshop run -- test-all` before submitting the change.
4. Run `workshop run -- check-elf` when linker layout or section permissions change.
5. Run `workshop run -- run-kernel` for any additional interactive inspection.
6. Record the commands and observed behavior in the pull request.

## Using an Interactive Shell

For investigation that is not covered by an action, open a shell in the environment:

```sh
workshop shell
```

The project is available at `/project`:

```sh
cd /project
```

Prefer the named actions for documented build and test procedures. They keep contributor workflows consistent with the repository configuration.

Exit the shell with:

```sh
exit
```

## Managing the Environment

Stop the environment when it is not needed:

```sh
workshop stop
```

Start the existing environment again:

```sh
workshop start
```

If `.workshop/kernel-dev.yaml`, its SDK definitions, or provisioning hooks change substantially, apply the new environment configuration with:

```sh
workshop refresh --verbose
```

Editing only an action does not require `workshop refresh` because actions are read on every invocation.

## Troubleshooting

### The workshop is not ready

Check the environment status and recent setup tasks:

```sh
workshop info
workshop changes
workshop tasks
```

The project SDK health check expects QEMU, Clang/LLVM, Python, and `opensbi/build/platform/generic/firmware/fw_jump.bin` to exist.
