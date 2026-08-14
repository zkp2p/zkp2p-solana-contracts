//! Stateful property tests adapted from the latest Foundry fuzz and invariant corpus.

use anchor_lang::prelude::Pubkey;
use proptest::prelude::*;
use zkp2p_solana::{AmountRange, Deposit, StakePosition};

const ACTORS: usize = 2;
const LOCK_SLOTS: usize = 8;
const INITIAL_BALANCE: u64 = 1_000_000;

fn bounded_nonzero(seed: u64, inclusive_maximum: u64) -> u64 {
    seed.checked_rem(inclusive_maximum)
        .unwrap_or(0)
        .saturating_add(1)
}

fn deposit() -> Deposit {
    Deposit {
        escrow_config: Pubkey::new_unique(),
        id: 1,
        depositor: Pubkey::new_unique(),
        delegate: None,
        token_mint: Pubkey::new_unique(),
        intent_amount_range: AmountRange {
            min: 1,
            max: INITIAL_BALANCE,
        },
        accepting_intents: true,
        remaining_deposits: INITIAL_BALANCE,
        outstanding_intent_amount: 0,
        active_intents: 0,
        intent_guardian: None,
        retain_on_empty: false,
        rate_manager: None,
        bump: 1,
        vault_authority_bump: 2,
    }
}

fn stake_position() -> StakePosition {
    StakePosition {
        vault: Pubkey::new_unique(),
        owner: Pubkey::new_unique(),
        balance: INITIAL_BALANCE,
        locked: 0,
        bump: 1,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Mirrors the Solidity handler invariant across arbitrary lock/cancel/settle sequences.
    #[test]
    fn escrow_state_machine_conserves_all_live_and_released_principal(
        operations in prop::collection::vec((any::<u8>(), any::<u64>(), any::<u64>()), 1..192),
    ) {
        let mut state = deposit();
        let mut locks = [0_u64; LOCK_SLOTS];
        let mut released = 0_u64;

        for (opcode, amount_seed, release_seed) in operations {
            let slot_index = usize::from(opcode) % LOCK_SLOTS;
            let action = usize::from(
                opcode.checked_div(u8::try_from(LOCK_SLOTS).expect("slot count")).unwrap_or(0),
            ) % 3;
            let Some(slot) = locks.get_mut(slot_index) else {
                continue;
            };
            match action {
                0 if *slot == 0 && state.remaining_deposits > 0 => {
                    let amount = bounded_nonzero(amount_seed, state.remaining_deposits);
                    state.lock(amount, u16::try_from(LOCK_SLOTS).expect("slot count"))?;
                    *slot = amount;
                }
                1 if *slot > 0 => {
                    state.unlock(*slot)?;
                    *slot = 0;
                }
                2 if *slot > 0 => {
                    let release = bounded_nonzero(release_seed, *slot);
                    state.settle(*slot, release)?;
                    released = released.checked_add(release).expect("bounded release total");
                    *slot = 0;
                }
                _ => {}
            }

            let live = locks.iter().try_fold(0_u64, |sum, amount| sum.checked_add(*amount))
                .expect("bounded live total");
            let active = u16::try_from(locks.iter().filter(|amount| **amount > 0).count())
                .expect("bounded active locks");
            prop_assert_eq!(state.outstanding_intent_amount, live);
            prop_assert_eq!(state.active_intents, active);
            prop_assert_eq!(state.total_liquidity().and_then(|total| total.checked_add(released)), Some(INITIAL_BALANCE));
        }
    }

    /// Exercises independent lock slots and owners while preserving every StakeVault aggregate liability.
    #[test]
    fn multi_actor_stake_sequences_preserve_lock_and_claim_liabilities(
        operations in prop::collection::vec((any::<u8>(), any::<u64>(), any::<u64>()), 1..192),
    ) {
        let mut positions = [stake_position(), stake_position()];
        let mut locks = [[0_u64; LOCK_SLOTS]; ACTORS];
        let mut claims = [0_u64; ACTORS];

        for (opcode, amount_seed, claim_seed) in operations {
            let actor_index = usize::from(opcode) % ACTORS;
            let slot_index = usize::from(
                opcode.checked_div(u8::try_from(ACTORS).expect("actor count")).unwrap_or(0),
            ) % LOCK_SLOTS;
            let action = usize::from(
                opcode
                    .checked_div(
                        u8::try_from(ACTORS.checked_mul(LOCK_SLOTS).expect("operation radix"))
                            .expect("operation radix"),
                    )
                    .unwrap_or(0),
            ) % 4;
            let Some(position) = positions.get_mut(actor_index) else {
                continue;
            };
            let Some(actor_locks) = locks.get_mut(actor_index) else {
                continue;
            };
            let Some(slot) = actor_locks.get_mut(slot_index) else {
                continue;
            };
            match action {
                0 if *slot == 0 && position.free().unwrap_or(0) > 0 => {
                    let amount = bounded_nonzero(amount_seed, position.free().unwrap_or(0));
                    position.lock(amount)?;
                    *slot = amount;
                }
                1 if *slot > 0 && position.free().unwrap_or(0) > 0 => {
                    let increase = bounded_nonzero(amount_seed, position.free().unwrap_or(0));
                    position.lock(increase)?;
                    *slot = slot.checked_add(increase).expect("bounded lock increase");
                }
                2 if *slot > 1 => {
                    let resized = bounded_nonzero(amount_seed, *slot);
                    let unlocked = slot.checked_sub(resized).expect("resize decreases lock");
                    position.unlock(unlocked)?;
                    *slot = resized;
                }
                3 if *slot > 0 => {
                    let claim = claim_seed.checked_rem(slot.saturating_add(1)).unwrap_or(0);
                    position.resolve(*slot, claim)?;
                    let Some(actor_claims) = claims.get_mut(actor_index) else {
                        continue;
                    };
                    *actor_claims = actor_claims.checked_add(claim).expect("bounded claims");
                    *slot = 0;
                }
                _ => {}
            }

            for ((actor, actor_locks), actor_claims) in
                positions.iter().zip(locks.iter()).zip(claims.iter())
            {
                let aggregate_locks = actor_locks
                    .iter()
                    .try_fold(0_u64, |sum, amount| sum.checked_add(*amount))
                    .expect("bounded aggregate locks");
                prop_assert_eq!(actor.locked, aggregate_locks);
                prop_assert!(actor.locked <= actor.balance);
                prop_assert_eq!(actor.free(), actor.balance.checked_sub(actor.locked));
                prop_assert_eq!(actor.balance.checked_add(*actor_claims), Some(INITIAL_BALANCE));
            }

            let total_recorded = positions
                .iter()
                .map(|position| position.balance)
                .chain(claims.iter().copied())
                .try_fold(0_u64, |sum, amount| sum.checked_add(amount))
                .expect("bounded global liabilities");
            prop_assert_eq!(
                total_recorded,
                INITIAL_BALANCE.checked_mul(u64::try_from(ACTORS).expect("actor count"))
                    .expect("bounded initial liabilities")
            );
        }
    }
}
