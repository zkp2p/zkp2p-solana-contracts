#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 [--dry-run|--apply] [--skip-build]"
}

mode="dry-run"
skip_build="false"
while (($# > 0)); do
  case "$1" in
    --dry-run) mode="dry-run" ;;
    --apply) mode="apply" ;;
    --skip-build) skip_build="true" ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
  shift
done

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

: "${SOLANA_RPC_URL:?SOLANA_RPC_URL is required}"
: "${ZKP2P_CLUSTER_NAME:?ZKP2P_CLUSTER_NAME is required}"
: "${ZKP2P_EXPECTED_GENESIS_HASH:?ZKP2P_EXPECTED_GENESIS_HASH is required}"
: "${ZKP2P_STAKE_MINT:?ZKP2P_STAKE_MINT is required}"
: "${ZKP2P_PROTOCOL_FEE_RECIPIENT:?ZKP2P_PROTOCOL_FEE_RECIPIENT is required}"
: "${ZKP2P_INITIAL_WITNESSES:?ZKP2P_INITIAL_WITNESSES is required}"

program_id="5TJD8vLWqAy4hEZLnsxuFKCDnuKXkfQQWpdnqNKYoA1x"
program_keypair="${ZKP2P_PROGRAM_KEYPAIR:-$repo_root/target/deploy/zkp2p_solana-keypair.json}"
program_binary="$repo_root/target/deploy/zkp2p_solana.so"
temporary_directory=""

cleanup() {
  if [[ -n "$temporary_directory" && -d "$temporary_directory" ]]; then
    find "$temporary_directory" -type f -exec /bin/unlink {} \;
    rmdir "$temporary_directory" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ "$skip_build" != "true" ]]; then
  anchor build --no-idl
fi
[[ -f "$program_binary" ]] || { echo "missing SBF artifact: $program_binary" >&2; exit 1; }

if [[ -n "${SOLANA_KEYPAIR_PATH:-}" ]]; then
  wallet_keypair="$SOLANA_KEYPAIR_PATH"
  [[ -f "$wallet_keypair" ]] || { echo "SOLANA_KEYPAIR_PATH does not exist" >&2; exit 1; }
else
  : "${SOLANA_PRIVATE_KEY:?SOLANA_PRIVATE_KEY or SOLANA_KEYPAIR_PATH is required}"
  temporary_directory="$(mktemp -d)"
  wallet_keypair="$temporary_directory/deployer.json"
  cargo run --quiet -p zkp2p-deployer -- materialize-keypair --output "$wallet_keypair" >/dev/null
fi

authority="$(solana-keygen pubkey "$wallet_keypair")"
cargo run --quiet -p zkp2p-deployer -- plan --authority "$authority" >/dev/null

actual_genesis_hash="$(solana genesis-hash --url "$SOLANA_RPC_URL")"
[[ "$actual_genesis_hash" == "$ZKP2P_EXPECTED_GENESIS_HASH" ]] || {
  echo "cluster genesis hash mismatch" >&2
  exit 1
}

echo "mode=$mode program=$program_id authority=$authority rpc_configured=true"
echo "artifact_bytes=$(wc -c < "$program_binary" | tr -d '[:space:]')"

if [[ -z "$temporary_directory" ]]; then
  temporary_directory="$(mktemp -d)"
fi
preflight_receipt="$temporary_directory/preflight-receipt.json"
cargo run --quiet -p zkp2p-deployer -- preflight \
  --rpc-url "$SOLANA_RPC_URL" \
  --keypair "$wallet_keypair" \
  --receipt "$preflight_receipt" >/dev/null
program_state="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["program_state"])' "$preflight_receipt")"
is_upgrade="false"

if [[ "$program_state" == "initialized" ]]; then
  is_upgrade="true"
  program_id_argument="$program_id"
  pre_upgrade_receipt="$temporary_directory/pre-upgrade-receipt.json"
  cargo run --quiet -p zkp2p-deployer -- verify \
    --rpc-url "$SOLANA_RPC_URL" \
    --keypair "$wallet_keypair" \
    --receipt "$pre_upgrade_receipt" >/dev/null
  pre_upgrade_programdata_slot="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["programdata_slot"])' "$pre_upgrade_receipt")"
  pre_upgrade_programdata="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["programdata"])' "$pre_upgrade_receipt")"
  pre_upgrade_configuration_fingerprint="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["configuration_fingerprint"])' "$pre_upgrade_receipt")"
  echo "pre_upgrade_verified=true programdata_slot=$pre_upgrade_programdata_slot"
  if [[ "$mode" == "apply" ]]; then
    : "${ZKP2P_ROLLBACK_ARTIFACT:?ZKP2P_ROLLBACK_ARTIFACT is required for upgrades}"
    rollback_metadata="$ZKP2P_ROLLBACK_ARTIFACT.metadata.json"
    [[ ! -e "$ZKP2P_ROLLBACK_ARTIFACT" ]] || {
      echo "rollback artifact already exists" >&2
      exit 1
    }
    [[ ! -e "$rollback_metadata" ]] || {
      echo "rollback metadata already exists" >&2
      exit 1
    }
    mkdir -p "$(dirname "$ZKP2P_ROLLBACK_ARTIFACT")"
    solana program dump --url "$SOLANA_RPC_URL" "$program_id" "$ZKP2P_ROLLBACK_ARTIFACT" >/dev/null
    chmod 600 "$ZKP2P_ROLLBACK_ARTIFACT"
    previous_program_sha256="$(shasum -a 256 "$ZKP2P_ROLLBACK_ARTIFACT" | awk '{print $1}')"
    python3 - "$rollback_metadata" "$program_id" "$pre_upgrade_programdata" "$pre_upgrade_programdata_slot" "$previous_program_sha256" "$actual_genesis_hash" <<'PY'
import json
import sys

path, program, programdata, slot, digest, genesis_hash = sys.argv[1:]
with open(path, "x", encoding="utf-8") as output:
    json.dump(
        {
            "program": program,
            "programdata": programdata,
            "programdata_slot": int(slot),
            "program_sha256": digest,
            "genesis_hash": genesis_hash,
        },
        output,
        indent=2,
    )
    output.write("\n")
PY
    chmod 600 "$rollback_metadata"
    echo "rollback_artifact_recorded=true previous_program_sha256=$previous_program_sha256"
  fi
elif [[ "$program_state" == "absent" ]]; then
  [[ -f "$program_keypair" ]] || { echo "missing program ID keypair for initial deployment" >&2; exit 1; }
  chmod 600 "$program_keypair"
  actual_program_id="$(solana-keygen pubkey "$program_keypair")"
  [[ "$actual_program_id" == "$program_id" ]] || {
    echo "program keypair does not match declared program ID" >&2
    exit 1
  }
  program_id_argument="$program_keypair"
elif [[ "$program_state" == "executable-uninitialized" ]]; then
  program_id_argument="$program_id"
  echo "resume_initialization=true loader_authority_verified=true"
else
  echo "unknown program preflight state" >&2
  exit 1
fi

if [[ "$mode" == "dry-run" ]]; then
  solana balance --url "$SOLANA_RPC_URL" --keypair "$wallet_keypair"
  echo "program_state=$program_state"
  echo "dry_run=passed no_cluster_writes=true"
  exit 0
fi

solana program deploy \
  --url "$SOLANA_RPC_URL" \
  --keypair "$wallet_keypair" \
  --fee-payer "$wallet_keypair" \
  --upgrade-authority "$wallet_keypair" \
  --program-id "$program_id_argument" \
  --use-rpc \
  "$program_binary"

if [[ -n "${ZKP2P_DEPLOYMENT_RECEIPT:-}" ]]; then
  post_deployment_receipt="$ZKP2P_DEPLOYMENT_RECEIPT"
else
  if [[ -z "$temporary_directory" ]]; then
    temporary_directory="$(mktemp -d)"
  fi
  post_deployment_receipt="$temporary_directory/post-deployment-receipt.json"
fi
cargo run --quiet -p zkp2p-deployer -- apply \
  --rpc-url "$SOLANA_RPC_URL" \
  --keypair "$wallet_keypair" \
  --receipt "$post_deployment_receipt"

if [[ "$is_upgrade" == "true" ]]; then
  post_deployment_configuration_fingerprint="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["configuration_fingerprint"])' "$post_deployment_receipt")"
  [[ "$post_deployment_configuration_fingerprint" == "$pre_upgrade_configuration_fingerprint" ]] || {
    echo "upgrade changed mutable protocol configuration; redeploy the recorded rollback artifact" >&2
    exit 1
  }
  echo "configuration_preserved=true"
fi

solana program show --url "$SOLANA_RPC_URL" --keypair "$wallet_keypair" "$program_id" >/dev/null
echo "deployment=verified program=$program_id"
