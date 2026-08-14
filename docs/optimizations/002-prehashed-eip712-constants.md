# Optimization 002: Prehashed EIP-712 constants

## Scope

Baseline commit: `d7490ca` (`perf: hash fixed-width attestations without allocation`).

The payment and dispute EIP-712 digest builders previously invoked Keccak at runtime for immutable domain names, the
version, and type schemas. This change embeds the six resulting 32-byte hashes and leaves only transaction-dependent
hashing on chain.

The constants cover:

- `EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)`
- version `1`
- `UnifiedPaymentVerifier`
- `PaymentAttestation(bytes32 intentHash,uint256 releaseAmount,bytes32 dataHash)`
- `ZKP2P DisputeVerifier`
- `DisputeAttestation(bytes32 intentHash,bytes32 dataHash)`

## Correctness gates

- Unit tests recompute each embedded constant from the exact canonical UTF-8 bytes and require equality.
- Existing domain-binding tests still require distinct digests for distinct verifier accounts.
- The real SBF suite signs the final payment and dispute digests with a deterministic secp256k1 key, recovers the witness,
  and executes both terminal state transitions.
- All 60 public instructions and 100% of executable checked-math/state-transition source lines remain covered.

No instruction, account layout, domain input, signature scheme, or state transition changes.

## Measurements

Measurements use Anchor 1.1.2, Agave/Solana 3.0.14, Rust 1.89, and LiteSVM 0.10.0. Each transaction was run twice with
identical results.

| Measurement | Before | After | Change |
| --- | ---: | ---: | ---: |
| SBF artifact | 1,681,128 bytes | 1,680,296 bytes | -832 bytes (-0.049%) |
| Threshold fulfillment | 126,324 CU | 125,844 CU | -480 CU (-0.380%) |
| Dispute submission | 64,526 CU | 64,059 CU | -467 CU (-0.724%) |

Reproduce with:

```sh
anchor build --no-idl
stat -c '%s' target/deploy/zkp2p_solana.so
cargo test --test parity_svm \
  threshold_payment_fulfillment_binds_nullifiers_and_resolves_dispute_claim -- --nocapture
```
