# Changelog

All notable changes are recorded here. Optimization entries must include before/after state-machine and SVM measurements.

## Unreleased

### Parity baseline

- Port the latest OrchestratorV3, EscrowV2, StakeVault, RateManagerV1, unified payment/dispute verifier, immutable nullifier,
  whitelist/address-group, and dispute-protection behavior into one Anchor program.
- Preserve exact fee floors, oracle-spread ceilings, release caps, escrow conservation, stake/claim liabilities, snapshot
  semantics, payment bindings, dispute replay protection, and terminal lifecycle ordering.
- Add native Ed25519 gating, Ethereum-compatible secp256k1 witness recovery with low-`s` enforcement, and canonical PDA
  validation for every optional lifecycle account.
- Add real-SBF LiteSVM initialization/governance coverage plus deterministic and property state-machine tests.

### Differences from Solidity

- Consolidate the Solidity contract graph into one SVM program with separate canonical configuration PDAs.
- Use chain ID zero plus a config-PDA-derived 20-byte verifier domain, and Borsh rather than ABI encoding for signed payload
  commitments. Attestation services must generate Solana-specific digests.
- Replace ECDSA deposit-gating signatures with Solana-native Ed25519 precompile verification.
- Fail closed on a missing/malformed delegated rate; the EVM-only reverting-manager fallback is not reproduced.
- Keep an economically empty deposit as a closed tombstone when all child PDAs are not supplied for explicit rent cleanup.
  No token or lock liability remains, but physical account deletion is an explicit SVM maintenance operation.
