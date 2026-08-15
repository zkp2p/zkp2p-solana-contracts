#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

temporary_directory="$(mktemp -d)"
validator_pid=""
cleanup() {
  if [[ -n "$validator_pid" ]]; then
    kill "$validator_pid" 2>/dev/null || true
    wait "$validator_pid" 2>/dev/null || true
  fi
  rm -rf "$temporary_directory"
}
trap cleanup EXIT

rpc_url="http://127.0.0.1:18999"
payer="$temporary_directory/payer.json"
wrong_authority="$temporary_directory/wrong-authority.json"
mint="$temporary_directory/mint.json"
receipt="$temporary_directory/receipt.json"

solana-keygen new --no-bip39-passphrase --silent --force --outfile "$payer"
solana-keygen new --no-bip39-passphrase --silent --force --outfile "$wrong_authority"
solana-keygen new --no-bip39-passphrase --silent --force --outfile "$mint"
solana-test-validator \
  --ledger "$temporary_directory/ledger" \
  --rpc-port 18999 \
  --faucet-port 19001 \
  --gossip-port 19002 \
  --dynamic-port-range 20000-21000 \
  --reset \
  --quiet >"$temporary_directory/validator.log" 2>&1 &
validator_pid="$!"

for _ in $(seq 1 60); do
  if solana cluster-version --url "$rpc_url" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
solana cluster-version --url "$rpc_url" >/dev/null

authority="$(solana-keygen pubkey "$payer")"
solana airdrop 1000 "$authority" --url "$rpc_url" >/dev/null
spl-token create-token "$mint" \
  --url "$rpc_url" \
  --fee-payer "$payer" \
  --mint-authority "$authority" \
  --decimals 6 \
  --output json-compact >/dev/null

export SOLANA_RPC_URL="$rpc_url"
export SOLANA_KEYPAIR_PATH="$payer"
export ZKP2P_CLUSTER_NAME=local
export ZKP2P_EXPECTED_GENESIS_HASH
ZKP2P_EXPECTED_GENESIS_HASH="$(solana genesis-hash --url "$rpc_url")"
export ZKP2P_STAKE_MINT
ZKP2P_STAKE_MINT="$(solana-keygen pubkey "$mint")"
export ZKP2P_PROTOCOL_FEE_RECIPIENT="$authority"
export ZKP2P_INITIAL_WITNESSES="00112233445566778899aabbccddeeff00112233"
export ZKP2P_DEPLOYMENT_RECEIPT="$receipt"

expected_genesis_hash="$ZKP2P_EXPECTED_GENESIS_HASH"
export ZKP2P_EXPECTED_GENESIS_HASH=11111111111111111111111111111111
if scripts/deploy-latest.sh --dry-run --skip-build >/dev/null 2>&1; then
  echo "wrong cluster genesis hash unexpectedly passed" >&2
  exit 1
fi
export ZKP2P_EXPECTED_GENESIS_HASH="$expected_genesis_hash"

stake_mint="$ZKP2P_STAKE_MINT"
export ZKP2P_STAKE_MINT=11111111111111111111111111111111
if scripts/deploy-latest.sh --dry-run --skip-build >/dev/null 2>&1; then
  echo "malformed stake mint unexpectedly passed" >&2
  exit 1
fi
if solana program show --url "$rpc_url" --keypair "$payer" 5TJD8vLWqAy4hEZLnsxuFKCDnuKXkfQQWpdnqNKYoA1x >/dev/null 2>&1; then
  echo "preflight failure unexpectedly wrote the program" >&2
  exit 1
fi
export ZKP2P_STAKE_MINT="$stake_mint"

scripts/deploy-latest.sh --dry-run --skip-build >/dev/null
solana program deploy \
  --url "$rpc_url" \
  --keypair "$payer" \
  --fee-payer "$payer" \
  --upgrade-authority "$payer" \
  --program-id target/deploy/zkp2p_solana-keypair.json \
  --use-rpc \
  target/deploy/zkp2p_solana.so >/dev/null

scripts/deploy-latest.sh --apply
if cargo run --quiet -p zkp2p-deployer -- verify \
  --rpc-url "$rpc_url" \
  --keypair "$wrong_authority" >/dev/null 2>&1; then
  echo "wrong upgrade authority unexpectedly verified" >&2
  exit 1
fi
cargo run --quiet -p zkp2p-deployer -- verify \
  --rpc-url "$rpc_url" \
  --keypair "$payer" >/dev/null

python3 - "$receipt" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as receipt_file:
    receipt = json.load(receipt_file)
assert receipt["status"] == "verified"
assert receipt["cluster"] == "local"
assert receipt["genesis_hash"]
assert receipt["program"] == "5TJD8vLWqAy4hEZLnsxuFKCDnuKXkfQQWpdnqNKYoA1x"
assert receipt["programdata"]
assert receipt["upgrade_authority"] == receipt["authority"]
assert len(receipt["domain_chain_id"]) == 64
assert len(receipt["domain_seed"]) == 64
assert receipt["domain_seed_slot"] >= 0
assert receipt["witness_count"] == 1
assert receipt["required_signatures"] == 1
PY

echo "local_deployment_test=passed program=5TJD8vLWqAy4hEZLnsxuFKCDnuKXkfQQWpdnqNKYoA1x"
