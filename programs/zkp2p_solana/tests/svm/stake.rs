//! Real-SBF StakeVault custody, delegation, lock, resolution, and claim transitions.

use super::common::{
    address, anchor_pubkey, install_token_account, pda, send, token_amount, Fixture,
};
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_signer::Signer;
use zkp2p_solana::{
    constants::{
        CLAIM_BALANCE_SEED, STAKE_LOCK_SEED, STAKE_POSITION_SEED, STAKE_SELECTION_SEED,
        STAKE_TOKEN_VAULT_SEED, TAKER_AUTHORIZATION_SEED,
    },
    ClaimBalance, ControllerLockArgs, StakeClaim, StakePosition, StakeSelection, StakeVaultConfig,
};

fn decode<T: AccountDeserialize>(fixture: &Fixture, key: anchor_lang::prelude::Pubkey) -> T {
    let account = fixture
        .svm
        .get_account(&address(key.to_bytes()))
        .expect("account exists");
    let mut data = account.data.as_slice();
    T::try_deserialize(&mut data).expect("account decodes")
}

#[test]
fn exact_stake_liabilities_survive_all_lock_and_claim_transitions() {
    let mut fixture = Fixture::new();
    let owner = anchor_pubkey(fixture.authority.pubkey());
    let owner_token = anchor_pubkey(Address::new_unique());
    install_token_account(&mut fixture.svm, owner_token, fixture.mint, owner, 1_000);
    let vault_token = pda(&[STAKE_TOKEN_VAULT_SEED, fixture.stake_vault.as_ref()]);
    let initialize_vault = fixture.program_instruction(
        zkp2p_solana::accounts::InitializeStakeTokenVault {
            payer: owner,
            vault: fixture.stake_vault,
            mint: fixture.mint,
            vault_token,
            token_program: anchor_spl::token::ID,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::InitializeStakeTokenVault {},
    );
    fixture
        .send(&[initialize_vault])
        .expect("initialize stake token vault");

    let owner_position = pda(&[
        STAKE_POSITION_SEED,
        fixture.stake_vault.as_ref(),
        owner.as_ref(),
    ]);
    let deposit = fixture.program_instruction(
        zkp2p_solana::accounts::DepositStake {
            owner,
            vault: fixture.stake_vault,
            position: owner_position,
            mint: fixture.mint,
            owner_token,
            vault_token,
            token_program: anchor_spl::token::ID,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::DepositStake { amount: 600 },
    );
    let withdraw = fixture.program_instruction(
        zkp2p_solana::accounts::WithdrawStake {
            owner,
            vault: fixture.stake_vault,
            position: owner_position,
            mint: fixture.mint,
            vault_token,
            owner_token,
            token_program: anchor_spl::token::ID,
        },
        zkp2p_solana::instruction::WithdrawStake { amount: 100 },
    );
    fixture
        .send(&[deposit, withdraw])
        .expect("deposit and withdraw stake");
    assert_eq!(token_amount(&fixture.svm, vault_token), 500);

    let taker = Keypair::new();
    fixture.fund(&taker);
    let taker_key = anchor_pubkey(taker.pubkey());
    let authorization = pda(&[TAKER_AUTHORIZATION_SEED, owner.as_ref(), taker_key.as_ref()]);
    let selection = pda(&[STAKE_SELECTION_SEED, taker_key.as_ref()]);
    let authorize = fixture.program_instruction(
        zkp2p_solana::accounts::SetTakerAuthorization {
            stake_owner: owner,
            taker: taker_key,
            authorization,
            selection,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::SetTakerAuthorization { authorized: true },
    );
    fixture.send(&[authorize]).expect("authorize taker");
    let select = fixture.program_instruction(
        zkp2p_solana::accounts::SelectStakeOwner {
            taker: taker_key,
            stake_owner: owner,
            authorization,
            selection,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::SelectStakeOwner {},
    );
    send(&mut fixture.svm, &taker, &[], &[select]).expect("select stake owner");
    let selected: StakeSelection = decode(&fixture, selection);
    assert_eq!(selected.selected_owner, Some(owner));
    let clear = fixture.program_instruction(
        zkp2p_solana::accounts::ClearStakeOwner {
            taker: taker_key,
            selection,
        },
        zkp2p_solana::instruction::ClearStakeOwner {},
    );
    send(&mut fixture.svm, &taker, &[], &[clear]).expect("clear stake owner");

    let controller = Keypair::new();
    fixture.fund(&controller);
    let controller_key = anchor_pubkey(controller.pubkey());
    let propose = fixture.program_instruction(
        zkp2p_solana::accounts::ProposeStakeController {
            authority: owner,
            protocol: fixture.protocol,
            vault: fixture.stake_vault,
        },
        zkp2p_solana::instruction::ProposeStakeController {
            pending: Some(controller_key),
        },
    );
    fixture.send(&[propose]).expect("propose controller");
    let mut clock = fixture.svm.get_sysvar::<anchor_lang::prelude::Clock>();
    clock.unix_timestamp = clock
        .unix_timestamp
        .checked_add(86_401)
        .expect("fixture clock fits");
    fixture.svm.set_sysvar(&clock);
    let accept = fixture.program_instruction(
        zkp2p_solana::accounts::AcceptStakeController {
            pending_controller: controller_key,
            vault: fixture.stake_vault,
        },
        zkp2p_solana::instruction::AcceptStakeController {},
    );
    send(&mut fixture.svm, &controller, &[], &[accept]).expect("accept controller");

    let now = fixture
        .svm
        .get_sysvar::<anchor_lang::prelude::Clock>()
        .unix_timestamp;
    let first_lock_id = [11_u8; 32];
    let first_lock = pda(&[
        STAKE_LOCK_SEED,
        fixture.stake_vault.as_ref(),
        &first_lock_id,
    ]);
    let create_lock = fixture.program_instruction(
        zkp2p_solana::accounts::ControllerLockStake {
            controller: controller_key,
            vault: fixture.stake_vault,
            stake_owner: owner,
            position: owner_position,
            stake_lock: first_lock,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::ControllerLockStake {
            args: ControllerLockArgs {
                lock_id: first_lock_id,
                amount: 200,
                matures_at: now.checked_add(10_000).expect("maturity fits"),
            },
        },
    );
    send(&mut fixture.svm, &controller, &[], &[create_lock]).expect("lock stake");
    let manage = zkp2p_solana::accounts::ManageStakeLock {
        controller: controller_key,
        vault: fixture.stake_vault,
        position: owner_position,
        stake_lock: first_lock,
    };
    let increase = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: manage.to_account_metas(None),
        data: zkp2p_solana::instruction::IncreaseStakeLock {
            lock_id: first_lock_id,
            additional_amount: 50,
        }
        .data(),
    };
    let resize = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: manage.to_account_metas(None),
        data: zkp2p_solana::instruction::ResizeStakeLock {
            lock_id: first_lock_id,
            new_amount: 150,
            new_matures_at: now.checked_add(5_000).expect("maturity fits"),
        }
        .data(),
    };
    let unlock = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: manage.to_account_metas(None),
        data: zkp2p_solana::instruction::ControllerUnlockStake {
            lock_id: first_lock_id,
        }
        .data(),
    };
    send(
        &mut fixture.svm,
        &controller,
        &[],
        &[increase, resize, unlock],
    )
    .expect("increase resize and unlock");

    let funded_owner = anchor_lang::prelude::Pubkey::new_unique();
    let funded_position = pda(&[
        STAKE_POSITION_SEED,
        fixture.stake_vault.as_ref(),
        funded_owner.as_ref(),
    ]);
    let controller_token = anchor_pubkey(Address::new_unique());
    install_token_account(
        &mut fixture.svm,
        controller_token,
        fixture.mint,
        controller_key,
        200,
    );
    let second_lock_id = [12_u8; 32];
    let second_lock = pda(&[
        STAKE_LOCK_SEED,
        fixture.stake_vault.as_ref(),
        &second_lock_id,
    ]);
    let fund_lock = fixture.program_instruction(
        zkp2p_solana::accounts::ControllerFundLock {
            controller: controller_key,
            vault: fixture.stake_vault,
            stake_owner: funded_owner,
            position: funded_position,
            stake_lock: second_lock,
            mint: fixture.mint,
            controller_token,
            vault_token,
            token_program: anchor_spl::token::ID,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::ControllerFundLock {
            args: ControllerLockArgs {
                lock_id: second_lock_id,
                amount: 200,
                matures_at: now.checked_add(10_000).expect("maturity fits"),
            },
        },
    );
    send(&mut fixture.svm, &controller, &[], &[fund_lock]).expect("fund locked stake");

    let beneficiary = Keypair::new();
    fixture.fund(&beneficiary);
    let beneficiary_key = anchor_pubkey(beneficiary.pubkey());
    let beneficiary_token = anchor_pubkey(Address::new_unique());
    install_token_account(
        &mut fixture.svm,
        beneficiary_token,
        fixture.mint,
        beneficiary_key,
        0,
    );
    let claim = pda(&[
        CLAIM_BALANCE_SEED,
        fixture.stake_vault.as_ref(),
        beneficiary_key.as_ref(),
    ]);
    let initialize_claim = fixture.program_instruction(
        zkp2p_solana::accounts::InitializeClaimBalance {
            payer: controller_key,
            vault: fixture.stake_vault,
            beneficiary: beneficiary_key,
            claim,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::InitializeClaimBalance {},
    );
    send(&mut fixture.svm, &controller, &[], &[initialize_claim])
        .expect("initialize claim balance");

    let mut resolve_accounts = zkp2p_solana::accounts::ResolveStakeLock {
        controller: controller_key,
        vault: fixture.stake_vault,
        position: funded_position,
        stake_lock: second_lock,
    }
    .to_account_metas(None);
    resolve_accounts.push(AccountMeta::new(address(claim.to_bytes()), false));
    let resolve = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: resolve_accounts,
        data: zkp2p_solana::instruction::ResolveStakeLock {
            lock_id: second_lock_id,
            claims: vec![StakeClaim {
                beneficiary: beneficiary_key,
                amount: 150,
            }],
        }
        .data(),
    };
    send(&mut fixture.svm, &controller, &[], &[resolve]).expect("resolve lock to claim");

    let claim_state: ClaimBalance = decode(&fixture, claim);
    assert_eq!(claim_state.amount, 150);
    let funded_state: StakePosition = decode(&fixture, funded_position);
    assert_eq!(funded_state.balance, 50);
    assert_eq!(funded_state.locked, 0);
    let vault_state: StakeVaultConfig = decode(&fixture, fixture.stake_vault);
    assert_eq!(vault_state.total_staked, 550);
    assert_eq!(vault_state.total_claimable, 150);

    let claim_instruction = fixture.program_instruction(
        zkp2p_solana::accounts::ClaimStake {
            beneficiary: beneficiary_key,
            vault: fixture.stake_vault,
            claim,
            mint: fixture.mint,
            vault_token,
            beneficiary_token,
            token_program: anchor_spl::token::ID,
        },
        zkp2p_solana::instruction::ClaimStake {},
    );
    send(&mut fixture.svm, &beneficiary, &[], &[claim_instruction]).expect("claim resolved stake");
    assert_eq!(token_amount(&fixture.svm, beneficiary_token), 150);
    assert_eq!(token_amount(&fixture.svm, vault_token), 550);
}
