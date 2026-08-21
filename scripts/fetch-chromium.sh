#!/usr/bin/env bash
# Fetch the pinned headless Chromium used by the dashboard browser tests
# (crates/careerai-dashboard/tests/browser_it.rs).
#
# The test resolves the executable via $CAREERAI_CHROMIUM, then
# ~/.cache/careerai/chromium/chrome-headless-shell-linux64/chrome-headless-shell
# (what this script installs), then common system locations.
#
# No package manager or sudo needed: downloads the official
# chrome-headless-shell zip from Chrome for Testing.
set -euo pipefail

VERSION="152.0.7977.54"
SHA256="11cedb5568cd374a76eb738e40bd434cd0c9956820fb406b8bd9edca53428d3e"
CACHE_DIR="${CAREERAI_CHROMIUM_CACHE:-${HOME:-}/.cache/careerai/chromium}"
INSTALL_DIR="${CACHE_DIR}/chrome-headless-shell-linux64"
EXE="${INSTALL_DIR}/chrome-headless-shell"

if [[ -x "${EXE}" ]]; then
  echo "chrome-headless-shell already present: ${EXE}"
  exit 0
fi

URL="https://storage.googleapis.com/chrome-for-testing-public/${VERSION}/linux64/chrome-headless-shell-linux64.zip"

mkdir -p "${CACHE_DIR}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

echo "downloading chrome-headless-shell ${VERSION} ..."
curl -fsSL --retry 3 "${URL}" -o "${TMP_DIR}/shell.zip"

ACTUAL="$(sha256sum "${TMP_DIR}/shell.zip" | awk '{print $1}')"
if [[ "${ACTUAL}" != "${SHA256}" ]]; then
  echo "sha256 mismatch: expected ${SHA256}, got ${ACTUAL}" >&2
  exit 1
fi

unzip -q "${TMP_DIR}/shell.zip" -d "${CACHE_DIR}"
chmod +x "${EXE}"
echo "installed: ${EXE}"
