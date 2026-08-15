#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SKIP_BUILD=false
REQUESTED_VERSION=

usage() {
  printf 'Usage: %s [--skip-build] [version]\n' "$0"
}

while (($# > 0)); do
  case "$1" in
    --skip-build)
      SKIP_BUILD=true
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      printf 'Unknown option: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
    *)
      if [[ -n "$REQUESTED_VERSION" ]]; then
        usage >&2
        exit 2
      fi
      REQUESTED_VERSION=$1
      ;;
  esac
  shift
done

PACKAGE_VERSION=$(awk '
  /^\[package\]$/ { package = 1; next }
  /^\[/ { package = 0 }
  package && /^version = / { gsub(/[" ]/, "", $3); print $3; exit }
' "$ROOT_DIR/programs/zkp2p_solana/Cargo.toml")
if [[ -z "$PACKAGE_VERSION" ]]; then
  printf 'Unable to read the program package version\n' >&2
  exit 1
fi
if [[ -n "$REQUESTED_VERSION" && "$REQUESTED_VERSION" != "$PACKAGE_VERSION" ]]; then
  printf 'Requested version %s does not match Cargo version %s\n' "$REQUESTED_VERSION" "$PACKAGE_VERSION" >&2
  exit 1
fi

if [[ "$SKIP_BUILD" != true ]]; then
  command -v anchor >/dev/null
  (
    cd "$ROOT_DIR"
    anchor build --no-idl
    env -u RUSTUP_TOOLCHAIN anchor idl build -p zkp2p_solana -o target/idl/zkp2p_solana.json
  )
fi

PROGRAM_PATH="$ROOT_DIR/target/deploy/zkp2p_solana.so"
IDL_PATH="$ROOT_DIR/target/idl/zkp2p_solana.json"
[[ -s "$PROGRAM_PATH" ]] || { printf 'Missing SBF artifact: %s\n' "$PROGRAM_PATH" >&2; exit 1; }
[[ -s "$IDL_PATH" ]] || { printf 'Missing IDL artifact: %s\n' "$IDL_PATH" >&2; exit 1; }

PROGRAM_ID=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["address"])' "$IDL_PATH")
CONFIGURED_PROGRAM_ID=$(awk -F '"' '/^zkp2p_solana = / { print $2; exit }' "$ROOT_DIR/Anchor.toml")
if [[ "$PROGRAM_ID" != "$CONFIGURED_PROGRAM_ID" ]]; then
  printf 'IDL program ID %s does not match Anchor.toml %s\n' "$PROGRAM_ID" "$CONFIGURED_PROGRAM_ID" >&2
  exit 1
fi

DIST_DIR="$ROOT_DIR/dist"
ARCHIVE_BASENAME="zkp2p-solana-v${PACKAGE_VERSION}.tar.gz"
ARCHIVE_PATH="$DIST_DIR/$ARCHIVE_BASENAME"
STAGING_DIR=$(mktemp -d "${TMPDIR:-/tmp}/zkp2p-solana-package.XXXXXX")
trap 'rm -rf "$STAGING_DIR"' EXIT
PACKAGE_ROOT="$STAGING_DIR/zkp2p-solana-v${PACKAGE_VERSION}"
mkdir -p "$PACKAGE_ROOT/program" "$PACKAGE_ROOT/idl" "$PACKAGE_ROOT/docs"

install -m 0644 "$PROGRAM_PATH" "$PACKAGE_ROOT/program/zkp2p_solana.so"
install -m 0644 "$IDL_PATH" "$PACKAGE_ROOT/idl/zkp2p_solana.json"
install -m 0644 "$ROOT_DIR/README.md" "$PACKAGE_ROOT/README.md"
install -m 0644 "$ROOT_DIR/CHANGELOG.md" "$PACKAGE_ROOT/CHANGELOG.md"
install -m 0644 "$ROOT_DIR/LICENSE" "$PACKAGE_ROOT/LICENSE"
install -m 0644 "$ROOT_DIR/docs/PARITY.md" "$PACKAGE_ROOT/docs/PARITY.md"
install -m 0644 "$ROOT_DIR/docs/TESTING.md" "$PACKAGE_ROOT/docs/TESTING.md"

PROGRAM_SHA256=$(shasum -a 256 "$PROGRAM_PATH" | awk '{ print $1 }')
IDL_SHA256=$(shasum -a 256 "$IDL_PATH" | awk '{ print $1 }')
printf '%s\n' \
  '{' \
  "  \"package\": \"zkp2p-solana\"," \
  "  \"version\": \"${PACKAGE_VERSION}\"," \
  "  \"program_id\": \"${PROGRAM_ID}\"," \
  "  \"program_sha256\": \"${PROGRAM_SHA256}\"," \
  "  \"idl_sha256\": \"${IDL_SHA256}\"," \
  '  "rust": "1.89.0",' \
  '  "anchor": "1.1.2",' \
  '  "agave": "3.0.14"' \
  '}' > "$PACKAGE_ROOT/manifest.json"

(
  cd "$PACKAGE_ROOT"
  find . -type f ! -name SHA256SUMS -print | LC_ALL=C sort | while IFS= read -r path; do
    shasum -a 256 "$path"
  done > SHA256SUMS
  shasum -a 256 -c SHA256SUMS >/dev/null
)

mkdir -p "$DIST_DIR"
python3 - "$PACKAGE_ROOT" "$ARCHIVE_PATH" <<'PY'
import gzip
import os
from pathlib import Path
import sys
import tarfile

root = Path(sys.argv[1])
destination = Path(sys.argv[2])
with destination.open("wb") as raw:
    with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
            paths = [root, *sorted(root.rglob("*"), key=lambda path: path.as_posix())]
            for path in paths:
                info = archive.gettarinfo(str(path), arcname=str(path.relative_to(root.parent)))
                info.uid = 0
                info.gid = 0
                info.uname = "root"
                info.gname = "root"
                info.mtime = 0
                if info.isdir():
                    info.mode = 0o755
                    archive.addfile(info)
                else:
                    info.mode = 0o644
                    with path.open("rb") as source:
                        archive.addfile(info, source)
PY
(
  cd "$DIST_DIR"
  shasum -a 256 "$ARCHIVE_BASENAME" > "$ARCHIVE_BASENAME.sha256"
)
"$ROOT_DIR/scripts/verify-release-package.sh" "$ARCHIVE_PATH"
printf 'release_package=%s\n' "$ARCHIVE_PATH"
printf 'release_checksum=%s\n' "$ARCHIVE_PATH.sha256"
