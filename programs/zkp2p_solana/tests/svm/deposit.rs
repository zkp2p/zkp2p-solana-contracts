//! Real-SBF EscrowV2 custody and deposit-policy transitions.

use super::common::{
    address, anchor_pubkey, install_token_account, pda, set_token_amount, token_amount, Fixture,
};
use anchor_lang::AccountDeserialize;
use solana_address::Address;
use solana_signer::Signer;
use zkp2p_solana::{
    constants::{
        ADDRESS_GROUP_SEED, DEPOSIT_CURRENCY_SEED, DEPOSIT_DISPUTE_SETTING_SEED, DEPOSIT_SEED,
        DEPOSIT_VAULT_SEED, DEPOSIT_WHITELIST_MEMBER_SEED, DEPOSIT_WHITELIST_SEED,
        PAYMENT_METHOD_SEED, RATE_MANAGER_SEED,
    },
    AmountRange, ConfigureCurrencyArgs, ConfigureEscrowArgs, ConfigurePaymentMethodArgs,
    CreateDepositArgs, CreateRateManagerArgs, Deposit, DepositDisputeSetting, DepositWhitelist,
    DepositWhitelistMember, UpdateDepositArgs,
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
fn deposit_custody_configuration_whitelist_and_dispute_setting_round_trip() {
    let mut fixture = Fixture::new();
    let depositor = anchor_pubkey(fixture.authority.pubkey());
    let depositor_token = anchor_pubkey(Address::new_unique());
    let dust_recipient_token = anchor_pubkey(Address::new_unique());
    install_token_account(
        &mut fixture.svm,
        depositor_token,
        fixture.mint,
        depositor,
        1_000,
    );
    install_token_account(
        &mut fixture.svm,
        dust_recipient_token,
        fixture.mint,
        depositor,
        0,
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
                amount: 600,
                intent_amount_range: AmountRange { min: 10, max: 300 },
                delegate: None,
                intent_guardian: Some(depositor),
                retain_on_empty: false,
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
    fixture.send(&[create]).expect("create funded deposit");
    assert_eq!(token_amount(&fixture.svm, deposit_vault), 600);
    assert_eq!(token_amount(&fixture.svm, depositor_token), 400);

    let add = fixture.program_instruction(
        zkp2p_solana::accounts::AddFunds {
            funder: depositor,
            escrow_config: fixture.escrow,
            deposit,
            token_mint: fixture.mint,
            funder_token: depositor_token,
            deposit_vault,
            token_program: anchor_spl::token::ID,
        },
        zkp2p_solana::instruction::AddFunds { amount: 100 },
    );
    let remove = fixture.program_instruction(
        zkp2p_solana::accounts::RemoveFunds {
            depositor,
            escrow_config: fixture.escrow,
            deposit,
            token_mint: fixture.mint,
            deposit_vault,
            depositor_token,
            token_program: anchor_spl::token::ID,
        },
        zkp2p_solana::instruction::RemoveFunds { amount: 150 },
    );
    fixture
        .send(&[add, remove])
        .expect("add and remove exact funds");
    assert_eq!(token_amount(&fixture.svm, deposit_vault), 550);

    let delegate = anchor_lang::prelude::Pubkey::new_unique();
    let update = fixture.program_instruction(
        zkp2p_solana::accounts::UpdateDeposit {
            authority: depositor,
            escrow_config: fixture.escrow,
            deposit,
        },
        zkp2p_solana::instruction::UpdateDeposit {
            args: UpdateDepositArgs {
                delegate: Some(Some(delegate)),
                intent_guardian: Some(None),
                intent_amount_range: Some(AmountRange { min: 20, max: 250 }),
                accepting_intents: Some(false),
                retain_on_empty: Some(true),
            },
        },
    );
    fixture.send(&[update]).expect("update deposit settings");

    let second_method_id = [4_u8; 32];
    let second_currency_id = [5_u8; 32];
    let second_method = pda(&[PAYMENT_METHOD_SEED, deposit.as_ref(), &second_method_id]);
    let second_currency = pda(&[
        DEPOSIT_CURRENCY_SEED,
        deposit.as_ref(),
        &second_method_id,
        &second_currency_id,
    ]);
    let configure_method = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigurePaymentMethod {
            authority: depositor,
            escrow_config: fixture.escrow,
            deposit,
            payment_method: second_method,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::ConfigurePaymentMethod {
            args: ConfigurePaymentMethodArgs {
                payment_method: second_method_id,
                payee_details: [6_u8; 32],
                gating_service: None,
                active: true,
            },
        },
    );
    let configure_currency = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigureCurrency {
            authority: depositor,
            escrow_config: fixture.escrow,
            deposit,
            payment_method: second_method,
            deposit_currency: second_currency,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::ConfigureCurrency {
            args: ConfigureCurrencyArgs {
                payment_method: second_method_id,
                currency: second_currency_id,
                fixed_min_rate: 1_100_000_000_000_000_000,
                oracle_quote: None,
                spread_bps: 25,
                max_staleness: 0,
                listed: true,
            },
        },
    );
    fixture
        .send(&[configure_method, configure_currency])
        .expect("configure another method and currency");

    let whitelist = pda(&[DEPOSIT_WHITELIST_SEED, deposit.as_ref()]);
    let initialize_whitelist = fixture.program_instruction(
        zkp2p_solana::accounts::InitializeDepositWhitelist {
            authority: depositor,
            deposit,
            deposit_whitelist: whitelist,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::InitializeDepositWhitelist { enabled: true },
    );
    fixture
        .send(&[initialize_whitelist])
        .expect("initialize deposit whitelist");

    let group = pda(&[
        ADDRESS_GROUP_SEED,
        fixture.whitelist_config.as_ref(),
        &0_u64.to_le_bytes(),
    ]);
    let create_group = fixture.program_instruction(
        zkp2p_solana::accounts::CreateAddressGroup {
            curator: depositor,
            whitelist_config: fixture.whitelist_config,
            group,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::CreateAddressGroup {
            name: "deposit group".into(),
            public: false,
        },
    );
    fixture.send(&[create_group]).expect("create deposit group");
    let set_group = fixture.program_instruction(
        zkp2p_solana::accounts::SetDepositAllowedGroup {
            authority: depositor,
            deposit,
            deposit_whitelist: whitelist,
            group,
        },
        zkp2p_solana::instruction::SetDepositAllowedGroup { allowed: true },
    );
    let taker = anchor_lang::prelude::Pubkey::new_unique();
    let direct_member = pda(&[
        DEPOSIT_WHITELIST_MEMBER_SEED,
        whitelist.as_ref(),
        taker.as_ref(),
    ]);
    let set_member = fixture.program_instruction(
        zkp2p_solana::accounts::SetDepositWhitelistMember {
            authority: depositor,
            deposit,
            deposit_whitelist: whitelist,
            taker,
            member: direct_member,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::SetDepositWhitelistMember { active: true },
    );
    let disable_whitelist = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigureDepositWhitelist {
            authority: depositor,
            deposit,
            deposit_whitelist: whitelist,
        },
        zkp2p_solana::instruction::SetWhitelistEnabled { enabled: false },
    );
    fixture
        .send(&[set_group, set_member, disable_whitelist])
        .expect("configure deposit whitelist");

    let dispute_setting = pda(&[DEPOSIT_DISPUTE_SETTING_SEED, deposit.as_ref()]);
    let set_dispute = fixture.program_instruction(
        zkp2p_solana::accounts::SetDepositDisputeProtection {
            authority: depositor,
            deposit,
            setting: dispute_setting,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::SetDepositDisputeProtection { enabled: false },
    );
    fixture.send(&[set_dispute]).expect("opt out of disputes");

    let state: Deposit = decode(&fixture, deposit);
    assert_eq!(state.delegate, Some(delegate));
    assert_eq!(state.remaining_deposits, 550);
    assert!(!state.accepting_intents);
    let whitelist_state: DepositWhitelist = decode(&fixture, whitelist);
    assert!(!whitelist_state.enabled);
    assert_eq!(whitelist_state.allowed_groups.len(), 1);
    let member_state: DepositWhitelistMember = decode(&fixture, direct_member);
    assert!(member_state.active);
    let setting: DepositDisputeSetting = decode(&fixture, dispute_setting);
    assert!(!setting.enabled);

    let rate_manager = pda(&[
        RATE_MANAGER_SEED,
        fixture.rate_manager_config.as_ref(),
        &0_u64.to_le_bytes(),
    ]);
    let create_manager = fixture.program_instruction(
        zkp2p_solana::accounts::CreateRateManager {
            payer: depositor,
            config: fixture.rate_manager_config,
            rate_manager,
            system_program: anchor_lang::system_program::ID,
        },
        zkp2p_solana::instruction::CreateRateManager {
            args: CreateRateManagerArgs {
                manager: depositor,
                fee_recipient: None,
                max_fee: 0,
                fee: 0,
                min_liquidity: 500,
                name: "deposit manager".into(),
                uri: String::new(),
            },
        },
    );
    fixture.send(&[create_manager]).expect("create manager");
    let select_manager = fixture.program_instruction(
        zkp2p_solana::accounts::SetDepositRateManager {
            authority: depositor,
            escrow_config: fixture.escrow,
            deposit,
            rate_manager: Some(rate_manager),
        },
        zkp2p_solana::instruction::SetDepositRateManager {
            manager: Some(rate_manager),
        },
    );
    fixture
        .send(&[select_manager])
        .expect("select delegated manager");
    let clear_manager = fixture.program_instruction(
        zkp2p_solana::accounts::SetDepositRateManager {
            authority: depositor,
            escrow_config: fixture.escrow,
            deposit,
            rate_manager: None,
        },
        zkp2p_solana::instruction::SetDepositRateManager { manager: None },
    );
    fixture
        .send(&[clear_manager])
        .expect("clear delegated manager");

    let allow_terminal_close = fixture.program_instruction(
        zkp2p_solana::accounts::UpdateDeposit {
            authority: depositor,
            escrow_config: fixture.escrow,
            deposit,
        },
        zkp2p_solana::instruction::UpdateDeposit {
            args: UpdateDepositArgs {
                delegate: None,
                intent_guardian: None,
                intent_amount_range: None,
                accepting_intents: None,
                retain_on_empty: Some(false),
            },
        },
    );
    let configure_dust_and_pause = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigureEscrow {
            authority: depositor,
            protocol: fixture.protocol,
            escrow: fixture.escrow,
        },
        zkp2p_solana::instruction::ConfigureEscrow {
            args: ConfigureEscrowArgs {
                dust_recipient: Some(depositor),
                dust_threshold: Some(1),
                max_intents_per_deposit: None,
                intent_expiration_period: None,
                paused: Some(true),
            },
        },
    );
    fixture
        .send(&[allow_terminal_close, configure_dust_and_pause])
        .expect("prepare paused dust exit");
    set_token_amount(&mut fixture.svm, deposit_vault, 551);

    let withdraw = fixture.program_instruction(
        zkp2p_solana::accounts::WithdrawDeposit {
            depositor,
            escrow_config: fixture.escrow,
            deposit,
            token_mint: fixture.mint,
            deposit_vault,
            depositor_token,
            dust_recipient_token,
            token_program: anchor_spl::token::ID,
        },
        zkp2p_solana::instruction::WithdrawDeposit {},
    );
    fixture.send(&[withdraw]).expect("withdraw deposit");
    assert_eq!(token_amount(&fixture.svm, depositor_token), 1_000);
    assert_eq!(token_amount(&fixture.svm, dust_recipient_token), 1);
    assert!(fixture
        .svm
        .get_account(&address(deposit.to_bytes()))
        .is_none());
    assert!(fixture
        .svm
        .get_account(&address(deposit_vault.to_bytes()))
        .is_none());
}

#[test]
fn above_threshold_dust_never_blocks_paused_principal_exit() {
    let mut fixture = Fixture::new();
    let depositor = anchor_pubkey(fixture.authority.pubkey());
    let depositor_token = anchor_pubkey(Address::new_unique());
    let dust_recipient_token = anchor_pubkey(Address::new_unique());
    install_token_account(
        &mut fixture.svm,
        depositor_token,
        fixture.mint,
        depositor,
        1_000,
    );
    install_token_account(
        &mut fixture.svm,
        dust_recipient_token,
        fixture.mint,
        depositor,
        0,
    );

    let deposit = pda(&[DEPOSIT_SEED, fixture.escrow.as_ref(), &0_u64.to_le_bytes()]);
    let payment_method_id = [81_u8; 32];
    let currency_id = [82_u8; 32];
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
                amount: 100,
                intent_amount_range: AmountRange { min: 10, max: 100 },
                delegate: None,
                intent_guardian: None,
                retain_on_empty: false,
                payment_method: payment_method_id,
                payee_details: [83_u8; 32],
                gating_service: None,
                currency: currency_id,
                fixed_min_rate: 1_000_000_000_000_000_000,
                oracle_quote: None,
                spread_bps: 0,
                max_staleness: 0,
            },
        },
    );
    fixture.send(&[create]).expect("create dust target");
    let pause = fixture.program_instruction(
        zkp2p_solana::accounts::ConfigureEscrow {
            authority: depositor,
            protocol: fixture.protocol,
            escrow: fixture.escrow,
        },
        zkp2p_solana::instruction::ConfigureEscrow {
            args: ConfigureEscrowArgs {
                dust_recipient: None,
                dust_threshold: Some(0),
                max_intents_per_deposit: None,
                intent_expiration_period: None,
                paused: Some(true),
            },
        },
    );
    fixture.send(&[pause]).expect("pause admissions");

    // Model an unsolicited canonical-token transfer that is not part of program accounting.
    set_token_amount(&mut fixture.svm, deposit_vault, 101);
    let withdraw = fixture.program_instruction(
        zkp2p_solana::accounts::WithdrawDeposit {
            depositor,
            escrow_config: fixture.escrow,
            deposit,
            token_mint: fixture.mint,
            deposit_vault,
            depositor_token,
            dust_recipient_token,
            token_program: anchor_spl::token::ID,
        },
        zkp2p_solana::instruction::WithdrawDeposit {},
    );
    fixture.send(&[withdraw]).expect("paused principal exit");

    assert_eq!(token_amount(&fixture.svm, depositor_token), 1_000);
    assert_eq!(token_amount(&fixture.svm, deposit_vault), 1);
    assert_eq!(token_amount(&fixture.svm, dust_recipient_token), 0);
    let deposit_state: Deposit = decode(&fixture, deposit);
    assert_eq!(deposit_state.remaining_deposits, 0);
    assert!(deposit_state.retain_on_empty);
    assert!(!deposit_state.accepting_intents);
}
