//! Real-SBF governance, verifier, rate-manager, oracle, and address-group transitions.

use super::common::{address, anchor_pubkey, pda, send, Fixture};
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_signer::Signer;
use zkp2p_solana::{
    constants::{
        ADDRESS_GROUP_SEED, ORACLE_QUOTE_SEED, RATE_ENTRY_SEED, RATE_MANAGER_SEED, RISK_WINDOW_SEED,
    },
    AddressGroup, ConfigureEscrowArgs, ConfigureOrchestratorArgs, CreateRateManagerArgs,
    EscrowConfig, LifecyclePolicy, OrchestratorConfig, RateEntry, RateManager, VerifierConfig,
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
fn governs_latest_configs_and_verifier() {
    let mut fixture = Fixture::new();
    let recipient = anchor_lang::prelude::Pubkey::new_unique();
    let configure_orchestrator = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigureOrchestrator {
            authority: anchor_pubkey(fixture.authority.pubkey()),
            protocol: fixture.protocol,
            orchestrator: fixture.orchestrator,
        },
        zkp2p_solana::instruction::ConfigureOrchestrator {
            args: ConfigureOrchestratorArgs {
                protocol_fee: Some(20_000_000_000_000_000),
                protocol_fee_recipient: Some(recipient),
                lifecycle_policy: Some(LifecyclePolicy::Whitelist),
                allow_multiple_intents: Some(true),
                paused: Some(true),
            },
        },
    );
    let configure_escrow = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigureEscrow {
            authority: anchor_pubkey(fixture.authority.pubkey()),
            protocol: fixture.protocol,
            escrow: fixture.escrow,
        },
        zkp2p_solana::instruction::ConfigureEscrow {
            args: ConfigureEscrowArgs {
                dust_recipient: Some(recipient),
                dust_threshold: Some(17),
                max_intents_per_deposit: Some(12),
                intent_expiration_period: Some(900),
                paused: Some(true),
            },
        },
    );
    fixture
        .send(&[configure_orchestrator, configure_escrow])
        .expect("govern configs");

    let method = [3_u8; 32];
    let add_method = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigureVerifier {
            authority: anchor_pubkey(fixture.authority.pubkey()),
            protocol: fixture.protocol,
            verifier: fixture.verifier,
        },
        zkp2p_solana::instruction::SetVerifierPaymentMethod {
            payment_method: method,
            enabled: true,
        },
    );
    let add_witness = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigureVerifier {
            authority: anchor_pubkey(fixture.authority.pubkey()),
            protocol: fixture.protocol,
            verifier: fixture.verifier,
        },
        zkp2p_solana::instruction::SetVerifierWitness {
            witness: [8_u8; 20],
            enabled: true,
        },
    );
    let threshold = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigureVerifier {
            authority: anchor_pubkey(fixture.authority.pubkey()),
            protocol: fixture.protocol,
            verifier: fixture.verifier,
        },
        zkp2p_solana::instruction::SetRequiredSignatures { required: 2 },
    );
    fixture
        .send(&[add_method, add_witness, threshold])
        .expect("govern verifier");

    let orchestrator: OrchestratorConfig = decode(&fixture, fixture.orchestrator);
    assert!(orchestrator.paused);
    assert_eq!(orchestrator.lifecycle_policy, LifecyclePolicy::Whitelist);
    assert_eq!(orchestrator.protocol_fee_recipient, recipient);
    let escrow: EscrowConfig = decode(&fixture, fixture.escrow);
    assert!(escrow.paused);
    assert_eq!(escrow.intent_expiration_period, 900);
    let verifier: VerifierConfig = decode(&fixture, fixture.verifier);
    assert_eq!(verifier.required_signatures, 2);
    assert!(verifier.payment_methods.contains(&method));
    assert!(verifier.witnesses.contains(&[8_u8; 20]));
}

#[test]
fn creates_and_updates_rate_manager_and_oracle_entries() {
    let mut fixture = Fixture::new();
    let manager = Keypair::new();
    fixture.fund(&manager);
    let manager_key = anchor_pubkey(manager.pubkey());
    let rate_manager = pda(&[
        RATE_MANAGER_SEED,
        fixture.rate_manager_config.as_ref(),
        &0_u64.to_le_bytes(),
    ]);
    let create = fixture.program_instruction(
        zkp2p_solana::accounts::CreateRateManager {
            payer: anchor_pubkey(fixture.authority.pubkey()),
            config: fixture.rate_manager_config,
            rate_manager,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::CreateRateManager {
            args: CreateRateManagerArgs {
                manager: manager_key,
                fee_recipient: Some(manager_key),
                max_fee: 50_000_000_000_000_000,
                fee: 10_000_000_000_000_000,
                min_liquidity: 100,
                name: "fixture manager".into(),
                uri: "https://example.invalid/rates".into(),
            },
        },
    );
    fixture.send(&[create]).expect("create rate manager");

    let manage_accounts = zkp2p_solana::accounts::ManageRateManager {
        manager: manager_key,
        rate_manager,
    };
    let configure = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: manage_accounts.to_account_metas(None),
        data: zkp2p_solana::instruction::SetRateManagerConfig {
            manager: manager_key,
            fee_recipient: Some(manager_key),
            name: "updated".into(),
            uri: "https://example.invalid/updated".into(),
        }
        .data(),
    };
    let set_fee = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: manage_accounts.to_account_metas(None),
        data: zkp2p_solana::instruction::SetManagerFee {
            fee: 20_000_000_000_000_000,
        }
        .data(),
    };
    let set_floor = Instruction {
        program_id: address(zkp2p_solana::ID.to_bytes()),
        accounts: manage_accounts.to_account_metas(None),
        data: zkp2p_solana::instruction::SetManagerMinLiquidity { min_liquidity: 500 }.data(),
    };
    send(
        &mut fixture.svm,
        &manager,
        &[],
        &[configure, set_fee, set_floor],
    )
    .expect("update rate manager");

    let payment_method = [4_u8; 32];
    let currency = [5_u8; 32];
    let rate_entry = pda(&[
        RATE_ENTRY_SEED,
        rate_manager.as_ref(),
        &payment_method,
        &currency,
    ]);
    let set_rate = fixture.program_instruction(
        zkp2p_solana::accounts::SetManagerRate {
            manager: manager_key,
            rate_manager,
            rate_entry,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::SetManagerRate {
            payment_method,
            currency,
            rate: 1_250_000_000_000_000_000,
        },
    );
    send(&mut fixture.svm, &manager, &[], &[set_rate]).expect("set manager rate");

    let quote_id = [6_u8; 32];
    let oracle_quote = pda(&[ORACLE_QUOTE_SEED, manager_key.as_ref(), &quote_id]);
    let quote = fixture.program_instruction(
        zkp2p_solana::accounts::UpdateOracleQuote {
            authority: manager_key,
            oracle_quote,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::UpdateOracleQuote {
            quote_id,
            market_rate: 1_000_000_000_000_000_000,
            valid: true,
        },
    );
    send(&mut fixture.svm, &manager, &[], &[quote]).expect("set oracle quote");

    let state: RateManager = decode(&fixture, rate_manager);
    assert_eq!(state.fee, 20_000_000_000_000_000);
    assert_eq!(state.min_liquidity, 500);
    let entry: RateEntry = decode(&fixture, rate_entry);
    assert_eq!(entry.rate, 1_250_000_000_000_000_000);
}

#[test]
fn manages_address_groups_dispute_windows_and_delayed_controller() {
    let mut fixture = Fixture::new();
    let group = pda(&[
        ADDRESS_GROUP_SEED,
        fixture.whitelist_config.as_ref(),
        &0_u64.to_le_bytes(),
    ]);
    let create_group = fixture.program_instruction(
        zkp2p_solana::accounts::CreateAddressGroup {
            curator: anchor_pubkey(fixture.authority.pubkey()),
            whitelist_config: fixture.whitelist_config,
            group,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::CreateAddressGroup {
            name: "public takers".into(),
            public: true,
        },
    );
    fixture.send(&[create_group]).expect("create group");

    let member = Keypair::new();
    fixture.fund(&member);
    let member_key = anchor_pubkey(member.pubkey());
    let member_pda = pda(&[
        zkp2p_solana::constants::GROUP_MEMBER_SEED,
        group.as_ref(),
        member_key.as_ref(),
    ]);
    let set_member = fixture.program_instruction(
        zkp2p_solana::accounts::SetGroupMember {
            curator: anchor_pubkey(fixture.authority.pubkey()),
            group,
            member_address: member_key,
            member: member_pda,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::SetGroupMember { active: true },
    );
    fixture.send(&[set_member]).expect("curator adds member");
    let self_membership = fixture.program_instruction(
        zkp2p_solana::accounts::SetSelfGroupMember {
            member_address: member_key,
            group,
            member: member_pda,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::SetSelfGroupMember { active: false },
    );
    send(&mut fixture.svm, &member, &[], &[self_membership]).expect("member leaves group");

    let pending_curator = Keypair::new();
    fixture.fund(&pending_curator);
    let pending_key = anchor_pubkey(pending_curator.pubkey());
    let configure_group = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigureAddressGroup {
            curator: anchor_pubkey(fixture.authority.pubkey()),
            group,
            resolver_program: None,
        },
        zkp2p_solana::instruction::ConfigureAddressGroup {
            public: Some(false),
            resolver: None,
            pending_curator: Some(Some(pending_key)),
        },
    );
    fixture
        .send(&[configure_group])
        .expect("configure address group");
    let accept_group = fixture.program_instruction(
        zkp2p_solana::accounts::AcceptGroupCurator {
            pending_curator: pending_key,
            group,
        },
        zkp2p_solana::instruction::AcceptGroupCurator {},
    );
    send(&mut fixture.svm, &pending_curator, &[], &[accept_group]).expect("accept group curator");
    let group_state: AddressGroup = decode(&fixture, group);
    assert_eq!(group_state.curator, pending_key);
    assert!(!group_state.public);

    let method = [9_u8; 32];
    let risk_window = pda(&[RISK_WINDOW_SEED, fixture.dispute_config.as_ref(), &method]);
    let set_risk = fixture.program_instruction(
        zkp2p_solana::accounts::SetRiskWindow {
            authority: anchor_pubkey(fixture.authority.pubkey()),
            protocol: fixture.protocol,
            dispute_config: fixture.dispute_config,
            risk_window,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::SetRiskWindow {
            payment_method: method,
            seconds: 7_200,
        },
    );
    let pause_disputes = fixture.program_instruction(
        zkp2p_solana::accounts::SetDisputeAdmissionsPaused {
            authority: anchor_pubkey(fixture.authority.pubkey()),
            protocol: fixture.protocol,
            dispute_config: fixture.dispute_config,
        },
        zkp2p_solana::instruction::SetDisputeAdmissionsPaused { paused: true },
    );
    fixture
        .send(&[set_risk, pause_disputes])
        .expect("configure disputes");

    let pending_controller = Keypair::new();
    fixture.fund(&pending_controller);
    let pending_controller_key = anchor_pubkey(pending_controller.pubkey());
    let propose = fixture.program_instruction(
        zkp2p_solana::accounts::ProposeStakeController {
            authority: anchor_pubkey(fixture.authority.pubkey()),
            protocol: fixture.protocol,
            vault: fixture.stake_vault,
        },
        zkp2p_solana::instruction::ProposeStakeController {
            pending: Some(pending_controller_key),
        },
    );
    fixture.send(&[propose]).expect("propose stake controller");
    let mut clock = fixture.svm.get_sysvar::<anchor_lang::prelude::Clock>();
    clock.unix_timestamp = clock
        .unix_timestamp
        .checked_add(86_401)
        .expect("fixture clock fits");
    fixture.svm.set_sysvar(&clock);
    let accept = fixture.program_instruction(
        zkp2p_solana::accounts::AcceptStakeController {
            pending_controller: pending_controller_key,
            vault: fixture.stake_vault,
        },
        zkp2p_solana::instruction::AcceptStakeController {},
    );
    send(&mut fixture.svm, &pending_controller, &[], &[accept]).expect("accept mature controller");
}
