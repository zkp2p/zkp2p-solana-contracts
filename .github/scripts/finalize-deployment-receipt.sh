#!/usr/bin/env bash

preflight_deployment_receipt_destination() {
  local destination_receipt="$1"
  local destination_directory

  [[ ! -e "$destination_receipt" ]] || {
    echo "deployment receipt already exists" >&2
    return 1
  }
  destination_directory="$(dirname "$destination_receipt")"
  [[ -d "$destination_directory" && -w "$destination_directory" ]] || {
    echo "deployment receipt directory does not exist or is not writable" >&2
    return 1
  }
}

finalize_deployment_receipt() {
  local source_receipt="$1"
  local destination_receipt="$2"
  local expected_fingerprint="${3:-}"
  local actual_fingerprint="${4:-}"
  local destination_directory
  local publication_candidate

  if [[ -n "$expected_fingerprint" && "$actual_fingerprint" != "$expected_fingerprint" ]]; then
    echo "upgrade changed mutable protocol configuration; redeploy the recorded rollback artifact" >&2
    return 1
  fi
  [[ -f "$source_receipt" ]] || {
    echo "verified deployment receipt is missing" >&2
    return 1
  }
  preflight_deployment_receipt_destination "$destination_receipt" || return 1
  destination_directory="$(dirname "$destination_receipt")"
  publication_candidate="$(mktemp "$destination_directory/.zkp2p-receipt.XXXXXX")"
  if ! cp "$source_receipt" "$publication_candidate"; then
    /bin/unlink "$publication_candidate"
    return 1
  fi
  chmod 644 "$publication_candidate"
  if ! python3 - "$publication_candidate" "$destination_receipt" <<'PY'
import os
import sys

os.link(sys.argv[1], sys.argv[2])
PY
  then
    /bin/unlink "$publication_candidate"
    return 1
  fi
  /bin/unlink "$publication_candidate"
}
