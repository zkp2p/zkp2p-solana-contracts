# Solidity-to-Solana parity specification

Status: implementation gate  
Solidity source of truth: `zkp2p/zkp2p-contracts@fbe141161fe4138421a21e28715e540dafdfee4f`  
Source verification: complete Foundry CI and release-readiness both passed for this exact SHA on 2026-08-13.

This document defines behavioral parity. The Solana implementation must not add a transition, terminal state, fallback,
or privileged path that is absent here. An SVM-specific representation is allowed only when every observable precondition,
state change, accounting invariant, and terminal outcome remains equivalent.

## Scope

Included latest components:

- `OrchestratorV3` and the registry/configuration state it consumes.
- `EscrowV2`, including fixed, oracle-spread, and delegated rate behavior.
- `StakeVault`.
- `RateManagerV1`.
- `UnifiedPaymentVerifierV3`, `MultiAttestationVerifier`, and bidirectional payment-nullifier binding.
- `AddressGroupRegistry`, `WhitelistPolicy`, `WhitelistLifecycleHook`, and `IntentLifecycleHookV1`.
- `DisputeProtectionPolicy`, `DisputeVerifier`, and its dedicated dispute-nullifier registry.
- Required SPL-token custody, governance, relayer admission, payment-method configuration, and oracle interfaces.

Excluded:

- `Escrow`, `Orchestrator`, `OrchestratorV2`, `UnifiedPaymentVerifier`, and all other predecessor implementations.
- `ProtocolViewer` and `ProtocolViewerV2`.
- Bridge, deferred-payout, legacy pre-intent, post-intent periphery, and unused deployment lanes.
- Legacy nullifier writes. A migration importer may mark predecessor payment nullifiers as spent, but it may never expose a
  legacy write path or allow routing back to a retired verifier.
- Payment-method-specific verifier contracts. Payment methods are data configured on the unified verifier.

## SVM representation contract

The first parity implementation is one upgradeable Anchor program with separately addressed PDA state for each logical
component. This is not a trust-boundary collapse: every instruction constrains the authority, PDA seeds, owning program,
token mint, token account authority, and component relationship at the account boundary.

The representation is deliberately account-oriented:

| Solidity identity/state | Solana representation |
| --- | --- |
| Contract address | Program ID plus a component configuration PDA |
| `address` | `Pubkey`, except Ethereum witnesses remain 20-byte addresses |
| `msg.sender` | Explicit signer account |
| `block.timestamp` | Checked nonnegative `Clock.unix_timestamp` |
| `uint256` token amount | `u64`, matching SPL token amount width |
| `uint256` rate/fee at `1e18` precision | `u128` with checked wide intermediates |
| Mapping entry | Deterministically seeded PDA |
| Dynamic member/config arrays | Bounded component-owned PDAs or fixed-capacity vectors |
| ERC-20 custody/allowance | SPL Token Interface vault and authority PDAs; no persistent delegate allowance |
| Contract call atomicity | Solana transaction/instruction atomicity |
| EIP-712/secp256k1 witness proof | Exact Ethereum digest plus validated preceding secp256k1 precompile instructions |
| Reentrancy guard | Solana account write locks plus explicit callback-free instruction design |
| Pause | Component configuration flag checked at the same admission boundary |

No account supplied by a client is authoritative when its value can be derived from seeds or canonical component state.

## Aggregate invariants

These hold after every successful instruction and remain unchanged after every failed instruction.

1. Escrow token custody equals the sum of each live deposit's available and outstanding liquidity, excluding amounts
   transferred by completed settlements and explicitly swept dust.
2. For each live deposit, `available + outstanding` changes only through funding, withdrawal, settlement transfer, or dust
   close. Lock, cancel, and expiry-prune only move value between the two subtotals.
3. Each live intent has exactly one escrow lock owned by the same orchestrator configuration. Terminal transitions delete
   both records atomically.
4. A payment nullifier binds to at most one intent, and an intent binds to at most one payment nullifier. Neither binding is
   mutable or reusable.
5. `free_stake(owner) = stake_balance(owner) - locked_stake(owner)` and `locked_stake <= stake_balance`.
6. `total_accounted = total_staked + total_claimable`; the stake-vault SPL balance is never below `total_accounted` after a
   successful instruction.
7. A dispute-protection intent and its stake lock either both exist in an active state or both reach the matching terminal
   outcome in the same transaction.
8. Fees are snapshotted when an intent is signaled. Later manager configuration changes cannot change that intent's fee.
9. The lifecycle policy is snapshotted when an intent is signaled. Governance rotation affects only future intents.
10. A failed admission, verification, lifecycle callback, token transfer, or accounting check commits no partial state.

## OrchestratorV3 state machine

Logical intent states are `ABSENT`, `ACTIVE`, and terminal `CANCELLED`, `PRUNED`, or `FULFILLED`. Terminal labels are events;
terminal intent state is deleted just as in Solidity.

### Signal: `ABSENT -> ACTIVE`

Preconditions:

- Orchestrator is not paused.
- Caller has no active intent unless global multiple-intent admission is enabled or caller is an admitted relayer.
- Recipient is nonzero.
- Referral fees have nonzero recipients, are individually bounded, and their aggregate plus protocol and manager fee cannot
  make settlement arithmetic underflow.
- Escrow is admitted, the deposit exists and accepts intents, the payment method is active, and the currency has a nonzero
  effective floor.
- Proposed conversion rate is at least the effective floor.
- A configured gating service has supplied a valid unexpired signature over the complete canonical signal tuple.
- A configured deposit pre-intent policy admits the taker.
- The current lifecycle policy admits the stored canonical intent.
- Escrow has enough available liquidity, amount lies within the deposit range, active-lock cap is not exceeded after expiry
  reclamation, and the intent identifier is unused.

Effects, atomically and in this order of dependency:

- Derive a globally unique intent ID from the orchestrator configuration and monotonically increasing counter.
- Snapshot all settlement fields, payee hash, manager fee/recipient, lifecycle policy, referral fees, and post-intent route.
- Append the intent to the owner's active set and increment the counter.
- Run lifecycle admission against the stored canonical intent.
- Move `amount` from deposit available liquidity to outstanding liquidity and create the escrow lock with expiry.

### Cancel: `ACTIVE -> CANCELLED`

- Only the intent owner may cancel.
- Delete orchestrator intent/account index/fee snapshots.
- Notify the snapshotted lifecycle policy. For covered intents this changes dispute protection `PENDING -> CANCELLED` and
  unlocks the stake lock.
- Delete the escrow lock and move its complete amount from outstanding back to available.
- Any failure rolls the complete transition back.

### Expiry prune: `ACTIVE -> PRUNED`

- An escrow lock is expired only when `expiry_time < now` (strict inequality).
- Expired locks may be reclaimed when liquidity is insufficient, the lock cap is reached, or explicit pruning is requested.
- Each reclaimed lock returns its amount to available liquidity, reduces outstanding liquidity, removes the matching
  orchestrator intent, and invokes the snapshotted lifecycle cancellation path.
- The SVM implementation has no gas-exhaustion orphan mode; it must still expose safe reconciliation diagnostics, but must
  not invent a partial-prune success path.

### Proof fulfillment: `ACTIVE -> FULFILLED`

- Orchestrator is not paused.
- Configured unified verifier validates the proof and returns the same intent ID plus a nonzero release amount capped at the
  intent amount.
- Delete the orchestrator record and escrow lock.
- Reduce escrow outstanding by the complete locked amount; return any `locked - released` remainder to available liquidity;
  transfer exactly `released` to the settlement vault.
- Transfer protocol, referral, and snapshotted manager fees, each rounded down from the released amount.
- Notify the snapshotted lifecycle policy with released and net amounts. Covered intents resize collateral and transition
  `PENDING -> SETTLED`.
- Transfer the exact remainder to the recipient or execute the configured post-intent route. No residual allowance exists.

### Manual release: `ACTIVE -> FULFILLED`

- Only the current escrow depositor may invoke it.
- Release amount is the complete intent amount and no payment-nullifier binding is created.
- Fee, lifecycle settlement, and final routing are identical to proof fulfillment, with `is_manual_release = true`.

## EscrowV2 state machine

Logical deposit states are `ABSENT`, `OPEN`, `CLOSED_TO_NEW_INTENTS`, `RETAINED_EMPTY`, and deleted `CLOSED`.

- Creation: validate nonzero depositor/mint/amount/minimum; `min <= max`; amount is at least min; delegate differs from
  depositor; method/currency arrays are aligned and unique. Create an `OPEN` deposit, initialize method/currency data, and
  transfer exact funding into its PDA vault.
- Funding: anyone may add a nonzero exact token amount to a live deposit; available liquidity increases.
- Withdrawal: depositor may withdraw available liquidity. Expired locks may first be reclaimed atomically. Insufficient
  post-reclamation liquidity rejects the instruction.
- Full withdrawal: depositor receives all available liquidity, accepting-intents becomes false, and the deposit remains live
  only while outstanding locks exist or `retain_on_empty` is true.
- Lock: only the configured/admitted orchestrator may create an unused lock for an open deposit. Amount/range/liquidity/cap
  checks precede the accounting move.
- Unlock: only the orchestrator recorded on that lock may restore its complete amount to available liquidity.
- Settle: only the recorded orchestrator may remove the lock and transfer a nonzero amount no greater than the lock. Any
  remainder returns to available liquidity.
- Intent guardian: only the configured guardian may extend a live lock by a nonzero duration, and total lifetime from signal
  cannot exceed five days.
- Empty close: when outstanding is zero, available is at or below dust threshold, and retention is false, delete all deposit
  configuration and transfer dust to the configured recipient.
- Pausing rejects creation, funding, depositor/delegate configuration, and lock admission. Existing cancel/unlock/settlement
  and guardian terminal safety paths remain available where Solidity permits them.

Effective rate:

1. An unlisted/deactivated tuple returns zero.
2. A configured oracle must produce a valid, nonzero, nonfuture, nonstale quote. Failure halts the tuple at zero.
3. Apply signed basis-point spread with round-up semantics; multiplier must remain positive.
4. Escrow floor is `max(fixed_floor, spread_rate)` and must be nonzero.
5. If delegated, a zero delegated rate halts at zero; otherwise effective rate is `max(delegated_rate, escrow_floor)`.
6. The EVM-only catch-and-fallback on a reverting delegated manager has no SVM analogue inside the consolidated program;
   malformed/missing delegated state fails closed. This is an intentional security-tightening difference and must be called
   out in the changelog.

## RateManagerV1 state machine

- Manager IDs are program-scoped, monotonic, deterministic hashes.
- Create validates a nonzero manager; nonzero fee requires a recipient; immutable manager max fee is at most 5%; current fee
  is at most manager max.
- Current manager alone may rotate manager/recipient/metadata, set fee, minimum liquidity, and payment/currency rates.
- A deposit can opt into at most one manager. Opt-in requires an admitted escrow, existing manager, and total deposit
  liquidity meeting the manager's current minimum. Existing opt-ins are not retroactively ejected by later minimum changes.
- Rate zero means disabled. Fee terms returned at signal are snapshotted by the orchestrator.

## StakeVault state machine

Stake owner state is aggregate; locks are `ABSENT` or `ACTIVE`, then deleted by `UNLOCKED` or `RESOLVED`.

- Deposit stake: exact nonzero SPL balance delta becomes owner free principal and increases total staked.
- Withdraw stake: only free principal may leave; reduce accounting before transfer.
- Claim: withdraw the caller's complete nonzero claimable amount; partial claims are unsupported.
- Delegation: an owner may authorize many takers. A taker selects at most one currently authorizing third party and otherwise
  falls back to self. Revocation clears a matching selection but does not alter existing locks.
- New lock: controller only; nonzero globally unused ID/owner/amount; strictly future maturity; sufficient free stake.
- Funded lock: controller adopts only vault tokens above all existing liabilities, credits them as stake, and locks them.
- Increase: controller only, pre-maturity, nonzero increment, sufficient free stake; maturity unchanged.
- Resize: controller only, pre-maturity; new amount is nonzero and cannot increase; new maturity is strictly future; removed
  amount becomes free.
- Unlock: controller may delete a lock before or after maturity; complete principal becomes free.
- Resolve: controller may allocate no more than the lock to nonzero beneficiary claims. Allocated principal moves from total
  staked to total claimable; the remainder becomes owner free stake.
- Controller initialization is allowed once only when both aggregate liabilities are zero. Later replacement is a delayed,
  two-step handover of at least one day. Ownership cannot be renounced.

## Whitelist state machine

- Groups have one curator, optional pending curator, visibility, explicit members, and an optional resolver policy.
- Group creation IDs are deterministic and unique. Curator rotation is two-step and cancellable.
- Explicit membership is curator-managed; public groups also allow self join/leave.
- Resolver failures, malformed results, or compute exhaustion deny membership without reverting the caller.
- Each deposit policy has an enforcement flag, permanent bootstrap marker, direct taker set, and at most ten allowed groups.
- Depositor-only configuration is scoped by `(escrow, deposit_id)`. Additions are idempotent; removals are idempotent.
- One-time governance bootstrap requires existing disabled, never-bootstrapped deposits and nonempty deposit/group arrays;
  the batch is atomic.
- If enforcement is disabled, every taker is allowed. If enabled, direct membership or membership in any allowed group is
  required.

Lifecycle routing for a newly signaled intent:

1. A whitelisted taker passes without stake.
2. A non-whitelisted taker on a payment method with zero risk window passes without stake.
3. Otherwise, if dispute protection is enabled, stake-backed admission is required.
4. Otherwise, an enabled whitelist rejects; a disabled whitelist remains open.

## Dispute protection state machine

`NONE -> PENDING -> CANCELLED`  
`NONE -> PENDING -> SETTLED -> RELEASED`  
`NONE -> PENDING -> SETTLED -> DISPUTED`

- Zero risk window is a pass-through and creates no policy state.
- Admission rejects when globally paused, deposit opted out, intent already exists, escrow mint differs from stake mint, or
  selected stake owner lacks free collateral equal to the full intent amount.
- `PENDING` collateral uses a never-matures sentinel.
- Cancellation unlocks the complete lock and reaches `CANCELLED`.
- Settlement snapshots the actual release amount, computes `release_eligible_at = now + snapshotted_risk_window`, resizes the
  lock to the release amount, and reaches `SETTLED`.
- Permissionless release is allowed at `now >= release_eligible_at`; it unlocks collateral and reaches `RELEASED`.
- A dispute remains valid after eligibility until release actually executes.
- Dispute submission is allowed only from `SETTLED`: validate signed evidence, exact payment-method match, and bidirectional
  original-payment binding; consume a payment-method-scoped dispute nullifier; resolve the full collateral into an
  immediately claimable depositor award; reach `DISPUTED`.
- Manual release has no payment binding, so a later dispute cannot pass binding verification.
- Admission pause affects new covered admissions only. Cancellation, settlement, release, and dispute remain available.

## Unified verifier and nullifier state machine

- Governance manages a nonempty threshold witness set and enabled payment methods. Threshold is in `1..=witness_count`.
- Only the configured orchestrator may verify a payment.
- Attestation must bind exact intent ID, nonzero release amount, signed payload hash, enabled method, nonzero payment ID,
  nonzero off-chain amount/currency, and every canonical intent snapshot field.
- Timestamp buffer cannot exceed 48 hours. Timestamp units remain milliseconds in the signed payload.
- The configured witness threshold must be met by unique authorized Ethereum addresses. Duplicate signer credit is rejected.
- Payment nullifier is `keccak256(payment_method || payment_id)` and becomes an immutable bidirectional binding to the intent.
- Release is `min(attested_release, intent_amount)`.
- Dispute attestation digest preserves the Solidity EIP-712 schema. It must match payment method and the original payment's
  bidirectional binding. Dispute nullifier is `keccak256(payment_method || dispute_id)`.

## Test and coverage parity gate

The Solidity source SHA passed 1,458 named deterministic/fuzz/invariant test functions across the complete repository.
The SVM port does not reproduce legacy-only tests, but every latest-stack behavior above must map to at least one SVM test.

Required test layers:

- Deterministic unit tests organized by `escrow`, `orchestrator`, `rate_manager`, `stake_vault`, `verifiers`, `whitelist`,
  `disputes`, `registries`, `integration`, and `deployment`.
- Broad property tests for stake accounting, nullifier uniqueness/bidirectionality, fee conservation, escrow liquidity
  conservation, and rate rounding/halts.
- Stateful invariants with multiple actors and explicit reachability counters.
- Deployment topology tests that initialize only this latest stack and assert every authority, PDA, mint, witness,
  registry, lifecycle, fee, and pause relationship.
- Independent black-box tests authored from the Foundry corpus without access to the Rust source or this design document.

Coverage cannot be compared instruction-for-instruction across Solidity and Rust. The release gate therefore requires:

- 100% public instruction/function coverage.
- At least 99.42% line, 98.70% statement/region, and 94.74% branch coverage for program-owned production logic, matching the
  Solidity enforced floors.
- 100% patch coverage after the parity baseline.
- No fuzz or invariant coverage credit toward deterministic coverage floors.

## Parity evidence required for every optimization commit

Each optimization commit must contain exactly one optimization category and record:

1. Baseline program size, compute units for affected instructions, account bytes/rent, and test/coverage result.
2. The single representation or execution change.
3. The same measurements afterward.
4. Full state-machine differential tests and the independent black-box suite passing at the new commit.
5. A changelog entry confirming no transition, authority, rounding rule, or terminal outcome changed, or explicitly
   documenting a reviewed difference.

Candidate categories, in order only after parity is green: account layout/zero-copy reads; PDA/account fanout for parallel
execution; serialization/allocation reduction; instruction/CPI reduction; batched terminal operations; binary size and
build-profile tuning. Categories must not be combined in one commit.
