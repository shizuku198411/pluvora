#!/usr/bin/env bash
set -euo pipefail

readonly PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'EOF'
Usage:
  scripts/test.sh all
  scripts/test.sh rust [FILTER]
  scripts/test.sh qemu [FILTER]

Examples:
  scripts/test.sh rust dtb_002
  scripts/test.sh qemu trap_q001
EOF
}

host_target() {
  rustc -vV | sed -n 's/^host: //p'
}

run_rust_tests() {
  local filter="${1:-}"
  local target
  target="$(host_target)"

  cd "${PROJECT_ROOT}/kernel"
  if [[ -n "${filter}" ]]; then
    cargo test --lib --target "${target}" "${filter}"
  else
    cargo test --lib --target "${target}"
  fi
}

build_kernel() {
  cd "${PROJECT_ROOT}/kernel"
  cargo build --release
}

run_qemu_tests() {
  local filter="${1:-}"
  build_kernel
  cd "${PROJECT_ROOT}"
  if [[ -n "${filter}" ]]; then
    python3 tests/qemu/run_tests.py "${filter}"
  else
    python3 tests/qemu/run_tests.py
  fi
}

readonly suite="${1:-}"
case "${suite}" in
  all)
    if [[ $# -ne 1 ]]; then
      usage
      exit 2
    fi
    cd "${PROJECT_ROOT}/kernel"
    cargo fmt --check
    cargo clippy --release -- -D warnings
    run_rust_tests
    run_qemu_tests
    ;;
  rust)
    if [[ $# -gt 2 ]]; then
      usage
      exit 2
    fi
    run_rust_tests "${2:-}"
    ;;
  qemu)
    if [[ $# -gt 2 ]]; then
      usage
      exit 2
    fi
    run_qemu_tests "${2:-}"
    ;;
  *)
    usage
    exit 2
    ;;
esac
