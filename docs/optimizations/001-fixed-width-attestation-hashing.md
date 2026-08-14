# Optimization 001: Fixed-width attestation hashing

## Scope

Baseline commit: `7336a7d` (`test: prove latest protocol parity on SVM`).

This change removes temporary heap-backed Borsh buffers at two hot verification boundaries:

- `PaymentDetails || IntentSnapshot` during threshold payment fulfillment.
- `DisputeDetails` during signed dispute submission.

Every field in these records has a fixed width. Passing the field byte slices directly to Solana's incremental Keccak
syscall produces the same byte stream without allocating a `Vec<u8>` or copying the records into it.

## Correctness gates

- Differential unit tests serialize maximum-width, non-zero fixtures with canonical Anchor/Borsh serialization and assert
  that the optimized hash is identical.
- The real SBF LiteSVM suite still exercises all 60 public IDL instructions, including signed threshold fulfillment and
  signed dispute submission.
- The instruction matrix and executable source-line coverage gate remain unchanged.
- The fixture authority is derived from the fixed seed `[1; 32]`, so PDA bumps, transaction inputs, and compute results are
  repeatable.

The change does not alter account layouts, instruction data, signatures, PDA derivation, replay protection, rounding, or a
state transition.

## Measurements

Measurements use Anchor 1.1.2, Agave/Solana 3.0.14, Rust 1.89, and LiteSVM 0.10.0. Each transaction was run twice with
identical results.

| Measurement | Before | After | Change |
| --- | ---: | ---: | ---: |
| SBF artifact | 1,684,696 bytes | 1,681,128 bytes | -3,568 bytes (-0.212%) |
| Threshold fulfillment | 127,304 CU | 126,324 CU | -980 CU (-0.770%) |
| Dispute submission | 65,271 CU | 64,526 CU | -745 CU (-1.141%) |

Reproduce the artifact and transaction measurements with:

```sh
anchor build --no-idl
stat -c '%s' target/deploy/zkp2p_solana.so
cargo test --test parity_svm \
  threshold_payment_fulfillment_binds_nullifiers_and_resolves_dispute_claim -- --nocapture
```

## Rejected alternative

Solana's native secp256k1 precompile was evaluated for Ethereum witness recovery and rejected. The precompile hashes its
message input with Keccak internally, while existing witnesses sign the already-hashed EIP-712 digest. Supplying that digest
would therefore verify a signature over `keccak256(eip712_digest)`, breaking exact witness parity. The in-program recovery
path remains intentional.
