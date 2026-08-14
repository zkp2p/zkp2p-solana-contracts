# ZKP2P Solana Programs

Latest-only Solana implementation of the ZKP2P settlement protocol. The behavioral baseline is
[`zkp2p/zkp2p-contracts@fbe1411`](https://github.com/zkp2p/zkp2p-contracts/commit/fbe141161fe4138421a21e28715e540dafdfee4f).
Older protocol versions, migration-only adapters, mocks, and unused periphery are intentionally absent.

The program is an Anchor 1.1.2 / Solana 3.0 Rust workspace. It consolidates the latest Solidity components into one SVM
program while preserving their logical authority boundaries and atomic state transitions:

- OrchestratorV3 intent signal, owner cancel, permissionless expiry prune, proof fulfillment, and maker manual release.
- EscrowV2 exact SPL-token custody, fixed/oracle/delegated rates, deposit configuration, guardian expiry extension, and
  full or partial withdrawal.
- StakeVault custody, delegation, exact collateral locks, releases, dispute resolution, and beneficiary claims.
- RateManagerV1 manager-scoped rates, immutable fee ceilings, fee snapshots, and liquidity-gated opt-in.
- UnifiedPaymentVerifierV3, MultiAttestationVerifier, immutable bidirectional payment bindings, and dedicated dispute
  nullifiers.
- Address groups, persistent whitelist policy/lifecycle behavior, and default-on stake-backed dispute protection.
- Two-step protocol governance and bounded pause, fee, lifecycle, expiry, and escrow configuration.

The complete audited transition map and porting requirements live in [docs/PARITY.md](docs/PARITY.md). That document is
normative: implementation shortcuts may not introduce a transition, authority, rounding rule, or terminal outcome that it
does not permit.

## Architecture

All durable records are canonical program-derived accounts. Escrow deposits and stake positions use separate token vaults;
an intent is a pair of orchestrator and escrow-lock PDAs; payment and dispute replay protection are immutable PDAs. The
account model makes unrelated deposits, intents, and stake owners parallelizable by the scheduler.

Every optional lifecycle account is re-derived and cross-checked in the handler, even though Anchor already verifies account
ownership. Token movement rejects transfer-tax or otherwise non-exact balance deltas. Fees, rates, and spreads use checked
integer math with Solidity-equivalent floor/ceiling semantics.

Payment attestations retain the Solidity EIP-712 struct schemas and Ethereum `r || s || v` witnesses. The SVM domain uses
chain ID zero and the last 20 bytes of `keccak256(config PDA)` as its verifying-contract value. Canonical payload commitments
use Borsh instead of Solidity ABI encoding. Intent gating uses Solana's native Ed25519 precompile and the instructions sysvar.

## Build and test

Install Rust 1.89, Anchor CLI 1.1.2, and Agave/Solana CLI 3.0.14, then run:

```sh
anchor build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The `parity_svm` integration suite loads the real SBF artifact into LiteSVM. Run `anchor build` first when the program binary
is absent or stale. Property tests exercise escrow, stake, dispute, fee, and rate invariants without external services.

## Deliberate SVM differences

- The EVM contract graph is one program with separate config PDAs. Internal component authorization is therefore PDA-based,
  not contract registry/CPI based.
- A malformed delegated-rate account fails closed. There is no analogue of the Solidity `try/catch` fallback.
- Account rent cleanup is explicit. A fully withdrawn, non-retained deposit becomes a closed-to-intents tombstone until its
  child configuration PDAs can be supplied for cleanup; economic liquidity and intent state match Solidity immediately.
- Native Ed25519 verifies intent gating; secp256k1 remains only where compatibility with existing attestation witnesses is
  required.
- Borsh replaces ABI encoding for signed data payload commitments, while the outer EIP-712 schemas remain stable.

See [CHANGELOG.md](CHANGELOG.md) for behavioral changes and optimization evidence.

## Security status

This repository is under active parity implementation and independent review. It is not yet a production deployment. The
release process requires deterministic coverage floors, fuzz/invariant tests, an independent black-box suite, an external
security pass, and a staging deployment before any production use.

## License

MIT
