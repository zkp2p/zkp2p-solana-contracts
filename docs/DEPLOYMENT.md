# Latest-stack deployment

The repository deploys one program and initializes exactly eight canonical component PDAs: protocol, EscrowV2,
OrchestratorV3, StakeVault, RateManager, unified verifier, whitelist, and dispute protection. There are no legacy,
migration, mock, or periphery deployment paths.

The configured mint must be the exact cluster-approved, initialized, six-decimal legacy SPL Token mint. Preflight,
initialization, and verification reject malformed accounts and Token-2022 ownership before any program write; operators
must independently confirm the mint address and issuer policy for the target environment.

## Preflight and secret handling

Copy the public variables from `deployments/staging.env.example` into the deployment environment. Supply the
base58-encoded 64-byte `SOLANA_PRIVATE_KEY` through the secret manager; the script never prints it. It materializes the key
only inside a mode-0600 temporary file, unlinks that file on exit, and refuses to overwrite an existing file.
For local testing only, `SOLANA_KEYPAIR_PATH` may point at an ephemeral JSON keypair.
`ZKP2P_EXPECTED_GENESIS_HASH` must be the independently verified target-cluster genesis hash. Both the shell preflight
and the deployer compare it to RPC state before any write, and the public receipt records the actual hash and cluster label.
The initializer accepts no signature-domain input. It authenticates the fixed SlotHashes sysvar, selects its newest entry,
and derives a full-width domain from a versioned prefix, the program ID, and that runtime slot hash. Verification recomputes
the domain and requires the protocol, orchestrator, payment verifier, and dispute verifier to match. The public receipt
records the selected slot, hash, and derived domain for signing infrastructure.

The program-ID keypair for an initial deployment must resolve to
`5TJD8vLWqAy4hEZLnsxuFKCDnuKXkfQQWpdnqNKYoA1x`. It defaults to the ignored Anchor artifact at
`target/deploy/zkp2p_solana-keypair.json`; set `ZKP2P_PROGRAM_KEYPAIR` when restoring it from a secure build artifact. The
deploy script refuses mismatches before making an RPC write. Once the program exists, upgrades use the public program
address and only the configured upgrade authority is required.

Run the write-free cluster preflight first:

```sh
scripts/deploy-latest.sh --dry-run
```

This builds the SBF artifact, validates every public configuration bound, derives the complete topology, verifies the
program ID, checks payer connectivity/balance, and reports whether the program already exists. It performs no cluster
writes.

Apply the deployment only after the dry run passes:

```sh
ZKP2P_DEPLOYMENT_RECEIPT=deployments/staging-receipt.json \
  scripts/deploy-latest.sh --apply
```

Solana CLI preflight remains enabled. The script deploys/upgrades the program, initializes the root only when absent, then
reads and deserializes all eight accounts. It fails unless every authority, mint, fee, witness threshold, component link,
lifecycle policy, delay, and initial pause state is exact. The optional receipt contains only public addresses,
configuration, and transaction signatures.
If an earlier first deployment left an executable but uninitialized program, preflight verifies its ProgramData and upgrade
authority and safely resumes initialization; it is not misclassified as a live upgrade. Live upgrades snapshot governance
configuration before code changes and require the post-upgrade fingerprint to match.

For an upgrade, set `ZKP2P_ROLLBACK_ARTIFACT` to a new protected file path. The script first verifies the current loader
authority and complete compatible topology, refuses an existing rollback path, dumps the exact pre-upgrade executable,
records its SHA-256, and only then changes code. Roll back by redeploying that artifact with the same upgrade authority.
The supplied fee, fee recipient, expiry, intent cap, witness set/threshold, and controller delay are explicit expectations
and must match current live state before any upgrade. Update those public inputs to the governance-approved current values;
all other mutable governance state is fingerprinted before the upgrade and must remain byte-for-byte preserved afterward.

## Local end-to-end test

```sh
scripts/test-local-deployment.sh
```

The test starts an isolated Agave validator, creates a real SPL mint, performs the actual upgradeable-loader deployment,
submits initialization, independently reads the topology again, validates the public receipt, and destroys its ledger and
ephemeral payer afterward. It uses the same scripts and SBF artifact as staging.

## Explorer and post-deploy verification

After `--apply`, run `zkp2p-deployer verify` again against the exact RPC and confirm the receipt's program, ProgramData,
upgrade authority, genesis hash, runtime-derived signature domain, mint, and eight canonical PDA links. For devnet, record
the transaction and program URLs under `https://explorer.solana.com/?cluster=devnet`; the executable program address is
`5TJD8vLWqAy4hEZLnsxuFKCDnuKXkfQQWpdnqNKYoA1x`. A visible explorer account is not sufficient by itself—the receipt and
read-only verifier are the topology and authority gates.
