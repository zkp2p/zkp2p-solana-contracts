#!/usr/bin/env bash

materialize_solana_keypair() {
  local deployer_binary="$1"
  local output="$2"
  local private_key="$3"

  [[ -n "$private_key" ]] || {
    echo "SOLANA_PRIVATE_KEY is required" >&2
    return 1
  }
  if ! SOLANA_PRIVATE_KEY="$private_key" \
    "$deployer_binary" materialize-keypair --output "$output" >/dev/null; then
    private_key=""
    return 1
  fi
  private_key=""
}
