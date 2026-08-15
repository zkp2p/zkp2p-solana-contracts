#!/usr/bin/env bash
set -euo pipefail

if (($# != 1)); then
  printf 'Usage: %s <release.tar.gz>\n' "$0" >&2
  exit 2
fi

ARCHIVE_PATH=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
[[ -s "$ARCHIVE_PATH" ]] || { printf 'Missing release archive: %s\n' "$ARCHIVE_PATH" >&2; exit 1; }
if [[ -f "$ARCHIVE_PATH.sha256" ]]; then
  (cd "$(dirname "$ARCHIVE_PATH")" && shasum -a 256 -c "$(basename "$ARCHIVE_PATH").sha256" >/dev/null)
fi

LISTING=$(tar -tzf "$ARCHIVE_PATH")
if printf '%s\n' "$LISTING" | awk '
  /^\// { bad = 1 }
  /(^|\/)\.\.($|\/)/ { bad = 1 }
  END { exit bad ? 0 : 1 }
'; then
  printf 'Archive contains an unsafe path\n' >&2
  exit 1
fi
if printf '%s\n' "$LISTING" | grep -Eiq '(^|/)([^/]*(keypair|secret)[^/]*|\.env)($|/)'; then
  printf 'Archive contains a forbidden secret-bearing filename\n' >&2
  exit 1
fi

VERIFY_DIR=$(mktemp -d "${TMPDIR:-/tmp}/zkp2p-solana-verify.XXXXXX")
trap 'rm -rf "$VERIFY_DIR"' EXIT
tar -xzf "$ARCHIVE_PATH" -C "$VERIFY_DIR"
PACKAGE_DIRS=()
if command -v mapfile >/dev/null 2>&1; then
  mapfile -t PACKAGE_DIRS < <(find "$VERIFY_DIR" -mindepth 1 -maxdepth 1 -type d)
else
  while IFS= read -r path; do PACKAGE_DIRS+=("$path"); done < <(find "$VERIFY_DIR" -mindepth 1 -maxdepth 1 -type d)
fi
if ((${#PACKAGE_DIRS[@]} != 1)); then
  printf 'Archive must contain exactly one root directory\n' >&2
  exit 1
fi
PACKAGE_ROOT=${PACKAGE_DIRS[0]}
for required in \
  program/zkp2p_solana.so \
  idl/zkp2p_solana.json \
  manifest.json \
  SHA256SUMS \
  README.md \
  CHANGELOG.md \
  LICENSE \
  docs/PARITY.md \
  docs/TESTING.md; do
  [[ -s "$PACKAGE_ROOT/$required" ]] || { printf 'Missing package file: %s\n' "$required" >&2; exit 1; }
done
(
  cd "$PACKAGE_ROOT"
  shasum -a 256 -c SHA256SUMS >/dev/null
)

python3 - "$PACKAGE_ROOT" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
idl = json.loads((root / "idl/zkp2p_solana.json").read_text(encoding="utf-8"))
required = {"package", "version", "program_id", "program_sha256", "idl_sha256", "rust", "anchor", "agave"}
if set(manifest) != required:
    raise SystemExit("manifest fields are not exact")
if manifest["package"] != "zkp2p-solana" or manifest["program_id"] != idl["address"]:
    raise SystemExit("manifest identity does not match the IDL")
for relative, key in (("program/zkp2p_solana.so", "program_sha256"), ("idl/zkp2p_solana.json", "idl_sha256")):
    actual = hashlib.sha256((root / relative).read_bytes()).hexdigest()
    if actual != manifest[key]:
        raise SystemExit(f"manifest digest mismatch for {relative}")
PY

printf 'release package verified: %s\n' "$ARCHIVE_PATH"
