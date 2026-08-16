#!/usr/bin/env bash
set -euo pipefail

solana_private_key="${SOLANA_PRIVATE_KEY-}"
unset SOLANA_PRIVATE_KEY

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

deploy_transport="${ZKP2P_SOLANA_DEPLOY_TRANSPORT:-rpc}"
case "$deploy_transport" in
  rpc) deploy_transport_flag="--use-rpc" ;;
  quic) deploy_transport_flag="--use-quic" ;;
  tpu-client) deploy_transport_flag="--use-tpu-client" ;;
  udp) deploy_transport_flag="--use-udp" ;;
  *)
    echo "ZKP2P_SOLANA_DEPLOY_TRANSPORT must be one of: rpc, quic, tpu-client, udp" >&2
    exit 2
    ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
# shellcheck source=../.github/scripts/finalize-deployment-receipt.sh
source "$repo_root/.github/scripts/finalize-deployment-receipt.sh"
# shellcheck source=../.github/scripts/materialize-keypair.sh
source "$repo_root/.github/scripts/materialize-keypair.sh"

: "${SOLANA_RPC_URL:?SOLANA_RPC_URL is required}"
: "${ZKP2P_CLUSTER_NAME:?ZKP2P_CLUSTER_NAME is required}"
: "${ZKP2P_EXPECTED_GENESIS_HASH:?ZKP2P_EXPECTED_GENESIS_HASH is required}"
: "${ZKP2P_STAKE_MINT:?ZKP2P_STAKE_MINT is required}"
: "${ZKP2P_PROTOCOL_FEE_RECIPIENT:?ZKP2P_PROTOCOL_FEE_RECIPIENT is required}"
: "${ZKP2P_INITIAL_WITNESSES:?ZKP2P_INITIAL_WITNESSES is required}"
if [[ -n "${ZKP2P_DEPLOYMENT_RECEIPT:-}" ]]; then
  preflight_deployment_receipt_destination "$ZKP2P_DEPLOYMENT_RECEIPT"
fi

program_id="5TJD8vLWqAy4hEZLnsxuFKCDnuKXkfQQWpdnqNKYoA1x"
program_keypair="${ZKP2P_PROGRAM_KEYPAIR:-$repo_root/target/deploy/zkp2p_solana-keypair.json}"
program_build_artifact="$repo_root/target/deploy/zkp2p_solana.so"
temporary_directory=""

cleanup() {
  if [[ -n "$temporary_directory" && -d "$temporary_directory" ]]; then
    find "$temporary_directory" -type f -exec /bin/unlink {} \;
    rmdir "$temporary_directory" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if [[ "$skip_build" != "true" ]]; then
  env -u SOLANA_PRIVATE_KEY anchor build --no-idl
fi
[[ -f "$program_build_artifact" ]] || { echo "missing SBF artifact: $program_build_artifact" >&2; exit 1; }
temporary_directory="$(mktemp -d)"
program_binary="$temporary_directory/zkp2p_solana.so"
cp "$program_build_artifact" "$program_binary"
chmod 600 "$program_binary"
program_sha256="$(shasum -a 256 "$program_binary" | awk '{print $1}')"
if [[ "$skip_build" == "true" ]]; then
  : "${ZKP2P_EXPECTED_PROGRAM_SHA256:?ZKP2P_EXPECTED_PROGRAM_SHA256 is required with --skip-build}"
  [[ "$ZKP2P_EXPECTED_PROGRAM_SHA256" =~ ^[0-9a-f]{64}$ ]] || {
    echo "expected program SHA-256 must be 64 lowercase hexadecimal characters" >&2
    exit 1
  }
  [[ "$program_sha256" == "$ZKP2P_EXPECTED_PROGRAM_SHA256" ]] || {
    echo "SBF artifact SHA-256 mismatch" >&2
    exit 1
  }
fi

if [[ -n "${SOLANA_KEYPAIR_PATH:-}" ]]; then
  wallet_keypair="$SOLANA_KEYPAIR_PATH"
  [[ -f "$wallet_keypair" ]] || { echo "SOLANA_KEYPAIR_PATH does not exist" >&2; exit 1; }
  unset SOLANA_PRIVATE_KEY
else
  [[ -n "$solana_private_key" ]] || {
    echo "SOLANA_PRIVATE_KEY or SOLANA_KEYPAIR_PATH is required" >&2
    exit 1
  }
  wallet_keypair="$temporary_directory/deployer.json"
  env -u SOLANA_PRIVATE_KEY cargo build --quiet --manifest-path deployment/Cargo.toml --bin zkp2p-deployer
  deployer_binary="$repo_root/deployment/target/debug/zkp2p-deployer"
  materialize_solana_keypair "$deployer_binary" "$wallet_keypair" "$solana_private_key"
fi
solana_private_key=""

authority="$(solana-keygen pubkey "$wallet_keypair")"
cargo run --quiet --manifest-path deployment/Cargo.toml -- plan --authority "$authority" >/dev/null

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
cargo run --quiet --manifest-path deployment/Cargo.toml -- preflight \
  --rpc-url "$SOLANA_RPC_URL" \
  --keypair "$wallet_keypair" \
  --receipt "$preflight_receipt" >/dev/null
program_state="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["program_state"])' "$preflight_receipt")"
is_upgrade="false"

if [[ "$program_state" == "initialized" ]]; then
  is_upgrade="true"
  program_id_argument="$program_id"
  pre_upgrade_receipt="$temporary_directory/pre-upgrade-receipt.json"
  cargo run --quiet --manifest-path deployment/Cargo.toml -- verify \
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

deploy_buffer_arguments=()
if [[ -n "${ZKP2P_SOLANA_DEPLOY_BUFFER_KEYPAIR:-}" ]]; then
  : "${ZKP2P_EXPECTED_DEPLOY_BUFFER:?ZKP2P_EXPECTED_DEPLOY_BUFFER is required with ZKP2P_SOLANA_DEPLOY_BUFFER_KEYPAIR}"
  [[ -f "$ZKP2P_SOLANA_DEPLOY_BUFFER_KEYPAIR" ]] || {
    echo "ZKP2P_SOLANA_DEPLOY_BUFFER_KEYPAIR does not exist" >&2
    exit 1
  }
  chmod 600 "$ZKP2P_SOLANA_DEPLOY_BUFFER_KEYPAIR"
  actual_deploy_buffer="$(solana-keygen pubkey "$ZKP2P_SOLANA_DEPLOY_BUFFER_KEYPAIR")"
  [[ "$actual_deploy_buffer" == "$ZKP2P_EXPECTED_DEPLOY_BUFFER" ]] || {
    echo "deployment buffer keypair does not match ZKP2P_EXPECTED_DEPLOY_BUFFER" >&2
    exit 1
  }
  deploy_buffer_arguments=(--buffer "$ZKP2P_SOLANA_DEPLOY_BUFFER_KEYPAIR")
  echo "deploy_buffer=$actual_deploy_buffer transport=$deploy_transport"
else
  echo "deploy_buffer=ephemeral transport=$deploy_transport"
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
  "${deploy_buffer_arguments[@]}" \
  "$deploy_transport_flag" \
  "$program_binary"

if [[ -z "$temporary_directory" ]]; then
  temporary_directory="$(mktemp -d)"
fi
post_deployment_receipt="$temporary_directory/post-deployment-receipt.json"
cargo run --quiet --manifest-path deployment/Cargo.toml -- apply \
  --rpc-url "$SOLANA_RPC_URL" \
  --keypair "$wallet_keypair" \
  --receipt "$post_deployment_receipt"
python3 - "$post_deployment_receipt" "$program_sha256" <<'PY'
import json
import sys

path, program_sha256 = sys.argv[1:]
with open(path, encoding="utf-8") as receipt_file:
    receipt = json.load(receipt_file)
receipt["program_sha256"] = program_sha256
with open(path, "w", encoding="utf-8") as receipt_file:
    json.dump(receipt, receipt_file, indent=2)
    receipt_file.write("\n")
PY
[[ "$(shasum -a 256 "$program_binary" | awk '{print $1}')" == "$program_sha256" ]] || {
  echo "private SBF deployment snapshot changed during deployment" >&2
  exit 1
}

if [[ "$is_upgrade" == "true" ]]; then
  post_deployment_configuration_fingerprint="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["configuration_fingerprint"])' "$post_deployment_receipt")"
else
  pre_upgrade_configuration_fingerprint=""
  post_deployment_configuration_fingerprint=""
fi

solana program show --url "$SOLANA_RPC_URL" --keypair "$wallet_keypair" "$program_id" >/dev/null
if [[ -n "${ZKP2P_DEPLOYMENT_RECEIPT:-}" ]]; then
  finalize_deployment_receipt \
    "$post_deployment_receipt" \
    "$ZKP2P_DEPLOYMENT_RECEIPT" \
    "$pre_upgrade_configuration_fingerprint" \
    "$post_deployment_configuration_fingerprint"
  echo "deployment_receipt_published=true path=$ZKP2P_DEPLOYMENT_RECEIPT"
elif [[ "$is_upgrade" == "true" && "$post_deployment_configuration_fingerprint" != "$pre_upgrade_configuration_fingerprint" ]]; then
  echo "upgrade changed mutable protocol configuration; redeploy the recorded rollback artifact" >&2
  exit 1
fi
if [[ "$is_upgrade" == "true" ]]; then
  echo "configuration_preserved=true"
fi
echo "deployment=verified program=$program_id"
