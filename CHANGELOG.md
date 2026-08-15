# Changelog

All notable changes are recorded here. Optimization entries must include before/after state-machine and SVM measurements.

## Unreleased

### Fuzz and invariant coverage

- Add 512-case, stateful operation-sequence properties adapted from the latest Foundry StakeVault and escrow invariants.
- Exercise two stake owners and eight independent lock slots per owner across lock, increase, resize, resolve, and claim
  allocations, checking per-owner and global liability conservation after every operation.
- Exercise eight concurrent escrow locks across signal, cancel, and partial-settlement outcomes, checking live principal,
  active-lock counts, and cumulative released-value conservation after every operation.

### SVM optimization 002: prehashed EIP-712 constants

- Replace six runtime Keccak operations over immutable EIP-712 names and schemas with audited byte constants.
- Assert every constant against its canonical UTF-8 source in unit tests, while retaining the existing digest-domain and
  real-signature SVM tests.
- Reduce the SBF artifact from 1,681,128 to 1,680,296 bytes (832 bytes, 0.049%). On the deterministic LiteSVM fixture,
  threshold fulfillment falls from 126,324 to 125,844 CU (480 CU, 0.380%) and dispute submission falls from 64,526 to
  64,059 CU (467 CU, 0.724%). Repeated runs are identical.
- Detailed evidence is in
  [`docs/optimizations/002-prehashed-eip712-constants.md`](docs/optimizations/002-prehashed-eip712-constants.md).

### SVM optimization 001: fixed-width attestation hashing

- Hash payment and dispute payloads directly from their fixed-width fields instead of allocating and serializing a
  temporary `Vec<u8>`.
- Preserve the canonical Borsh byte stream exactly; differential unit tests compare both implementations byte-for-byte at
  their hash boundary.
- Reduce the SBF artifact from 1,684,696 to 1,681,128 bytes (3,568 bytes, 0.212%). On a deterministic LiteSVM fixture,
  threshold fulfillment falls from 127,304 to 126,324 CU (980 CU, 0.770%) and dispute submission falls from 65,271 to
  64,526 CU (745 CU, 1.141%). Repeated runs produce identical measurements.
- Keep the full 60/60 instruction parity suite and executable source-line coverage gate unchanged. Detailed evidence is in
  [`docs/optimizations/001-fixed-width-attestation-hashing.md`](docs/optimizations/001-fixed-width-attestation-hashing.md).

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
- Use a full-width nonzero deployment ID derived on-chain from the program ID and newest authenticated SlotHashes entry,
  plus a config-PDA-derived 20-byte verifier address. Callers cannot choose the signature domain; attestation services must
  read the initialized cluster-specific domain and use Borsh rather than ABI for native signed payload commitments.
- Replace ECDSA deposit-gating signatures with Solana-native Ed25519 precompile verification.
- Fail closed on a missing/malformed delegated rate; the EVM-only reverting-manager fallback is not reproduced.
- Restrict canonical custody to the legacy SPL Token program and reject Token-2022 mints whose extension semantics can
  invalidate exact escrow and stake accounting.
- Cap witness sets at two and require v0 address compression for two-witness settlement transactions.
- Close a fully withdrawn, non-retained deposit and its empty token vault when no intent liability remains. Existing child
  configuration PDAs become inert after the canonical parent closes; reclaiming their rent is a separate maintenance task.
- Bind initialization to the executable's upgrade authority, authenticate target genesis state before writes, and verify
  loader owner, ProgramData link, upgrade authority, and complete topology before and after upgrades.
- Require dispute preparation/cancellation to be adjacent to their precisely bound lifecycle transitions so collateral and
  escrow state cannot be orphaned by a partially committed transaction.
