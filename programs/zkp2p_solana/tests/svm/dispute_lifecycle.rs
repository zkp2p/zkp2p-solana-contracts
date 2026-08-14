//! Real-SBF dispute admission, cancellation, covered settlement, and maturity release.

use super::common::{
    address, anchor_pubkey, install_token_account, pda, send_with_compute, token_amount, Fixture,
};
use anchor_lang::{AccountDeserialize, AnchorSerialize};
use k256::ecdsa::SigningKey;
use solana_address::Address;
use solana_signer::Signer;
use zkp2p_solana::{
    constants::{
        DEPOSIT_CURRENCY_SEED, DEPOSIT_SEED, DEPOSIT_VAULT_SEED, DISPUTE_INTENT_SEED,
        DISPUTE_NULLIFIER_SEED, ESCROW_INTENT_LOCK_SEED, INTENT_PAYMENT_BINDING_SEED, INTENT_SEED,
        PAYMENT_BINDING_SEED, PAYMENT_METHOD_SEED, RISK_WINDOW_SEED, STAKE_LOCK_SEED,
        STAKE_POSITION_SEED, STAKE_TOKEN_VAULT_SEED, TAKER_INTENT_STATE_SEED,
    },
    derive_intent_hash, dispute_attestation_digest, payment_attestation_digest, AmountRange,
    ClaimBalance, CreateDepositArgs, DisputeAttestation, DisputeDetails, FulfillIntentArgs, Intent,
    IntentSnapshot, PaymentAttestation, PaymentDetails, PrepareDisputeArgs, SignalIntentArgs,
    SubmitDisputeArgs,
};

fn decode<T: AccountDeserialize>(fixture: &Fixture, key: anchor_lang::prelude::Pubkey) -> T {
    let account = fixture
        .svm
        .get_account(&address(key.to_bytes()))
        .expect("account exists");
    let mut data = account.data.as_slice();
    T::try_deserialize(&mut data).expect("account decodes")
}

fn witness_key() -> (SigningKey, [u8; 20]) {
    let signing_key = SigningKey::from_slice(&[42_u8; 32]).expect("fixture signing key");
    let encoded = signing_key.verifying_key().to_encoded_point(false);
    let public_key = encoded
        .as_bytes()
        .get(1..)
        .expect("uncompressed public key body");
    let hash = solana_keccak_hasher::hash(public_key).to_bytes();
    let address: [u8; 20] = hash
        .get(12..)
        .expect("Ethereum address suffix")
        .try_into()
        .expect("Ethereum address width");
    (signing_key, address)
}

fn sign_digest(signing_key: &SigningKey, digest: [u8; 32]) -> [u8; 65] {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .expect("sign fixture digest");
    let mut output = [0_u8; 65];
    output
        .get_mut(..64)
        .expect("signature body")
        .copy_from_slice(signature.to_bytes().as_ref());
    *output.get_mut(64).expect("recovery byte") = recovery_id.to_byte();
    output
}

struct CoveredFixture {
    fixture: Fixture,
    actor: anchor_lang::prelude::Pubkey,
    deposit: anchor_lang::prelude::Pubkey,
    deposit_vault: anchor_lang::prelude::Pubkey,
    payment_method: anchor_lang::prelude::Pubkey,
    payment_method_id: [u8; 32],
    currency: anchor_lang::prelude::Pubkey,
    currency_id: [u8; 32],
    risk_window: anchor_lang::prelude::Pubkey,
    stake_position: anchor_lang::prelude::Pubkey,
    stake_vault_token: anchor_lang::prelude::Pubkey,
}

impl CoveredFixture {
    fn new() -> Self {
        let mut fixture = Fixture::new();
        let actor = anchor_pubkey(fixture.authority.pubkey());
        let actor_token = anchor_pubkey(Address::new_unique());
        install_token_account(&mut fixture.svm, actor_token, fixture.mint, actor, 2_000);
        let stake_vault_token = pda(&[STAKE_TOKEN_VAULT_SEED, fixture.stake_vault.as_ref()]);
        let initialize_stake_vault = fixture.program_instruction(
            zkp2p_solana::accounts::InitializeStakeTokenVault {
                payer: actor,
                vault: fixture.stake_vault,
                mint: fixture.mint,
                vault_token: stake_vault_token,
                token_program: anchor_spl::token::ID,
                system_program: anchor_lang::system_program::ID,
            },
            zkp2p_solana::instruction::InitializeStakeTokenVault {},
        );
        fixture
            .send(&[initialize_stake_vault])
            .expect("initialize stake vault");
        let stake_position = pda(&[
            STAKE_POSITION_SEED,
            fixture.stake_vault.as_ref(),
            actor.as_ref(),
        ]);
        let deposit_stake = fixture.program_instruction(
            zkp2p_solana::accounts::DepositStake {
                owner: actor,
                vault: fixture.stake_vault,
                position: stake_position,
                mint: fixture.mint,
                owner_token: actor_token,
                vault_token: stake_vault_token,
                token_program: anchor_spl::token::ID,
                system_program: anchor_lang::system_program::ID,
            },
            zkp2p_solana::instruction::DepositStake { amount: 500 },
        );
        fixture
            .send(&[deposit_stake])
            .expect("fund dispute collateral");

        let deposit = pda(&[DEPOSIT_SEED, fixture.escrow.as_ref(), &0_u64.to_le_bytes()]);
        let payment_method_id = [21_u8; 32];
        let currency_id = [22_u8; 32];
        let payment_method = pda(&[PAYMENT_METHOD_SEED, deposit.as_ref(), &payment_method_id]);
        let currency = pda(&[
            DEPOSIT_CURRENCY_SEED,
            deposit.as_ref(),
            &payment_method_id,
            &currency_id,
        ]);
        let deposit_vault = pda(&[DEPOSIT_VAULT_SEED, deposit.as_ref()]);
        let create_deposit = fixture.program_instruction(
            zkp2p_solana::accounts::CreateDeposit {
                depositor: actor,
                escrow_config: fixture.escrow,
                deposit,
                payment_method,
                currency,
                mint: fixture.mint,
                depositor_token: actor_token,
                deposit_vault,
                token_program: anchor_spl::token::ID,
                system_program: anchor_lang::system_program::ID,
            },
            zkp2p_solana::instruction::CreateDeposit {
                args: CreateDepositArgs {
                    amount: 500,
                    intent_amount_range: AmountRange { min: 10, max: 200 },
                    delegate: None,
                    intent_guardian: None,
                    retain_on_empty: true,
                    payment_method: payment_method_id,
                    payee_details: [23_u8; 32],
                    gating_service: None,
                    currency: currency_id,
                    fixed_min_rate: 1_000_000_000_000_000_000,
                    oracle_quote: None,
                    spread_bps: 0,
                    max_staleness: 0,
                },
            },
        );
        fixture
            .send(&[create_deposit])
            .expect("create covered deposit");

        let risk_window = pda(&[
            RISK_WINDOW_SEED,
            fixture.dispute_config.as_ref(),
            &payment_method_id,
        ]);
        let set_risk = fixture.program_instruction(
            zkp2p_solana::accounts::SetRiskWindow {
                authority: actor,
                protocol: fixture.protocol,
                dispute_config: fixture.dispute_config,
                risk_window,
                system_program: anchor_lang::system_program::ID,
            },
            zkp2p_solana::instruction::SetRiskWindow {
                payment_method: payment_method_id,
                seconds: 3_600,
            },
        );
        fixture.send(&[set_risk]).expect("set dispute risk");
        Self {
            fixture,
            actor,
            deposit,
            deposit_vault,
            payment_method,
            payment_method_id,
            currency,
            currency_id,
            risk_window,
            stake_position,
            stake_vault_token,
        }
    }

    fn addresses(
        &self,
        nonce: u64,
    ) -> (
        [u8; 32],
        anchor_lang::prelude::Pubkey,
        anchor_lang::prelude::Pubkey,
        anchor_lang::prelude::Pubkey,
        anchor_lang::prelude::Pubkey,
        anchor_lang::prelude::Pubkey,
    ) {
        let hash = derive_intent_hash(self.fixture.orchestrator, nonce);
        (
            hash,
            pda(&[
                INTENT_SEED,
                self.fixture.orchestrator.as_ref(),
                &nonce.to_le_bytes(),
            ]),
            pda(&[ESCROW_INTENT_LOCK_SEED, self.deposit.as_ref(), &hash]),
            pda(&[
                DISPUTE_INTENT_SEED,
                self.fixture.dispute_config.as_ref(),
                &hash,
            ]),
            pda(&[STAKE_LOCK_SEED, self.fixture.stake_vault.as_ref(), &hash]),
            pda(&[
                TAKER_INTENT_STATE_SEED,
                self.fixture.orchestrator.as_ref(),
                self.actor.as_ref(),
            ]),
        )
    }

    fn prepare_and_signal(
        &mut self,
        nonce: u64,
        amount: u64,
    ) -> (
        [u8; 32],
        anchor_lang::prelude::Pubkey,
        anchor_lang::prelude::Pubkey,
        anchor_lang::prelude::Pubkey,
        anchor_lang::prelude::Pubkey,
        anchor_lang::prelude::Pubkey,
    ) {
        let addresses = self.addresses(nonce);
        let (hash, intent, intent_lock, dispute_intent, stake_lock, taker_state) = addresses;
        let prepare = self.fixture.program_instruction(
            zkp2p_solana::accounts::PrepareDispute {
                taker: self.actor,
                orchestrator: self.fixture.orchestrator,
                dispute_config: self.fixture.dispute_config,
                stake_vault: self.fixture.stake_vault,
                deposit: self.deposit,
                deposit_setting: None,
                risk_window: self.risk_window,
                selection: None,
                stake_owner: self.actor,
                position: self.stake_position,
                stake_lock,
                dispute_intent,
                system_program: anchor_lang::system_program::ID,
            },
            zkp2p_solana::instruction::PrepareDispute {
                args: PrepareDisputeArgs {
                    expected_intent_hash: hash,
                    payment_method: self.payment_method_id,
                    amount,
                },
            },
        );
        let signal = self.fixture.program_instruction(
            zkp2p_solana::accounts::SignalIntent {
                taker: self.actor,
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
                dispute_intent: Some(dispute_intent),
                deposit_dispute_setting: None,
                risk_window: Some(self.risk_window),
                intent,
                intent_lock,
                taker_state,
                instructions_sysvar: solana_program::sysvar::instructions::ID,
                system_program: anchor_lang::system_program::ID,
            },
            zkp2p_solana::instruction::SignalIntent {
                args: SignalIntentArgs {
                    expected_intent_hash: hash,
                    amount,
                    recipient: self.actor,
                    payment_method: self.payment_method_id,
                    fiat_currency: self.currency_id,
                    conversion_rate: 1_000_000_000_000_000_000,
                    referral_fees: Vec::new(),
                    gating_signature_expiration: 0,
                },
            },
        );
        self.fixture
            .send(&[prepare, signal])
            .expect("prepare and signal covered intent");
        addresses
    }
}

#[test]
fn covered_intents_cancel_and_release_only_through_matching_dispute_states() {
    let mut covered = CoveredFixture::new();
    let (_hash, intent, intent_lock, dispute_intent, stake_lock, taker_state) =
        covered.prepare_and_signal(0, 100);
    let cancel_dispute = covered.fixture.program_instruction(
        zkp2p_solana::accounts::CancelDispute {
            owner: covered.actor,
            intent,
            stake_vault: covered.fixture.stake_vault,
            position: covered.stake_position,
            stake_lock,
            dispute_intent,
        },
        zkp2p_solana::instruction::CancelDispute {},
    );
    let cancel_intent = covered.fixture.program_instruction(
        zkp2p_solana::accounts::CancelIntent {
            owner: covered.actor,
            orchestrator: covered.fixture.orchestrator,
            intent,
            deposit: covered.deposit,
            intent_lock,
            resolved_dispute: Some(dispute_intent),
            taker_state,
        },
        zkp2p_solana::instruction::CancelIntent {},
    );
    covered
        .fixture
        .send(&[cancel_dispute, cancel_intent])
        .expect("cancel covered intent in order");

    let (_hash, intent, intent_lock, dispute_intent, stake_lock, taker_state) =
        covered.prepare_and_signal(1, 100);
    let recipient_token = anchor_pubkey(Address::new_unique());
    let protocol_fee_token = anchor_pubkey(Address::new_unique());
    install_token_account(
        &mut covered.fixture.svm,
        recipient_token,
        covered.fixture.mint,
        covered.actor,
        0,
    );
    install_token_account(
        &mut covered.fixture.svm,
        protocol_fee_token,
        covered.fixture.mint,
        covered.actor,
        0,
    );
    let release = covered.fixture.program_instruction(
        zkp2p_solana::accounts::ManualRelease {
            depositor: covered.actor,
            owner_rent: covered.actor,
            orchestrator: covered.fixture.orchestrator,
            intent,
            deposit: covered.deposit,
            intent_lock,
            taker_state,
            tokens: zkp2p_solana::accounts::SettlementTokenAccounts {
                mint: covered.fixture.mint,
                deposit_vault: covered.deposit_vault,
                recipient_token,
                protocol_fee_token,
                manager_fee_token: None,
                token_program: anchor_spl::token::ID,
            },
            dispute: zkp2p_solana::accounts::SettlementDisputeAccounts {
                dispute_intent: Some(dispute_intent),
                stake_vault: Some(covered.fixture.stake_vault),
                stake_position: Some(covered.stake_position),
                stake_lock: Some(stake_lock),
            },
        },
        zkp2p_solana::instruction::ManualRelease {},
    );
    covered
        .fixture
        .send(&[release])
        .expect("settle covered intent");
    assert_eq!(token_amount(&covered.fixture.svm, recipient_token), 99);

    let mut clock = covered
        .fixture
        .svm
        .get_sysvar::<anchor_lang::prelude::Clock>();
    clock.unix_timestamp = clock
        .unix_timestamp
        .checked_add(3_601)
        .expect("fixture clock fits");
    covered.fixture.svm.set_sysvar(&clock);
    let release_matured = covered.fixture.program_instruction(
        zkp2p_solana::accounts::ReleaseMaturedDispute {
            caller: covered.actor,
            stake_vault: covered.fixture.stake_vault,
            position: covered.stake_position,
            stake_lock,
            dispute_intent,
        },
        zkp2p_solana::instruction::ReleaseMaturedDispute {},
    );
    covered
        .fixture
        .send(&[release_matured])
        .expect("release matured collateral");
    assert_eq!(
        token_amount(&covered.fixture.svm, covered.stake_vault_token),
        500
    );
}

#[test]
fn threshold_payment_fulfillment_binds_nullifiers_and_resolves_dispute_claim() {
    let mut covered = CoveredFixture::new();
    let (signing_key, witness) = witness_key();
    let verifier_accounts = zkp2p_solana::accounts::ConfigureVerifier {
        authority: covered.actor,
        protocol: covered.fixture.protocol,
        verifier: covered.fixture.verifier,
    };
    let add_witness = covered.fixture.program_instruction(
        verifier_accounts,
        zkp2p_solana::instruction::SetVerifierWitness {
            witness,
            enabled: true,
        },
    );
    let remove_initial = covered.fixture.program_instruction(
        verifier_accounts,
        zkp2p_solana::instruction::SetVerifierWitness {
            witness: [7_u8; 20],
            enabled: false,
        },
    );
    let add_method = covered.fixture.program_instruction(
        verifier_accounts,
        zkp2p_solana::instruction::SetVerifierPaymentMethod {
            payment_method: covered.payment_method_id,
            enabled: true,
        },
    );
    covered
        .fixture
        .send(&[add_witness, remove_initial, add_method])
        .expect("configure signing witness and method");

    let (intent_hash, intent_key, intent_lock, dispute_intent, stake_lock, taker_state) =
        covered.prepare_and_signal(0, 100);
    let intent: Intent = decode(&covered.fixture, intent_key);
    let payment = PaymentDetails {
        method: covered.payment_method_id,
        payee_id: intent.payee_id,
        amount: 100,
        currency: covered.currency_id,
        timestamp_ms: 1,
        payment_id: [31_u8; 32],
    };
    let snapshot = IntentSnapshot {
        intent_hash,
        amount: intent.amount,
        payment_method: intent.payment_method,
        fiat_currency: intent.fiat_currency,
        payee_details: intent.payee_id,
        conversion_rate: intent.conversion_rate,
        signal_timestamp: intent.timestamp,
        timestamp_buffer_ms: 0,
    };
    let mut payment_payload = Vec::new();
    payment
        .serialize(&mut payment_payload)
        .expect("serialize payment");
    snapshot
        .serialize(&mut payment_payload)
        .expect("serialize snapshot");
    let data_hash = solana_keccak_hasher::hash(&payment_payload).to_bytes();
    let payment_digest =
        payment_attestation_digest(covered.fixture.verifier, intent_hash, 100, data_hash);
    let attestation = PaymentAttestation {
        intent_hash,
        release_amount: 100,
        data_hash,
        signatures: vec![sign_digest(&signing_key, payment_digest)],
        payment,
        snapshot,
    };
    let payment_nullifier =
        solana_keccak_hasher::hashv(&[&covered.payment_method_id, &[31_u8; 32]]).to_bytes();
    let payment_binding = pda(&[
        PAYMENT_BINDING_SEED,
        covered.fixture.verifier.as_ref(),
        &payment_nullifier,
    ]);
    let intent_payment_binding = pda(&[
        INTENT_PAYMENT_BINDING_SEED,
        covered.fixture.verifier.as_ref(),
        &intent_hash,
    ]);
    let recipient_token = anchor_pubkey(Address::new_unique());
    let protocol_fee_token = anchor_pubkey(Address::new_unique());
    install_token_account(
        &mut covered.fixture.svm,
        recipient_token,
        covered.fixture.mint,
        covered.actor,
        0,
    );
    install_token_account(
        &mut covered.fixture.svm,
        protocol_fee_token,
        covered.fixture.mint,
        covered.actor,
        0,
    );
    let fulfill = covered.fixture.program_instruction(
        zkp2p_solana::accounts::FulfillIntent {
            caller: covered.actor,
            owner_rent: covered.actor,
            orchestrator: covered.fixture.orchestrator,
            verifier: covered.fixture.verifier,
            intent: intent_key,
            deposit: covered.deposit,
            intent_lock,
            taker_state,
            payment_binding,
            intent_payment_binding,
            tokens: zkp2p_solana::accounts::SettlementTokenAccounts {
                mint: covered.fixture.mint,
                deposit_vault: covered.deposit_vault,
                recipient_token,
                protocol_fee_token,
                manager_fee_token: None,
                token_program: anchor_spl::token::ID,
            },
            dispute: zkp2p_solana::accounts::SettlementDisputeAccounts {
                dispute_intent: Some(dispute_intent),
                stake_vault: Some(covered.fixture.stake_vault),
                stake_position: Some(covered.stake_position),
                stake_lock: Some(stake_lock),
            },
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::FulfillIntent {
            args: FulfillIntentArgs {
                attestation,
                expected_nullifier: payment_nullifier,
            },
        },
    );
    let fulfill_compute = send_with_compute(
        &mut covered.fixture.svm,
        &covered.fixture.authority,
        &[],
        &[fulfill],
    )
    .expect("fulfill threshold payment");
    eprintln!("fulfill_compute_units={fulfill_compute}");
    assert_eq!(token_amount(&covered.fixture.svm, recipient_token), 99);

    let details = DisputeDetails {
        payment_method: covered.payment_method_id,
        original_payment_id: [31_u8; 32],
        dispute_id: [32_u8; 32],
        payment_amount: 100,
        payment_currency: covered.currency_id,
    };
    let mut dispute_payload = Vec::new();
    details
        .serialize(&mut dispute_payload)
        .expect("serialize dispute details");
    let dispute_data_hash = solana_keccak_hasher::hash(&dispute_payload).to_bytes();
    let dispute_digest = dispute_attestation_digest(
        covered.fixture.dispute_config,
        intent_hash,
        dispute_data_hash,
    );
    let dispute_nullifier =
        solana_keccak_hasher::hashv(&[&covered.payment_method_id, &[32_u8; 32]]).to_bytes();
    let dispute_nullifier_account = pda(&[
        DISPUTE_NULLIFIER_SEED,
        covered.fixture.dispute_config.as_ref(),
        &dispute_nullifier,
    ]);
    let claim = pda(&[
        zkp2p_solana::constants::CLAIM_BALANCE_SEED,
        covered.fixture.stake_vault.as_ref(),
        covered.actor.as_ref(),
    ]);
    let submit = covered.fixture.program_instruction(
        zkp2p_solana::accounts::SubmitDispute {
            caller: covered.actor,
            dispute_config: covered.fixture.dispute_config,
            verifier: covered.fixture.verifier,
            dispute_intent,
            payment_binding,
            intent_payment_binding,
            dispute_nullifier: dispute_nullifier_account,
            stake_vault: covered.fixture.stake_vault,
            position: covered.stake_position,
            stake_lock,
            claim,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::SubmitDispute {
            args: SubmitDisputeArgs {
                attestation: DisputeAttestation {
                    intent_hash,
                    data_hash: dispute_data_hash,
                    signatures: vec![sign_digest(&signing_key, dispute_digest)],
                    details,
                },
                expected_payment_nullifier: payment_nullifier,
                expected_dispute_nullifier: dispute_nullifier,
            },
        },
    );
    let dispute_compute = send_with_compute(
        &mut covered.fixture.svm,
        &covered.fixture.authority,
        &[],
        &[submit],
    )
    .expect("resolve signed dispute");
    eprintln!("submit_dispute_compute_units={dispute_compute}");
    let claim_state: ClaimBalance = decode(&covered.fixture, claim);
    assert_eq!(claim_state.amount, 100);
}
