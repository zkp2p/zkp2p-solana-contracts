#!/usr/bin/env bash
set -euo pipefail

: "${RUNNER_TEMP:?RUNNER_TEMP is required}"
: "${GITHUB_PATH:?GITHUB_PATH is required}"

ANCHOR_VERSION=1.1.2
ANCHOR_SHA256=fdea9979629e9416e5f5e5622ff6c11b8c691d1e559581ece368e903c0c980c1
AGAVE_VERSION=3.0.14
AGAVE_SHA256=65614325423316a48f57f1ceeaa91ca78f4516e165e305b38c01873cf8c6b8b4
TOOLCHAIN_ROOT="$RUNNER_TEMP/zkp2p-solana-toolchain-${ANCHOR_VERSION}-${AGAVE_VERSION}"

mkdir -p "$TOOLCHAIN_ROOT/bin"
if [[ ! -x "$TOOLCHAIN_ROOT/bin/anchor" ]]; then
  curl --fail --location --retry 3 --silent --show-error \
    "https://github.com/otter-sec/anchor/releases/download/v${ANCHOR_VERSION}/anchor-${ANCHOR_VERSION}-x86_64-unknown-linux-gnu" \
    --output "$TOOLCHAIN_ROOT/bin/anchor"
  printf '%s  %s\n' "$ANCHOR_SHA256" "$TOOLCHAIN_ROOT/bin/anchor" | sha256sum --check --status
  chmod 0755 "$TOOLCHAIN_ROOT/bin/anchor"
fi

if [[ ! -x "$TOOLCHAIN_ROOT/solana-release/bin/solana" ]]; then
  archive="$RUNNER_TEMP/solana-release-${AGAVE_VERSION}.tar.bz2"
  curl --fail --location --retry 3 --silent --show-error \
    "https://github.com/anza-xyz/agave/releases/download/v${AGAVE_VERSION}/solana-release-x86_64-unknown-linux-gnu.tar.bz2" \
    --output "$archive"
  printf '%s  %s\n' "$AGAVE_SHA256" "$archive" | sha256sum --check --status
  tar -xjf "$archive" -C "$TOOLCHAIN_ROOT"
fi

printf '%s\n' "$TOOLCHAIN_ROOT/bin" "$TOOLCHAIN_ROOT/solana-release/bin" >> "$GITHUB_PATH"
