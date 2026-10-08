#!/usr/bin/env bash
set -euo pipefail

BIN_NAME="gh-actions-tui"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${HOME}/.local/bin"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found. Install Rust from https://rustup.rs and try again." >&2
  exit 1
fi

echo "==> Building ${BIN_NAME} in release mode..."
(cd "${REPO_DIR}" && cargo build --release)

BUILT_BIN="${REPO_DIR}/target/release/${BIN_NAME}"
if [ ! -x "${BUILT_BIN}" ]; then
  echo "error: build succeeded but binary not found at ${BUILT_BIN}" >&2
  exit 1
fi

echo "==> Installing to ${INSTALL_DIR}..."
mkdir -p "${INSTALL_DIR}"
ln -sf "${BUILT_BIN}" "${INSTALL_DIR}/${BIN_NAME}"

echo "==> Installed: ${INSTALL_DIR}/${BIN_NAME} -> ${BUILT_BIN}"

case ":${PATH}:" in
  *":${INSTALL_DIR}:"*)
    echo "==> Done. Run '${BIN_NAME}' from anywhere."
    ;;
  *)
    echo
    echo "warning: ${INSTALL_DIR} is not on your PATH."
    echo "Add this to your shell profile (e.g. ~/.zshrc or ~/.bashrc):"
    echo
    echo "  export PATH=\"\${HOME}/.local/bin:\${PATH}\""
    echo
    ;;
esac
