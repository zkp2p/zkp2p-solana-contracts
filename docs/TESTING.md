# Testing and coverage contract

The Solidity source revision enforces 99.42% line, 98.70% statement, 94.74% branch, and 100% function coverage. Rust host
coverage cannot observe code executing inside a separately loaded SBF artifact, so this repository uses two non-overlapping
gates instead of reporting a misleading aggregate percentage:

1. `cargo llvm-cov --lib` measures every executable source line in the pure state-transition and checked-math core.
2. LiteSVM loads `target/deploy/zkp2p_solana.so` and executes every public instruction through the SVM account decoder,
   PDA checks, CPI boundary, and token program. The checked-in IDL is the denominator.

The release floor is 100% of executable `state.rs` and `math.rs` source lines plus 60/60 successful public-instruction
paths. Negative boundary, authorization, replay, rounding, fuzz, invariant, deployment, and independent black-box suites
are additional gates; they do not substitute for the deterministic denominator.

## Public instruction matrix

| Suite | Public instructions executed successfully |
| --- | --- |
| `parity_svm::initialization` | `initialize_protocol`, `propose_protocol_authority`, `accept_protocol_authority` |
| `parity_svm::configuration` | `configure_orchestrator`, `configure_escrow`, `propose_stake_controller`, `accept_stake_controller`, `create_rate_manager`, `set_rate_manager_config`, `set_manager_fee`, `set_manager_min_liquidity`, `set_manager_rate`, `update_oracle_quote`, `create_address_group`, `configure_address_group`, `accept_group_curator`, `set_group_member`, `set_self_group_member`, `set_risk_window`, `set_dispute_admissions_paused`, `set_verifier_payment_method`, `set_verifier_witness`, `set_required_signatures` |
| `parity_svm::deposit` | `create_deposit`, `add_funds`, `remove_funds`, `withdraw_deposit`, `update_deposit`, `configure_payment_method`, `configure_currency`, `set_deposit_rate_manager`, `initialize_deposit_whitelist`, `set_whitelist_enabled`, `set_deposit_allowed_group`, `set_deposit_whitelist_member`, `set_deposit_dispute_protection` |
| `parity_svm::stake` | `initialize_stake_token_vault`, `deposit_stake`, `withdraw_stake`, `claim_stake`, `set_taker_authorization`, `select_stake_owner`, `clear_stake_owner`, `controller_lock_stake`, `controller_fund_lock`, `increase_stake_lock`, `resize_stake_lock`, `controller_unlock_stake`, `initialize_claim_balance`, `resolve_stake_lock` |
| `parity_svm::orchestrator` | `signal_intent`, `extend_intent_expiry`, `cancel_intent`, `prune_expired_intent`, `manual_release` |
| `parity_svm::dispute_lifecycle` | `prepare_dispute`, `cancel_dispute`, `release_matured_dispute`, `fulfill_intent`, `submit_dispute` |

The matrix contains 60 unique instructions, exactly matching `target/idl/zkp2p_solana.json`. A CI script must compare the
two sets rather than trusting this table by inspection.

## Commands

Build a fresh artifact before any SVM test so stale binaries cannot mask source changes:

```sh
anchor build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo llvm-cov --lib --lcov --output-path target/coverage/core.lcov
```

The test helpers construct real SPL Token accounts and submit signed Solana transactions. Threshold verifier tests derive
an Ethereum address from a deterministic test-only secp256k1 key, sign the exact EIP-712 digest, and exercise both payment
and dispute nullifier bindings. No deployment credential or production witness key is used in tests.

## Fuzz and invariant suites

`cargo test --test fuzz_invariants` runs 512 generated cases for each stateful property. Every case contains up to 191
operations and checks invariants after each reachable transition:

| Property | Generated state | Invariants |
| --- | --- | --- |
| Escrow lifecycle | Eight concurrent lock slots; lock, cancel, and partial settlement | live-slot sum equals outstanding principal; live-slot count equals active intents; available + outstanding + cumulative releases equals initial custody |
| StakeVault liabilities | Two owners, eight independent slots each; lock, increase, resize, resolve, and arbitrary claim split | each owner's slot sum equals locked stake; locked never exceeds principal; free stake is exact; owner principal + claims and global liabilities remain conserved |

These tests are adapted from the latest Foundry fuzz and `StakeVaultInvariant` handlers. They are supplemental and do not
contribute to the deterministic coverage floor.
