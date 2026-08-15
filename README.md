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
a nonzero deployment ID deterministically derived from the authenticated cluster genesis hash and the last 20 bytes of
`keccak256(config PDA)` as its verifying-contract value. Canonical payload commitments use Borsh instead of Solidity ABI
encoding. Intent gating uses Solana's native Ed25519 precompile and the instructions sysvar.

## Build and test

Install Rust 1.89, Anchor CLI 1.1.2, and Agave/Solana CLI 3.0.14, then run:

```sh
anchor build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The `parity_svm` integration suite loads the real SBF artifact into LiteSVM. Run `anchor build` first when the program binary
is absent or stale. The real-SBF suites execute all 60 public instructions; property tests exercise escrow, stake, dispute,
fee, and rate invariants without external services. See [docs/TESTING.md](docs/TESTING.md) for the coverage denominator and
instruction-to-suite matrix.

`scripts/build-release-package.sh` creates a checksummed archive containing only the SBF program, IDL, and public protocol
documentation. Pull requests build and verify that package; the manual publishing workflow accepts only the exact version
on canonical `main`, attests the validated archive, and publishes it as a GitHub release.

## Deliberate SVM differences

- The EVM contract graph is one program with separate config PDAs. Internal component authorization is therefore PDA-based,
  not contract registry/CPI based.
- A malformed delegated-rate account fails closed. There is no analogue of the Solidity `try/catch` fallback.
- A fully withdrawn, non-retained deposit with no outstanding intents closes its deposit and token-vault accounts, matching
  Solidity terminal state. Any already-created child configuration PDAs are inert once the canonical parent is closed and
  can be reclaimed by a future rent-maintenance instruction without preserving a legacy protocol path.
- Native Ed25519 verifies intent gating; secp256k1 remains only where compatibility with existing attestation witnesses is
  required.
- The canonical custody mint must be owned by the legacy SPL Token program. Token-2022 extensions, transfer hooks, permanent
  delegates, and transfer-fee semantics are deliberately unsupported because they can violate exact liability accounting.
- Witness sets are capped at two. Two-witness fulfillment uses a v0 transaction with an address lookup table; the real-SBF
  suite proves the canonical wire payload is 811 bytes, below Solana's 1,232-byte packet limit.
- Borsh replaces ABI encoding for signed data payload commitments, while the outer EIP-712 schemas remain stable.

See [CHANGELOG.md](CHANGELOG.md) for behavioral changes and optimization evidence.
Each optimization is isolated in its own commit and has a reproducible before/after report under
[`docs/optimizations/`](docs/optimizations/). An optimization is retained only when the state-machine suite and canonical
serialization checks remain unchanged.

## Security status

This repository is under active parity implementation and independent review. It is not yet a production deployment. The
release process requires deterministic coverage floors, fuzz/invariant tests, an independent black-box suite, an external
security pass, and a staging deployment before any production use.

## License

MIT
