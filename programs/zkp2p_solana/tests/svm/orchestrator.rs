//! Real-SBF OrchestratorV3 signal, guardian, cancel, prune, and manual-release paths.

use super::common::{address, anchor_pubkey, install_token_account, pda, token_amount, Fixture};
use solana_address::Address;
use solana_signer::Signer;
use zkp2p_solana::{
    constants::{
        DEPOSIT_CURRENCY_SEED, DEPOSIT_SEED, DEPOSIT_VAULT_SEED, ESCROW_INTENT_LOCK_SEED,
        INTENT_SEED, PAYMENT_METHOD_SEED, TAKER_INTENT_STATE_SEED,
    },
    derive_intent_hash, AmountRange, ConfigureOrchestratorArgs, CreateDepositArgs, LifecyclePolicy,
    SignalIntentArgs,
};

struct IntentFixture {
    fixture: Fixture,
    depositor: anchor_lang::prelude::Pubkey,
    deposit: anchor_lang::prelude::Pubkey,
    deposit_vault: anchor_lang::prelude::Pubkey,
    payment_method: anchor_lang::prelude::Pubkey,
    payment_method_id: [u8; 32],
    currency: anchor_lang::prelude::Pubkey,
    currency_id: [u8; 32],
}

impl IntentFixture {
    fn new() -> Self {
        let mut fixture = Fixture::new();
        let depositor = anchor_pubkey(fixture.authority.pubkey());
        let depositor_token = anchor_pubkey(Address::new_unique());
        install_token_account(
            &mut fixture.svm,
            depositor_token,
            fixture.mint,
            depositor,
            1_000,
        );
        let deposit = pda(&[DEPOSIT_SEED, fixture.escrow.as_ref(), &0_u64.to_le_bytes()]);
        let payment_method_id = [1_u8; 32];
        let currency_id = [2_u8; 32];
        let payment_method = pda(&[PAYMENT_METHOD_SEED, deposit.as_ref(), &payment_method_id]);
        let currency = pda(&[
            DEPOSIT_CURRENCY_SEED,
            deposit.as_ref(),
            &payment_method_id,
            &currency_id,
        ]);
        let deposit_vault = pda(&[DEPOSIT_VAULT_SEED, deposit.as_ref()]);
        let create = fixture.program_instruction(
            zkp2p_solana::accounts::CreateDeposit {
                depositor,
                escrow_config: fixture.escrow,
                deposit,
                payment_method,
                currency,
                mint: fixture.mint,
                depositor_token,
                deposit_vault,
                token_program: anchor_spl::token::ID,
                system_program: anchor_lang::system_program::ID,
            },
            zkp2p_solana::instruction::CreateDeposit {
                args: CreateDepositArgs {
                    amount: 500,
                    intent_amount_range: AmountRange { min: 10, max: 200 },
                    delegate: None,
                    intent_guardian: Some(depositor),
                    retain_on_empty: true,
                    payment_method: payment_method_id,
                    payee_details: [3_u8; 32],
                    gating_service: None,
                    currency: currency_id,
                    fixed_min_rate: 1_000_000_000_000_000_000,
                    oracle_quote: None,
                    spread_bps: 0,
                    max_staleness: 0,
                },
            },
        );
        fixture.send(&[create]).expect("create intent deposit");
        let disable_lifecycle = fixture.program_instruction(
            zkp2p_solana::accounts::ConfigureOrchestrator {
                authority: depositor,
                protocol: fixture.protocol,
                orchestrator: fixture.orchestrator,
            },
            zkp2p_solana::instruction::ConfigureOrchestrator {
                args: ConfigureOrchestratorArgs {
                    protocol_fee: None,
                    protocol_fee_recipient: None,
                    lifecycle_policy: Some(LifecyclePolicy::None),
                    allow_multiple_intents: None,
                    paused: None,
                },
            },
        );
        fixture
            .send(&[disable_lifecycle])
            .expect("select no-hook lifecycle");
        Self {
            fixture,
            depositor,
            deposit,
            deposit_vault,
            payment_method,
            payment_method_id,
            currency,
            currency_id,
        }
    }

    fn intent_addresses(
        &self,
        nonce: u64,
    ) -> (
        [u8; 32],
        anchor_lang::prelude::Pubkey,
        anchor_lang::prelude::Pubkey,
        anchor_lang::prelude::Pubkey,
    ) {
        let intent_hash = derive_intent_hash(self.fixture.orchestrator, nonce);
        let intent = pda(&[
            INTENT_SEED,
            self.fixture.orchestrator.as_ref(),
            &nonce.to_le_bytes(),
        ]);
        let lock = pda(&[ESCROW_INTENT_LOCK_SEED, self.deposit.as_ref(), &intent_hash]);
        let taker_state = pda(&[
            TAKER_INTENT_STATE_SEED,
            self.fixture.orchestrator.as_ref(),
            self.depositor.as_ref(),
        ]);
        (intent_hash, intent, lock, taker_state)
    }

    fn signal(&self, nonce: u64, amount: u64) -> solana_instruction::Instruction {
        let (intent_hash, intent, intent_lock, taker_state) = self.intent_addresses(nonce);
        self.fixture.program_instruction(
            zkp2p_solana::accounts::SignalIntent {
                taker: self.depositor,
                orchestrator: self.fixture.orchestrator,
                escrow_config: self.fixture.escrow,
                deposit: self.deposit,
                payment_method: self.payment_method,
                deposit_currency: self.currency,
                oracle_quote: None,
                rate_manager: None,
                rate_entry: None,
                deposit_whitelist: None,
                direct_whitelist_member: None,
                allowed_group: None,
                group_member: None,
                resolver_program: None,
                dispute_intent: None,
                deposit_dispute_setting: None,
                risk_window: None,
                intent,
                intent_lock,
                taker_state,
                instructions_sysvar: solana_program::sysvar::instructions::ID,
                system_program: anchor_lang::system_program::ID,
            },
            zkp2p_solana::instruction::SignalIntent {
                args: SignalIntentArgs {
                    expected_intent_hash: intent_hash,
                    amount,
                    recipient: self.depositor,
                    payment_method: self.payment_method_id,
                    fiat_currency: self.currency_id,
                    conversion_rate: 1_000_000_000_000_000_000,
                    referral_fees: Vec::new(),
                    gating_signature_expiration: 0,
                },
            },
        )
    }
}

#[test]
fn signal_extend_cancel_prune_and_manual_release_conserve_deposit() {
    let mut intent_fixture = IntentFixture::new();
    let signal_first = intent_fixture.signal(0, 100);
    intent_fixture
        .fixture
        .send(&[signal_first])
        .expect("signal first intent");
    let (first_hash, first_intent, first_lock, taker_state) = intent_fixture.intent_addresses(0);
    let extend = intent_fixture.fixture.program_instruction(
        zkp2p_solana::accounts::ExtendIntentExpiry {
            guardian: intent_fixture.depositor,
            deposit: intent_fixture.deposit,
            intent_lock: first_lock,
        },
        zkp2p_solana::instruction::ExtendIntentExpiry {
            additional_time: 60,
        },
    );
    intent_fixture
        .fixture
        .send(&[extend])
        .expect("guardian extends intent");
    let cancel = intent_fixture.fixture.program_instruction(
        zkp2p_solana::accounts::CancelIntent {
            owner: intent_fixture.depositor,
            orchestrator: intent_fixture.fixture.orchestrator,
            intent: first_intent,
            deposit: intent_fixture.deposit,
            intent_lock: first_lock,
            resolved_dispute: None,
            taker_state,
        },
        zkp2p_solana::instruction::CancelIntent {},
    );
    intent_fixture
        .fixture
        .send(&[cancel])
        .expect("cancel first intent");
    assert!(intent_fixture
        .fixture
        .svm
        .get_account(&address(first_intent.to_bytes()))
        .is_none());

    let signal_second = intent_fixture.signal(1, 80);
    intent_fixture
        .fixture
        .send(&[signal_second])
        .expect("signal second intent");
    let (_second_hash, second_intent, second_lock, _) = intent_fixture.intent_addresses(1);
    let mut clock = intent_fixture
        .fixture
        .svm
        .get_sysvar::<anchor_lang::prelude::Clock>();
    clock.unix_timestamp = clock
        .unix_timestamp
        .checked_add(2_000)
        .expect("fixture clock fits");
    intent_fixture.fixture.svm.set_sysvar(&clock);
    let prune = intent_fixture.fixture.program_instruction(
        zkp2p_solana::accounts::PruneExpiredIntent {
            caller: intent_fixture.depositor,
            owner_rent: intent_fixture.depositor,
            orchestrator: intent_fixture.fixture.orchestrator,
            intent: second_intent,
            deposit: intent_fixture.deposit,
            intent_lock: second_lock,
            taker_state,
            dispute: zkp2p_solana::accounts::PruneDisputeAccounts {
                dispute_config: None,
                dispute_intent: None,
                stake_vault: None,
                stake_position: None,
                stake_lock: None,
            },
        },
        zkp2p_solana::instruction::PruneExpiredIntent {},
    );
    intent_fixture
        .fixture
        .send(&[prune])
        .expect("prune expired intent");

    let signal_third = intent_fixture.signal(2, 100);
    intent_fixture
        .fixture
        .send(&[signal_third])
        .expect("signal third intent");
    let (_third_hash, third_intent, third_lock, _) = intent_fixture.intent_addresses(2);
    let recipient_token = anchor_pubkey(Address::new_unique());
    let protocol_fee_token = anchor_pubkey(Address::new_unique());
    install_token_account(
        &mut intent_fixture.fixture.svm,
        recipient_token,
        intent_fixture.fixture.mint,
        intent_fixture.depositor,
        0,
    );
    install_token_account(
        &mut intent_fixture.fixture.svm,
        protocol_fee_token,
        intent_fixture.fixture.mint,
        intent_fixture.depositor,
        0,
    );
    let manual_release = intent_fixture.fixture.program_instruction(
        zkp2p_solana::accounts::ManualRelease {
            depositor: intent_fixture.depositor,
            owner_rent: intent_fixture.depositor,
            orchestrator: intent_fixture.fixture.orchestrator,
            intent: third_intent,
            deposit: intent_fixture.deposit,
            intent_lock: third_lock,
            taker_state,
            tokens: zkp2p_solana::accounts::SettlementTokenAccounts {
                mint: intent_fixture.fixture.mint,
                deposit_vault: intent_fixture.deposit_vault,
                recipient_token,
                protocol_fee_token,
                manager_fee_token: None,
                token_program: anchor_spl::token::ID,
            },
            dispute: zkp2p_solana::accounts::SettlementDisputeAccounts {
                dispute_intent: None,
                stake_vault: None,
                stake_position: None,
                stake_lock: None,
            },
        },
        zkp2p_solana::instruction::ManualRelease {},
    );
    intent_fixture
        .fixture
        .send(&[manual_release])
        .expect("manually release intent");
    assert_eq!(
        token_amount(&intent_fixture.fixture.svm, recipient_token),
        99
    );
    assert_eq!(
        token_amount(&intent_fixture.fixture.svm, protocol_fee_token),
        1
    );
    assert_eq!(
        token_amount(&intent_fixture.fixture.svm, intent_fixture.deposit_vault),
        400
    );

    let _hashes_are_nonzero = (
        first_hash,
        derive_intent_hash(intent_fixture.fixture.orchestrator, 2),
    );
    assert_ne!(_hashes_are_nonzero.0, [0; 32]);
}
