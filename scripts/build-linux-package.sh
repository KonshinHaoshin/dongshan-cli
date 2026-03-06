#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
BIN_PATH="${ROOT_DIR}/target/release/dongshan"

cd "${ROOT_DIR}"

echo "[1/4] Building release binary..."
cargo build --release

echo "[2/4] Preparing dist directory..."
mkdir -p "${DIST_DIR}"
cp "${BIN_PATH}" "${DIST_DIR}/dongshan"
chmod +x "${DIST_DIR}/dongshan"

echo "[3/4] Creating tarball..."
tar -C "${DIST_DIR}" -czf "${DIST_DIR}/dongshan-linux-x86_64.tar.gz" dongshan

echo "[4/4] Generating checksum..."
sha256sum "${DIST_DIR}/dongshan-linux-x86_64.tar.gz" > "${DIST_DIR}/SHA256SUMS-linux.txt"

echo "Done:"
echo "  - ${DIST_DIR}/dongshan-linux-x86_64.tar.gz"
echo "  - ${DIST_DIR}/SHA256SUMS-linux.txt"
