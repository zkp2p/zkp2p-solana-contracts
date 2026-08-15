//! Black-box SVM parity tests derived independently from the public Solidity
//! Foundry suite. This file intentionally uses only the public Anchor client
//! API, IDL-described account layouts, and the built SBF artifact.

#![allow(clippy::arithmetic_side_effects)]

use anchor_lang::{
    solana_program::{program_option::COption, program_pack::Pack},
    AccountDeserialize, InstructionData, ToAccountMetas,
};
use anchor_spl::token::spl_token::{
    self,
    state::{Account as SplAccount, AccountState, Mint},
};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_loader_v3_interface::state::UpgradeableLoaderState;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::{path::PathBuf, str::FromStr};
use zkp2p_solana::{
    accounts as z_accounts, instruction as z_instruction,
    state::{
        AmountRange, ClaimBalance, CreateDepositArgs, Deposit, EscrowConfig,
        InitializeProtocolArgs, StakeLock, StakePosition, StakeVaultConfig,
    },
    ControllerLockArgs, StakeClaim,
};

const USER_STARTING_TOKENS: u64 = 1_000_000;
const CONTROLLER_DELAY: i64 = 86_400;
const IDL_PROGRAM_ID: &str = "5TJD8vLWqAy4hEZLnsxuFKCDnuKXkfQQWpdnqNKYoA1x";
const STAKE_VAULT_SEED: &[u8] = b"stake-vault-config";
const STAKE_TOKEN_VAULT_SEED: &[u8] = b"stake-token-vault";
const STAKE_POSITION_SEED: &[u8] = b"stake-position";
const STAKE_LOCK_SEED: &[u8] = b"stake-lock";
const CLAIM_BALANCE_SEED: &[u8] = b"claim-balance";
const DEPOSIT_SEED: &[u8] = b"deposit";
const DEPOSIT_VAULT_SEED: &[u8] = b"deposit-vault";
const PAYMENT_METHOD_SEED: &[u8] = b"payment-method";
const DEPOSIT_CURRENCY_SEED: &[u8] = b"deposit-currency";

struct Harness {
    svm: LiteSVM,
    authority: Keypair,
    controller: Keypair,
    owner: Keypair,
    mint: anchor_lang::prelude::Pubkey,
    owner_token: anchor_lang::prelude::Pubkey,
    protocol: anchor_lang::prelude::Pubkey,
    escrow: anchor_lang::prelude::Pubkey,
    vault: anchor_lang::prelude::Pubkey,
    vault_token: anchor_lang::prelude::Pubkey,
}

impl Harness {
    fn new() -> Self {
        let authority = Keypair::new();
        let controller = Keypair::new();
        let owner = Keypair::new();
        let mint_authority = Keypair::new();
        let mint = Keypair::new().pubkey();
        let owner_token = Keypair::new().pubkey();
        let authority_token = Keypair::new().pubkey();

        let mut svm = LiteSVM::new().with_default_programs();
        let program_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/deploy/zkp2p_solana.so");
        svm.add_program_from_file(program_id(), program_path)
            .expect("load public SBF artifact");
        let program_data = authorize_program_upgrade(&mut svm, authority.pubkey());

        for signer in [&authority, &controller, &owner, &mint_authority] {
            svm.airdrop(&signer.pubkey(), 10_000_000_000)
                .expect("fund test signer");
        }
        install_mint(
            &mut svm,
            mint,
            mint_authority.pubkey(),
            USER_STARTING_TOKENS,
        );
        install_token_account(
            &mut svm,
            owner_token,
            mint,
            owner.pubkey(),
            USER_STARTING_TOKENS,
        );
        install_token_account(&mut svm, authority_token, mint, authority.pubkey(), 0);

        let program_id = program_id();
        let protocol = pda(&[b"protocol"], &program_id);
        let escrow = pda(&[b"escrow-config"], &program_id);
        let verifier = pda(&[b"verifier-config"], &program_id);
        let orchestrator = pda(&[b"orchestrator-config"], &program_id);
        let vault = pda(&[STAKE_VAULT_SEED], &program_id);
        let rate_manager = pda(&[b"rate-manager-config"], &program_id);
        let whitelist = pda(&[b"whitelist-config"], &program_id);
        let dispute = pda(&[b"dispute-config"], &program_id);
        let vault_token = pda(&[STAKE_TOKEN_VAULT_SEED, vault.as_ref()], &program_id);

        let initialize = instruction(
            z_accounts::InitializeProtocol {
                authority: authority.pubkey(),
                program: program_id,
                program_data,
                slot_hashes: solana_program::sysvar::slot_hashes::ID,
                protocol,
                stake_mint: mint,
                escrow_config: escrow,
                verifier_config: verifier,
                orchestrator_config: orchestrator,
                stake_vault_config: vault,
                rate_manager_config: rate_manager,
                whitelist_config: whitelist,
                dispute_config: dispute,
                system_program: solana_sdk_ids::system_program::ID,
            },
            z_instruction::InitializeProtocol {
                args: InitializeProtocolArgs {
                    protocol_fee: 10_000_000_000_000_000,
                    protocol_fee_recipient: authority.pubkey(),
                    intent_expiration_period: 3_600,
                    max_intents_per_deposit: 8,
                    controller_change_delay: CONTROLLER_DELAY,
                    initial_witnesses: vec![[7_u8; 20]],
                    required_signatures: 1,
                },
            },
        );
        send_ok(&mut svm, &authority, vec![initialize], &[]);

        let initialize_vault = instruction(
            z_accounts::InitializeStakeTokenVault {
                payer: authority.pubkey(),
                vault,
                mint,
                vault_token,
                token_program: spl_token::ID,
                system_program: solana_sdk_ids::system_program::ID,
            },
            z_instruction::InitializeStakeTokenVault,
        );
        send_ok(&mut svm, &authority, vec![initialize_vault], &[]);

        let propose = instruction(
            z_accounts::ProposeStakeController {
                authority: authority.pubkey(),
                protocol,
                vault,
            },
            z_instruction::ProposeStakeController {
                pending: Some(controller.pubkey()),
            },
        );
        send_ok(&mut svm, &authority, vec![propose], &[]);

        let mut clock: anchor_lang::prelude::Clock = svm.get_sysvar();
        clock.unix_timestamp += CONTROLLER_DELAY + 1;
        svm.set_sysvar(&clock);
        let accept = instruction(
            z_accounts::AcceptStakeController {
                pending_controller: controller.pubkey(),
                vault,
            },
            z_instruction::AcceptStakeController,
        );
        send_ok(&mut svm, &controller, vec![accept], &[]);

        Self {
            svm,
            authority,
            controller,
            owner,
            mint,
            owner_token,
            protocol,
            escrow,
            vault,
            vault_token,
        }
    }

    fn position(&self) -> anchor_lang::prelude::Pubkey {
        pda(
            &[
                STAKE_POSITION_SEED,
                self.vault.as_ref(),
                self.owner.pubkey().as_ref(),
            ],
            &program_id(),
        )
    }

    fn create_deposit(&mut self, amount: u64) -> Result<DepositAddresses, String> {
        let escrow: EscrowConfig = read_anchor(&self.svm, self.escrow);
        let deposit_id = escrow.next_deposit_id.to_le_bytes();
        let deposit = pda(
            &[DEPOSIT_SEED, self.escrow.as_ref(), &deposit_id],
            &program_id(),
        );
        let payment_method = [61_u8; 32];
        let currency = [62_u8; 32];
        let payment_method_account = pda(
            &[PAYMENT_METHOD_SEED, deposit.as_ref(), &payment_method],
            &program_id(),
        );
        let currency_account = pda(
            &[
                DEPOSIT_CURRENCY_SEED,
                deposit.as_ref(),
                &payment_method,
                &currency,
            ],
            &program_id(),
        );
        let vault = pda(&[DEPOSIT_VAULT_SEED, deposit.as_ref()], &program_id());
        let ix = instruction(
            z_accounts::CreateDeposit {
                depositor: self.owner.pubkey(),
                escrow_config: self.escrow,
                deposit,
                payment_method: payment_method_account,
                currency: currency_account,
                mint: self.mint,
                depositor_token: self.owner_token,
                deposit_vault: vault,
                token_program: spl_token::ID,
                system_program: solana_sdk_ids::system_program::ID,
            },
            z_instruction::CreateDeposit {
                args: CreateDepositArgs {
                    amount,
                    intent_amount_range: AmountRange {
                        min: 1,
                        max: amount,
                    },
                    delegate: None,
                    intent_guardian: None,
                    retain_on_empty: false,
                    payment_method,
                    payee_details: [63_u8; 32],
                    gating_service: None,
                    currency,
                    fixed_min_rate: 1_000_000_000_000_000_000,
                    oracle_quote: None,
                    spread_bps: 0,
                    max_staleness: 0,
                },
            },
        );
        send(&mut self.svm, &self.owner, vec![ix], &[])?;
        Ok(DepositAddresses { deposit, vault })
    }

    fn deposit_stake(&mut self, amount: u64) -> Result<(), String> {
        let ix = instruction(
            z_accounts::DepositStake {
                owner: self.owner.pubkey(),
                vault: self.vault,
                position: self.position(),
                mint: self.mint,
                owner_token: self.owner_token,
                vault_token: self.vault_token,
                token_program: spl_token::ID,
                system_program: solana_sdk_ids::system_program::ID,
            },
            z_instruction::DepositStake { amount },
        );
        send(&mut self.svm, &self.owner, vec![ix], &[])
    }

    fn withdraw_stake(&mut self, amount: u64) -> Result<(), String> {
        let ix = instruction(
            z_accounts::WithdrawStake {
                owner: self.owner.pubkey(),
                vault: self.vault,
                position: self.position(),
                mint: self.mint,
                vault_token: self.vault_token,
                owner_token: self.owner_token,
                token_program: spl_token::ID,
            },
            z_instruction::WithdrawStake { amount },
        );
        send(&mut self.svm, &self.owner, vec![ix], &[])
    }

    fn lock(&mut self, lock_id: [u8; 32], amount: u64, matures_at: i64) -> Result<(), String> {
        let lock = self.lock_address(lock_id);
        let ix = instruction(
            z_accounts::ControllerLockStake {
                controller: self.controller.pubkey(),
                vault: self.vault,
                stake_owner: self.owner.pubkey(),
                position: self.position(),
                stake_lock: lock,
                system_program: solana_sdk_ids::system_program::ID,
            },
            z_instruction::ControllerLockStake {
                args: ControllerLockArgs {
                    lock_id,
                    amount,
                    matures_at,
                },
            },
        );
        send(&mut self.svm, &self.controller, vec![ix], &[])
    }

    fn increase_lock(&mut self, lock_id: [u8; 32], amount: u64) -> Result<(), String> {
        let ix = instruction(
            z_accounts::ManageStakeLock {
                controller: self.controller.pubkey(),
                vault: self.vault,
                position: self.position(),
                stake_lock: self.lock_address(lock_id),
            },
            z_instruction::IncreaseStakeLock {
                lock_id,
                additional_amount: amount,
            },
        );
        send(&mut self.svm, &self.controller, vec![ix], &[])
    }

    fn resize_lock(
        &mut self,
        lock_id: [u8; 32],
        amount: u64,
        matures_at: i64,
    ) -> Result<(), String> {
        let ix = instruction(
            z_accounts::ManageStakeLock {
                controller: self.controller.pubkey(),
                vault: self.vault,
                position: self.position(),
                stake_lock: self.lock_address(lock_id),
            },
            z_instruction::ResizeStakeLock {
                lock_id,
                new_amount: amount,
                new_matures_at: matures_at,
            },
        );
        send(&mut self.svm, &self.controller, vec![ix], &[])
    }

    fn unlock(&mut self, lock_id: [u8; 32]) -> Result<(), String> {
        let ix = instruction(
            z_accounts::ManageStakeLock {
                controller: self.controller.pubkey(),
                vault: self.vault,
                position: self.position(),
                stake_lock: self.lock_address(lock_id),
            },
            z_instruction::ControllerUnlockStake { lock_id },
        );
        send(&mut self.svm, &self.controller, vec![ix], &[])
    }

    fn lock_address(&self, lock_id: [u8; 32]) -> anchor_lang::prelude::Pubkey {
        pda(
            &[STAKE_LOCK_SEED, self.vault.as_ref(), &lock_id],
            &program_id(),
        )
    }

    fn vault_state(&self) -> StakeVaultConfig {
        read_anchor(&self.svm, self.vault)
    }

    fn position_state(&self) -> StakePosition {
        read_anchor(&self.svm, self.position())
    }

    fn assert_custody_invariant(&self) {
        let vault = self.vault_state();
        let custody = token_amount(&self.svm, self.vault_token);
        assert_eq!(
            custody,
            vault.total_staked + vault.total_claimable,
            "custody must cover all accounted principal and claims"
        );
    }
}

#[derive(Clone, Copy)]
struct DepositAddresses {
    deposit: anchor_lang::prelude::Pubkey,
    vault: anchor_lang::prelude::Pubkey,
}

#[test]
fn protocol_singleton_reinitialization_fails_without_mutating_state() {
    let mut h = Harness::new();
    let root_before = account_bytes(&h.svm, h.protocol);
    let vault_before = account_bytes(&h.svm, h.vault);
    let program_id = program_id();
    let ix = instruction(
        z_accounts::InitializeProtocol {
            authority: h.authority.pubkey(),
            program: program_id,
            program_data: program_data_address(&program_id),
            slot_hashes: solana_program::sysvar::slot_hashes::ID,
            protocol: h.protocol,
            stake_mint: h.mint,
            escrow_config: h.escrow,
            verifier_config: pda(&[b"verifier-config"], &program_id),
            orchestrator_config: pda(&[b"orchestrator-config"], &program_id),
            stake_vault_config: h.vault,
            rate_manager_config: pda(&[b"rate-manager-config"], &program_id),
            whitelist_config: pda(&[b"whitelist-config"], &program_id),
            dispute_config: pda(&[b"dispute-config"], &program_id),
            system_program: solana_sdk_ids::system_program::ID,
        },
        z_instruction::InitializeProtocol {
            args: InitializeProtocolArgs {
                protocol_fee: 0,
                protocol_fee_recipient: h.authority.pubkey(),
                intent_expiration_period: 1,
                max_intents_per_deposit: 1,
                controller_change_delay: CONTROLLER_DELAY,
                initial_witnesses: vec![[8_u8; 20]],
                required_signatures: 1,
            },
        },
    );

    assert_failed(send(&mut h.svm, &h.authority, vec![ix], &[]));
    assert_eq!(account_bytes(&h.svm, h.protocol), root_before);
    assert_eq!(account_bytes(&h.svm, h.vault), vault_before);
}

fn program_data_address(program_id: &anchor_lang::prelude::Pubkey) -> anchor_lang::prelude::Pubkey {
    anchor_lang::prelude::Pubkey::find_program_address(
        &[program_id.as_ref()],
        &anchor_lang::solana_program::bpf_loader_upgradeable::ID,
    )
    .0
}

fn authorize_program_upgrade(
    svm: &mut LiteSVM,
    authority: anchor_lang::prelude::Pubkey,
) -> anchor_lang::prelude::Pubkey {
    let program_data = program_data_address(&program_id());
    let mut account = svm.get_account(&program_data).expect("programdata account");
    bincode::serialize_into(
        account
            .data
            .get_mut(..UpgradeableLoaderState::size_of_programdata_metadata())
            .expect("programdata metadata"),
        &UpgradeableLoaderState::ProgramData {
            slot: 0,
            upgrade_authority_address: Some(
                authority.to_string().parse().expect("interface authority"),
            ),
        },
    )
    .expect("set test upgrade authority");
    svm.set_account(program_data, account)
        .expect("update programdata account");
    program_data
}

#[test]
fn controller_handover_cannot_be_accepted_before_delay() {
    let mut h = Harness::new();
    let pending = Keypair::new();
    h.svm
        .airdrop(&pending.pubkey(), 10_000_000_000)
        .expect("fund pending controller");
    let propose = instruction(
        z_accounts::ProposeStakeController {
            authority: h.authority.pubkey(),
            protocol: h.protocol,
            vault: h.vault,
        },
        z_instruction::ProposeStakeController {
            pending: Some(pending.pubkey()),
        },
    );
    send_ok(&mut h.svm, &h.authority, vec![propose], &[]);
    let vault_before = account_bytes(&h.svm, h.vault);
    let accept = instruction(
        z_accounts::AcceptStakeController {
            pending_controller: pending.pubkey(),
            vault: h.vault,
        },
        z_instruction::AcceptStakeController,
    );
    assert_error(
        send(&mut h.svm, &pending, vec![accept], &[]),
        "ControllerNotReady",
    );
    assert_eq!(account_bytes(&h.svm, h.vault), vault_before);
}

#[test]
fn stake_deposit_and_withdraw_update_exact_balances() {
    let mut h = Harness::new();

    h.deposit_stake(300_000).expect("deposit stake");
    let position = h.position_state();
    assert_eq!(position.balance, 300_000);
    assert_eq!(position.locked, 0);
    assert_eq!(h.vault_state().total_staked, 300_000);
    assert_eq!(token_amount(&h.svm, h.owner_token), 700_000);
    h.assert_custody_invariant();

    h.withdraw_stake(125_000).expect("withdraw free stake");
    let position = h.position_state();
    assert_eq!(position.balance, 175_000);
    assert_eq!(position.locked, 0);
    assert_eq!(h.vault_state().total_staked, 175_000);
    assert_eq!(token_amount(&h.svm, h.owner_token), 825_000);
    h.assert_custody_invariant();
}

#[test]
fn zero_deposit_and_over_withdraw_fail_without_state_changes() {
    let mut h = Harness::new();

    let vault_before = account_bytes(&h.svm, h.vault);
    let owner_before = token_amount(&h.svm, h.owner_token);
    assert_error(h.deposit_stake(0), "ZeroValue");
    assert_eq!(account_bytes(&h.svm, h.vault), vault_before);
    assert_eq!(token_amount(&h.svm, h.owner_token), owner_before);
    assert!(h.svm.get_account(&h.position()).is_none());

    h.deposit_stake(100_000).expect("seed position");
    let position_before = account_bytes(&h.svm, h.position());
    let vault_before = account_bytes(&h.svm, h.vault);
    let custody_before = token_amount(&h.svm, h.vault_token);
    assert_error(h.withdraw_stake(100_001), "InsufficientFreeStake");
    assert_eq!(account_bytes(&h.svm, h.position()), position_before);
    assert_eq!(account_bytes(&h.svm, h.vault), vault_before);
    assert_eq!(token_amount(&h.svm, h.vault_token), custody_before);
    h.assert_custody_invariant();
}

#[test]
fn locked_stake_is_isolated_and_duplicate_lock_is_atomic() {
    let mut h = Harness::new();
    h.deposit_stake(500_000).expect("deposit stake");
    let now: anchor_lang::prelude::Clock = h.svm.get_sysvar();
    let lock_id = [11_u8; 32];

    h.lock(lock_id, 350_000, now.unix_timestamp + 3_600)
        .expect("lock free stake");
    let position = h.position_state();
    assert_eq!(position.balance, 500_000);
    assert_eq!(position.locked, 350_000);
    h.assert_custody_invariant();

    let position_before = account_bytes(&h.svm, h.position());
    let vault_before = account_bytes(&h.svm, h.vault);
    assert_error(h.withdraw_stake(150_001), "InsufficientFreeStake");
    assert_eq!(account_bytes(&h.svm, h.position()), position_before);
    assert_eq!(account_bytes(&h.svm, h.vault), vault_before);

    let lock = h.lock_address(lock_id);
    let lock_before = account_bytes(&h.svm, lock);
    assert_error(
        h.lock(lock_id, 1, now.unix_timestamp + 7_200),
        "already in use",
    );
    assert_eq!(account_bytes(&h.svm, lock), lock_before);
    assert_eq!(account_bytes(&h.svm, h.position()), position_before);
    h.assert_custody_invariant();
}

#[test]
fn unauthorized_and_over_capacity_locks_roll_back() {
    let mut h = Harness::new();
    h.deposit_stake(200_000).expect("deposit stake");
    let now: anchor_lang::prelude::Clock = h.svm.get_sysvar();
    let lock_id = [22_u8; 32];
    let lock = h.lock_address(lock_id);

    let position_before = account_bytes(&h.svm, h.position());
    assert_error(
        h.lock(lock_id, 200_001, now.unix_timestamp + 3_600),
        "InsufficientFreeStake",
    );
    assert_eq!(account_bytes(&h.svm, h.position()), position_before);
    assert!(h.svm.get_account(&lock).is_none());

    let attacker = Keypair::new();
    h.svm
        .airdrop(&attacker.pubkey(), 10_000_000_000)
        .expect("fund attacker");
    let attack_id = [23_u8; 32];
    let attack_lock = h.lock_address(attack_id);
    let ix = instruction(
        z_accounts::ControllerLockStake {
            controller: attacker.pubkey(),
            vault: h.vault,
            stake_owner: h.owner.pubkey(),
            position: h.position(),
            stake_lock: attack_lock,
            system_program: solana_sdk_ids::system_program::ID,
        },
        z_instruction::ControllerLockStake {
            args: ControllerLockArgs {
                lock_id: attack_id,
                amount: 1,
                matures_at: now.unix_timestamp + 3_600,
            },
        },
    );
    assert_error(
        send(&mut h.svm, &attacker, vec![ix], &[]),
        "ConstraintAddress",
    );
    assert_eq!(account_bytes(&h.svm, h.position()), position_before);
    assert!(h.svm.get_account(&attack_lock).is_none());
    h.assert_custody_invariant();
}

#[test]
fn resize_and_unlock_preserve_principal_conservation() {
    let mut h = Harness::new();
    h.deposit_stake(600_000).expect("deposit stake");
    let now: anchor_lang::prelude::Clock = h.svm.get_sysvar();
    let lock_id = [31_u8; 32];
    let lock_address = h.lock_address(lock_id);

    h.lock(lock_id, 200_000, now.unix_timestamp + 7_200)
        .expect("create lock");
    h.increase_lock(lock_id, 175_000)
        .expect("increase from free stake");
    let lock: StakeLock = read_anchor(&h.svm, lock_address);
    assert_eq!(lock.amount, 375_000);
    assert_eq!(h.position_state().locked, 375_000);
    assert_eq!(h.position_state().balance, 600_000);

    h.resize_lock(lock_id, 125_000, now.unix_timestamp + 10_800)
        .expect("shrink and re-time lock");
    let lock: StakeLock = read_anchor(&h.svm, lock_address);
    assert_eq!(lock.amount, 125_000);
    assert_eq!(h.position_state().locked, 125_000);
    assert_eq!(h.position_state().balance, 600_000);
    h.assert_custody_invariant();

    h.unlock(lock_id).expect("unlock complete lock");
    assert!(h.svm.get_account(&lock_address).is_none());
    assert_eq!(h.position_state().locked, 0);
    assert_eq!(h.position_state().balance, 600_000);
    h.withdraw_stake(600_000).expect("all principal is free");
    assert_eq!(h.vault_state().total_staked, 0);
    assert_eq!(token_amount(&h.svm, h.vault_token), 0);
    assert_eq!(token_amount(&h.svm, h.owner_token), USER_STARTING_TOKENS);
    h.assert_custody_invariant();
}

#[test]
fn resolving_a_lock_moves_principal_to_claimable_then_claims_exactly() {
    let mut h = Harness::new();
    h.deposit_stake(500_000).expect("deposit stake");
    let now: anchor_lang::prelude::Clock = h.svm.get_sysvar();
    let lock_id = [41_u8; 32];
    let lock_address = h.lock_address(lock_id);
    h.lock(lock_id, 300_000, now.unix_timestamp + 3_600)
        .expect("lock principal");

    let beneficiary = Keypair::new();
    h.svm
        .airdrop(&beneficiary.pubkey(), 10_000_000_000)
        .expect("fund beneficiary");
    let beneficiary_token = Keypair::new().pubkey();
    install_token_account(
        &mut h.svm,
        beneficiary_token,
        h.mint,
        beneficiary.pubkey(),
        0,
    );
    let claim = pda(
        &[
            CLAIM_BALANCE_SEED,
            h.vault.as_ref(),
            beneficiary.pubkey().as_ref(),
        ],
        &program_id(),
    );
    let initialize_claim = instruction(
        z_accounts::InitializeClaimBalance {
            payer: h.controller.pubkey(),
            vault: h.vault,
            beneficiary: beneficiary.pubkey(),
            claim,
            system_program: solana_sdk_ids::system_program::ID,
        },
        z_instruction::InitializeClaimBalance,
    );
    send_ok(&mut h.svm, &h.controller, vec![initialize_claim], &[]);

    let position_before = account_bytes(&h.svm, h.position());
    let vault_before = account_bytes(&h.svm, h.vault);
    let lock_before = account_bytes(&h.svm, lock_address);
    let claim_before = account_bytes(&h.svm, claim);
    let mut overclaim = instruction(
        z_accounts::ResolveStakeLock {
            controller: h.controller.pubkey(),
            vault: h.vault,
            position: h.position(),
            stake_lock: lock_address,
        },
        z_instruction::ResolveStakeLock {
            lock_id,
            claims: vec![StakeClaim {
                beneficiary: beneficiary.pubkey(),
                amount: 300_001,
            }],
        },
    );
    overclaim.accounts.push(AccountMeta::new(claim, false));
    assert_error(
        send(&mut h.svm, &h.controller, vec![overclaim], &[]),
        "ClaimsExceedLock",
    );
    assert_eq!(account_bytes(&h.svm, h.position()), position_before);
    assert_eq!(account_bytes(&h.svm, h.vault), vault_before);
    assert_eq!(account_bytes(&h.svm, lock_address), lock_before);
    assert_eq!(account_bytes(&h.svm, claim), claim_before);

    let mut resolve = instruction(
        z_accounts::ResolveStakeLock {
            controller: h.controller.pubkey(),
            vault: h.vault,
            position: h.position(),
            stake_lock: lock_address,
        },
        z_instruction::ResolveStakeLock {
            lock_id,
            claims: vec![StakeClaim {
                beneficiary: beneficiary.pubkey(),
                amount: 100_000,
            }],
        },
    );
    resolve.accounts.push(AccountMeta::new(claim, false));
    send_ok(&mut h.svm, &h.controller, vec![resolve], &[]);

    assert!(h.svm.get_account(&lock_address).is_none());
    let position = h.position_state();
    assert_eq!(position.balance, 400_000);
    assert_eq!(position.locked, 0);
    let vault = h.vault_state();
    assert_eq!(vault.total_staked, 400_000);
    assert_eq!(vault.total_claimable, 100_000);
    assert_eq!(read_anchor::<ClaimBalance>(&h.svm, claim).amount, 100_000);
    assert_eq!(token_amount(&h.svm, h.vault_token), 500_000);
    h.assert_custody_invariant();

    let claim_ix = instruction(
        z_accounts::ClaimStake {
            beneficiary: beneficiary.pubkey(),
            vault: h.vault,
            claim,
            mint: h.mint,
            vault_token: h.vault_token,
            beneficiary_token,
            token_program: spl_token::ID,
        },
        z_instruction::ClaimStake,
    );
    send_ok(&mut h.svm, &beneficiary, vec![claim_ix], &[]);
    assert_eq!(token_amount(&h.svm, beneficiary_token), 100_000);
    assert_eq!(token_amount(&h.svm, h.vault_token), 400_000);
    assert_eq!(h.vault_state().total_staked, 400_000);
    assert_eq!(h.vault_state().total_claimable, 0);
    assert_eq!(read_anchor::<ClaimBalance>(&h.svm, claim).amount, 0);
    h.assert_custody_invariant();
}

#[test]
fn escrow_create_add_remove_and_withdraw_track_exact_liquidity() {
    let mut h = Harness::new();
    let addresses = h.create_deposit(400_000).expect("create deposit");

    let deposit: Deposit = read_anchor(&h.svm, addresses.deposit);
    assert_eq!(deposit.depositor, h.owner.pubkey());
    assert_eq!(deposit.remaining_deposits, 400_000);
    assert_eq!(deposit.outstanding_intent_amount, 0);
    assert_eq!(deposit.active_intents, 0);
    assert!(!deposit.retain_on_empty);
    assert_eq!(token_amount(&h.svm, addresses.vault), 400_000);
    assert_eq!(token_amount(&h.svm, h.owner_token), 600_000);

    let add = instruction(
        z_accounts::AddFunds {
            funder: h.owner.pubkey(),
            escrow_config: h.escrow,
            deposit: addresses.deposit,
            token_mint: h.mint,
            funder_token: h.owner_token,
            deposit_vault: addresses.vault,
            token_program: spl_token::ID,
        },
        z_instruction::AddFunds { amount: 100_000 },
    );
    send_ok(&mut h.svm, &h.owner, vec![add], &[]);
    assert_eq!(
        read_anchor::<Deposit>(&h.svm, addresses.deposit).remaining_deposits,
        500_000
    );
    assert_eq!(token_amount(&h.svm, addresses.vault), 500_000);

    let remove = |amount| {
        instruction(
            z_accounts::RemoveFunds {
                depositor: h.owner.pubkey(),
                escrow_config: h.escrow,
                deposit: addresses.deposit,
                token_mint: h.mint,
                deposit_vault: addresses.vault,
                depositor_token: h.owner_token,
                token_program: spl_token::ID,
            },
            z_instruction::RemoveFunds { amount },
        )
    };
    send_ok(&mut h.svm, &h.owner, vec![remove(125_000)], &[]);
    assert_eq!(
        read_anchor::<Deposit>(&h.svm, addresses.deposit).remaining_deposits,
        375_000
    );
    assert_eq!(token_amount(&h.svm, addresses.vault), 375_000);
    assert_eq!(token_amount(&h.svm, h.owner_token), 625_000);

    let deposit_before = account_bytes(&h.svm, addresses.deposit);
    let vault_before = token_amount(&h.svm, addresses.vault);
    assert_error(
        send(&mut h.svm, &h.owner, vec![remove(375_001)], &[]),
        "InsufficientBalance",
    );
    assert_eq!(account_bytes(&h.svm, addresses.deposit), deposit_before);
    assert_eq!(token_amount(&h.svm, addresses.vault), vault_before);

    let withdraw = instruction(
        z_accounts::WithdrawDeposit {
            depositor: h.owner.pubkey(),
            escrow_config: h.escrow,
            deposit: addresses.deposit,
            token_mint: h.mint,
            deposit_vault: addresses.vault,
            depositor_token: h.owner_token,
            token_program: spl_token::ID,
            dust_recipient_token: None,
        },
        z_instruction::WithdrawDeposit,
    );
    send_ok(&mut h.svm, &h.owner, vec![withdraw], &[]);
    assert_eq!(token_amount(&h.svm, h.owner_token), USER_STARTING_TOKENS);
    assert!(
        h.svm.get_account(&addresses.deposit).is_none(),
        "retain_on_empty=false deposit with no intents must close after full withdrawal"
    );
    assert!(
        h.svm.get_account(&addresses.vault).is_none(),
        "empty per-deposit token vault must close with its deposit"
    );
}

fn pda(seeds: &[&[u8]], program_id: &anchor_lang::prelude::Pubkey) -> anchor_lang::prelude::Pubkey {
    anchor_lang::prelude::Pubkey::find_program_address(seeds, program_id).0
}

fn program_id() -> anchor_lang::prelude::Pubkey {
    anchor_lang::prelude::Pubkey::from_str(IDL_PROGRAM_ID).expect("valid public IDL program id")
}

fn instruction<A: ToAccountMetas, D: InstructionData>(accounts: A, data: D) -> Instruction {
    Instruction {
        program_id: program_id(),
        accounts: accounts.to_account_metas(None),
        data: data.data(),
    }
}

fn send(
    svm: &mut LiteSVM,
    payer: &Keypair,
    instructions: Vec<Instruction>,
    other_signers: &[&Keypair],
) -> Result<(), String> {
    let mut signers: Vec<&dyn Signer> = vec![payer];
    signers.extend(other_signers.iter().map(|s| *s as &dyn Signer));
    let tx = Transaction::new_signed_with_payer(
        &instructions,
        Some(&payer.pubkey()),
        &signers,
        svm.latest_blockhash(),
    );
    svm.send_transaction(tx)
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
}

fn send_ok(
    svm: &mut LiteSVM,
    payer: &Keypair,
    instructions: Vec<Instruction>,
    other_signers: &[&Keypair],
) {
    if let Err(error) = send(svm, payer, instructions, other_signers) {
        panic!("transaction failed: {error}");
    }
}

fn assert_error(result: Result<(), String>, expected: &str) {
    let error = result.expect_err("transaction unexpectedly succeeded");
    assert!(
        error.contains(expected),
        "expected error containing {expected:?}, got {error}"
    );
}

fn assert_failed(result: Result<(), String>) {
    assert!(result.is_err(), "transaction unexpectedly succeeded");
}

fn read_anchor<T: AccountDeserialize>(svm: &LiteSVM, address: anchor_lang::prelude::Pubkey) -> T {
    let account = svm.get_account(&address).expect("account exists");
    let mut data: &[u8] = &account.data;
    T::try_deserialize(&mut data).expect("deserialize Anchor account")
}

fn account_bytes(svm: &LiteSVM, address: anchor_lang::prelude::Pubkey) -> Vec<u8> {
    svm.get_account(&address)
        .expect("account exists")
        .data
        .clone()
}

fn token_amount(svm: &LiteSVM, address: anchor_lang::prelude::Pubkey) -> u64 {
    let account = svm.get_account(&address).expect("token account exists");
    SplAccount::unpack(&account.data)
        .expect("valid SPL token account")
        .amount
}

fn install_mint(
    svm: &mut LiteSVM,
    address: anchor_lang::prelude::Pubkey,
    authority: anchor_lang::prelude::Pubkey,
    supply: u64,
) {
    let state = Mint {
        mint_authority: COption::Some(authority),
        supply,
        decimals: 6,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut data = vec![0_u8; Mint::LEN];
    Mint::pack(state, &mut data).expect("pack mint");
    svm.set_account(
        address,
        Account {
            lamports: 10_000_000,
            data,
            owner: spl_token::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .expect("install mint");
}

fn install_token_account(
    svm: &mut LiteSVM,
    address: anchor_lang::prelude::Pubkey,
    mint: anchor_lang::prelude::Pubkey,
    owner: anchor_lang::prelude::Pubkey,
    amount: u64,
) {
    let state = SplAccount {
        mint,
        owner,
        amount,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0_u8; SplAccount::LEN];
    SplAccount::pack(state, &mut data).expect("pack token account");
    svm.set_account(
        address,
        Account {
            lamports: 10_000_000,
            data,
            owner: spl_token::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .expect("install token account");
}
