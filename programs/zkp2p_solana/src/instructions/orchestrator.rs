//! OrchestratorV3-equivalent intent admission and cancellation.

use anchor_lang::prelude::*;
use solana_keccak_hasher as keccak;
use solana_program::sysvar::instructions::{
    load_current_index_checked, load_instruction_at_checked,
};

use crate::{
    constants::{
        ADDRESS_GROUP_SEED, DEPOSIT_CURRENCY_SEED, DEPOSIT_DISPUTE_SETTING_SEED,
        DEPOSIT_WHITELIST_MEMBER_SEED, DEPOSIT_WHITELIST_SEED, DISPUTE_CONFIG_SEED,
        DISPUTE_INTENT_SEED, ESCROW_INTENT_LOCK_SEED, GROUP_MEMBER_SEED, INTENT_SEED, MAX_FEE,
        MAX_REFERRAL_FEE, PAYMENT_METHOD_SEED, RISK_WINDOW_SEED, STAKE_LOCK_SEED,
        STAKE_POSITION_SEED, STAKE_VAULT_CONFIG_SEED, TAKER_INTENT_STATE_SEED,
    },
    error::Zkp2pError,
    state::{
        AddressGroup, Deposit, DepositCurrency, DepositDisputeSetting, DepositPaymentMethod,
        DepositWhitelist, DepositWhitelistMember, DisputeConfig, DisputeIntent, DisputeStatus,
        EscrowConfig, EscrowIntentLock, GroupMember, Intent, LifecyclePolicy, OracleQuote,
        OrchestratorConfig, RateEntry, RateManager, ReferralFee, RiskWindow, StakeLock,
        StakePosition, StakeVaultConfig, TakerIntentState,
    },
};

/// Canonical fields supplied when signaling an intent.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct SignalIntentArgs {
    /// Client-derived hash, cross-checked against the serialized orchestrator counter.
    pub expected_intent_hash: [u8; 32],
    /// Principal to lock.
    pub amount: u64,
    /// Final token recipient.
    pub recipient: Pubkey,
    /// Active payment method.
    pub payment_method: [u8; 32],
    /// Listed fiat currency.
    pub fiat_currency: [u8; 32],
    /// Agreed conversion rate.
    pub conversion_rate: u128,
    /// Ordered fee recipients and precise-unit rates.
    pub referral_fees: Vec<ReferralFee>,
    /// Expiration for a configured native gating signature.
    pub gating_signature_expiration: i64,
}

/// Accounts for atomically signaling an intent and locking escrow liquidity.
#[derive(Accounts)]
#[instruction(args: SignalIntentArgs)]
pub struct SignalIntent<'info> {
    /// Intent owner and account rent payer.
    #[account(mut)]
    pub taker: Signer<'info>,
    /// Canonical orchestrator configuration.
    #[account(mut, seeds = [crate::constants::ORCHESTRATOR_CONFIG_SEED], bump = orchestrator.bump)]
    pub orchestrator: Box<Account<'info, OrchestratorConfig>>,
    /// Canonical escrow configuration.
    #[account(
        seeds = [crate::constants::ESCROW_CONFIG_SEED],
        bump = escrow_config.bump,
        constraint = orchestrator.escrow_config == escrow_config.key() @ Zkp2pError::Unauthorized
    )]
    pub escrow_config: Box<Account<'info, EscrowConfig>>,
    /// Selected maker deposit.
    #[account(mut, constraint = deposit.escrow_config == escrow_config.key() @ Zkp2pError::DepositNotFound)]
    pub deposit: Box<Account<'info, Deposit>>,
    /// Active payment method.
    #[account(
        seeds = [PAYMENT_METHOD_SEED, deposit.key().as_ref(), &args.payment_method],
        bump = payment_method.bump,
        constraint = payment_method.deposit == deposit.key() @ Zkp2pError::PaymentMethodNotSupported
    )]
    pub payment_method: Box<Account<'info, DepositPaymentMethod>>,
    /// Listed currency configuration.
    #[account(
        seeds = [DEPOSIT_CURRENCY_SEED, deposit.key().as_ref(), &args.payment_method, &args.fiat_currency],
        bump = deposit_currency.bump,
        constraint = deposit_currency.deposit == deposit.key() @ Zkp2pError::CurrencyNotSupported
    )]
    pub deposit_currency: Box<Account<'info, DepositCurrency>>,
    /// Oracle quote required when the currency is oracle-backed.
    pub oracle_quote: Option<Box<Account<'info, OracleQuote>>>,
    /// Selected delegated manager, when configured by the deposit.
    pub rate_manager: Option<Box<Account<'info, RateManager>>>,
    /// Selected manager's exact payment/currency rate.
    pub rate_entry: Option<Box<Account<'info, RateEntry>>>,
    /// Persistent whitelist policy required by whitelist lifecycle modes.
    pub deposit_whitelist: Option<Box<Account<'info, DepositWhitelist>>>,
    /// Optional direct whitelist membership for this taker.
    pub direct_whitelist_member: Option<Box<Account<'info, DepositWhitelistMember>>>,
    /// Optional allowed address group used for admission.
    pub allowed_group: Option<Box<Account<'info, AddressGroup>>>,
    /// Optional explicit membership in the supplied allowed group.
    pub group_member: Option<Box<Account<'info, GroupMember>>>,
    /// Optional resolver program configured by the supplied group.
    /// CHECK: Exact program ID and executable status are checked before CPI.
    pub resolver_program: Option<UncheckedAccount<'info>>,
    /// Prepared dispute admission required by dispute lifecycle mode.
    pub dispute_intent: Option<Box<Account<'info, DisputeIntent>>>,
    /// Optional explicit deposit dispute-protection setting; absence means enabled.
    pub deposit_dispute_setting: Option<Box<Account<'info, DepositDisputeSetting>>>,
    /// Method risk window required by the dispute lifecycle mode.
    pub risk_window: Option<Box<Account<'info, RiskWindow>>>,
    /// New canonical intent PDA.
    #[account(
        init,
        payer = taker,
        space = 8 + Intent::INIT_SPACE,
        seeds = [INTENT_SEED, orchestrator.key().as_ref(), &orchestrator.next_intent_id.to_le_bytes()],
        bump
    )]
    pub intent: Box<Account<'info, Intent>>,
    /// Paired escrow lock PDA.
    #[account(
        init,
        payer = taker,
        space = 8 + EscrowIntentLock::INIT_SPACE,
        seeds = [ESCROW_INTENT_LOCK_SEED, deposit.key().as_ref(), &args.expected_intent_hash],
        bump
    )]
    pub intent_lock: Box<Account<'info, EscrowIntentLock>>,
    /// Persistent active-intent counter for the taker.
    #[account(
        init_if_needed,
        payer = taker,
        space = 8 + TakerIntentState::INIT_SPACE,
        seeds = [TAKER_INTENT_STATE_SEED, orchestrator.key().as_ref(), taker.key().as_ref()],
        bump
    )]
    pub taker_state: Box<Account<'info, TakerIntentState>>,
    /// Transaction instruction sysvar used to attest native Ed25519 verification.
    /// CHECK: The fixed address is enforced and only instruction data is read.
    #[account(address = solana_program::sysvar::instructions::ID)]
    pub instructions_sysvar: UncheckedAccount<'info>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Signals one fully snapshotted intent and creates its escrow lock atomically.
pub fn handle_signal_intent<'info>(
    ctx: Context<'info, SignalIntent<'info>>,
    args: SignalIntentArgs,
) -> Result<()> {
    require!(!ctx.accounts.orchestrator.paused, Zkp2pError::Paused);
    require!(!ctx.accounts.escrow_config.paused, Zkp2pError::Paused);
    require!(args.recipient != Pubkey::default(), Zkp2pError::ZeroAddress);
    require!(
        ctx.accounts.payment_method.active,
        Zkp2pError::PaymentMethodNotSupported
    );
    require!(
        ctx.accounts.deposit_currency.listed,
        Zkp2pError::CurrencyNotSupported
    );
    validate_referral_fees(&args.referral_fees)?;
    validate_gating_signature(
        ctx.accounts.payment_method.gating_service,
        &ctx.accounts.instructions_sysvar,
        &ctx.accounts.orchestrator,
        &ctx.accounts.deposit,
        ctx.accounts.taker.key(),
        &args,
    )?;

    if !ctx.accounts.orchestrator.allow_multiple_intents {
        require!(
            ctx.accounts.taker_state.active_intents == 0,
            Zkp2pError::AccountHasActiveIntent
        );
    }

    let nonce = ctx.accounts.orchestrator.next_intent_id;
    let intent_hash = derive_intent_hash(ctx.accounts.orchestrator.key(), nonce);
    require!(
        intent_hash == args.expected_intent_hash,
        Zkp2pError::DataHashMismatch
    );

    let floor = effective_rate(
        &ctx.accounts.deposit_currency,
        ctx.accounts.oracle_quote.as_deref(),
        ctx.accounts.deposit.rate_manager,
        ctx.accounts.rate_manager.as_deref(),
        ctx.accounts.rate_entry.as_deref(),
        Clock::get()?.unix_timestamp,
    )?;
    require!(floor > 0, Zkp2pError::CurrencyNotSupported);
    require!(args.conversion_rate >= floor, Zkp2pError::RateBelowMinimum);

    validate_lifecycle_admission(LifecycleAdmission {
        policy: ctx.accounts.orchestrator.lifecycle_policy,
        deposit: &ctx.accounts.deposit,
        whitelist: ctx.accounts.deposit_whitelist.as_deref(),
        direct_member: ctx.accounts.direct_whitelist_member.as_deref(),
        allowed_group: ctx.accounts.allowed_group.as_deref(),
        group_member: ctx.accounts.group_member.as_deref(),
        resolver_program: ctx.accounts.resolver_program.as_ref(),
        resolver_accounts: ctx.remaining_accounts,
        dispute_intent: ctx.accounts.dispute_intent.as_deref(),
        deposit_dispute_setting: ctx.accounts.deposit_dispute_setting.as_deref(),
        risk_window: ctx.accounts.risk_window.as_deref(),
        intent_hash,
        taker: ctx.accounts.taker.key(),
        payment_method: args.payment_method,
        amount: args.amount,
    })?;

    ctx.accounts.deposit.lock(
        args.amount,
        ctx.accounts.escrow_config.max_intents_per_deposit,
    )?;
    let now = Clock::get()?.unix_timestamp;
    let expiry = now
        .checked_add(ctx.accounts.escrow_config.intent_expiration_period)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;

    let manager_terms = ctx
        .accounts
        .rate_manager
        .as_ref()
        .map_or((None, 0), |manager| (manager.fee_recipient, manager.fee));
    require!(manager_terms.1 <= MAX_FEE, Zkp2pError::FeeExceedsMaximum);

    let intent = &mut ctx.accounts.intent;
    intent.orchestrator = ctx.accounts.orchestrator.key();
    intent.intent_hash = intent_hash;
    intent.nonce = nonce;
    intent.owner = ctx.accounts.taker.key();
    intent.recipient = args.recipient;
    intent.deposit = ctx.accounts.deposit.key();
    intent.amount = args.amount;
    intent.timestamp = now;
    intent.payment_method = args.payment_method;
    intent.fiat_currency = args.fiat_currency;
    intent.conversion_rate = args.conversion_rate;
    intent.payee_id = ctx.accounts.payment_method.payee_details;
    intent.manager_fee_recipient = manager_terms.0;
    intent.manager_fee = manager_terms.1;
    intent.lifecycle_policy = ctx.accounts.orchestrator.lifecycle_policy;
    intent.dispute_covered = ctx
        .accounts
        .risk_window
        .as_ref()
        .is_some_and(|window| window.seconds > 0);
    intent.referral_fees = args.referral_fees;
    intent.bump = ctx.bumps.intent;

    let lock = &mut ctx.accounts.intent_lock;
    lock.deposit = ctx.accounts.deposit.key();
    lock.intent_hash = intent_hash;
    lock.orchestrator = ctx.accounts.orchestrator.key();
    lock.amount = args.amount;
    lock.timestamp = now;
    lock.expiry_time = expiry;
    lock.bump = ctx.bumps.intent_lock;

    let taker_state = &mut ctx.accounts.taker_state;
    if taker_state.taker == Pubkey::default() {
        taker_state.orchestrator = ctx.accounts.orchestrator.key();
        taker_state.taker = ctx.accounts.taker.key();
        taker_state.bump = ctx.bumps.taker_state;
    }
    taker_state.active_intents = taker_state
        .active_intents
        .checked_add(1)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    ctx.accounts.orchestrator.next_intent_id =
        nonce.checked_add(1).ok_or(Zkp2pError::ArithmeticOverflow)?;
    Ok(())
}

/// Accounts for owner cancellation of one active intent.
#[derive(Accounts)]
pub struct CancelIntent<'info> {
    /// Original intent owner and rent recipient.
    #[account(mut, address = intent.owner)]
    pub owner: Signer<'info>,
    /// Canonical orchestrator configuration.
    #[account(seeds = [crate::constants::ORCHESTRATOR_CONFIG_SEED], bump = orchestrator.bump)]
    pub orchestrator: Box<Account<'info, OrchestratorConfig>>,
    /// Intent to remove.
    #[account(
        mut,
        close = owner,
        seeds = [INTENT_SEED, orchestrator.key().as_ref(), &intent.nonce.to_le_bytes()],
        bump = intent.bump,
        constraint = intent.orchestrator == orchestrator.key() @ Zkp2pError::IntentNotFound
    )]
    pub intent: Box<Account<'info, Intent>>,
    /// Parent deposit.
    #[account(mut, address = intent.deposit)]
    pub deposit: Box<Account<'info, Deposit>>,
    /// Paired escrow lock.
    #[account(
        mut,
        close = owner,
        seeds = [ESCROW_INTENT_LOCK_SEED, deposit.key().as_ref(), &intent.intent_hash],
        bump = intent_lock.bump,
        constraint = intent_lock.intent_hash == intent.intent_hash @ Zkp2pError::IntentNotFound,
        constraint = intent_lock.orchestrator == orchestrator.key() @ Zkp2pError::Unauthorized
    )]
    pub intent_lock: Box<Account<'info, EscrowIntentLock>>,
    /// Cancelled dispute state required when this intent had coverage.
    pub resolved_dispute: Option<Box<Account<'info, DisputeIntent>>>,
    /// Owner's active-intent counter.
    #[account(
        mut,
        seeds = [TAKER_INTENT_STATE_SEED, orchestrator.key().as_ref(), owner.key().as_ref()],
        bump = taker_state.bump,
        constraint = taker_state.taker == owner.key() @ Zkp2pError::Unauthorized
    )]
    pub taker_state: Box<Account<'info, TakerIntentState>>,
}

/// Cancels one active intent and conserves the complete locked principal.
pub fn handle_cancel_intent(ctx: Context<CancelIntent>) -> Result<()> {
    if ctx.accounts.intent.dispute_covered {
        let dispute = ctx
            .accounts
            .resolved_dispute
            .as_ref()
            .ok_or(Zkp2pError::DisputeIntentNotPending)?;
        require!(
            dispute.intent_hash == ctx.accounts.intent.intent_hash
                && dispute.status == DisputeStatus::Cancelled,
            Zkp2pError::DisputeIntentNotPending
        );
    } else {
        require!(
            ctx.accounts.resolved_dispute.is_none(),
            Zkp2pError::Unauthorized
        );
    }
    ctx.accounts
        .deposit
        .unlock(ctx.accounts.intent_lock.amount)?;
    ctx.accounts.taker_state.active_intents = ctx
        .accounts
        .taker_state
        .active_intents
        .checked_sub(1)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    Ok(())
}

/// Optional stake-backed lifecycle accounts used by permissionless expiry pruning.
#[derive(Accounts)]
pub struct PruneDisputeAccounts<'info> {
    /// Canonical dispute component.
    #[account(seeds = [DISPUTE_CONFIG_SEED], bump = dispute_config.bump)]
    pub dispute_config: Option<Box<Account<'info, DisputeConfig>>>,
    /// Covered pending lifecycle state.
    #[account(mut)]
    pub dispute_intent: Option<Box<Account<'info, DisputeIntent>>>,
    /// Canonical stake vault.
    #[account(seeds = [STAKE_VAULT_CONFIG_SEED], bump = stake_vault.bump)]
    pub stake_vault: Option<Box<Account<'info, StakeVaultConfig>>>,
    /// Selected collateral-owner position.
    #[account(mut)]
    pub stake_position: Option<Box<Account<'info, StakePosition>>>,
    /// Exact active collateral lock.
    #[account(mut)]
    pub stake_lock: Option<Box<Account<'info, StakeLock>>>,
}

/// Accounts for one permissionless, atomically complete expiry prune.
#[derive(Accounts)]
pub struct PruneExpiredIntent<'info> {
    /// Permissionless caller.
    pub caller: Signer<'info>,
    /// Original owner receives reclaimed account rent.
    #[account(mut, address = intent.owner)]
    pub owner_rent: SystemAccount<'info>,
    /// Canonical orchestrator.
    #[account(seeds = [crate::constants::ORCHESTRATOR_CONFIG_SEED], bump = orchestrator.bump)]
    pub orchestrator: Box<Account<'info, OrchestratorConfig>>,
    /// Expired active intent.
    #[account(
        mut,
        close = owner_rent,
        seeds = [INTENT_SEED, orchestrator.key().as_ref(), &intent.nonce.to_le_bytes()],
        bump = intent.bump
    )]
    pub intent: Box<Account<'info, Intent>>,
    /// Parent maker deposit.
    #[account(mut, address = intent.deposit)]
    pub deposit: Box<Account<'info, Deposit>>,
    /// Paired expired escrow lock.
    #[account(
        mut,
        close = owner_rent,
        seeds = [ESCROW_INTENT_LOCK_SEED, deposit.key().as_ref(), &intent.intent_hash],
        bump = intent_lock.bump,
        constraint = intent_lock.orchestrator == orchestrator.key() @ Zkp2pError::Unauthorized
    )]
    pub intent_lock: Box<Account<'info, EscrowIntentLock>>,
    /// Original taker's active-intent counter.
    #[account(
        mut,
        seeds = [TAKER_INTENT_STATE_SEED, orchestrator.key().as_ref(), intent.owner.as_ref()],
        bump = taker_state.bump
    )]
    pub taker_state: Box<Account<'info, TakerIntentState>>,
    /// Optional covered-dispute accounts.
    pub dispute: PruneDisputeAccounts<'info>,
}

/// Prunes one strictly expired intent while conserving escrow and collateral accounting.
pub fn handle_prune_expired_intent(ctx: Context<PruneExpiredIntent>) -> Result<()> {
    require!(
        Clock::get()?.unix_timestamp > ctx.accounts.intent_lock.expiry_time,
        Zkp2pError::IntentNotFound
    );
    if ctx.accounts.intent.dispute_covered {
        let dispute_config = ctx
            .accounts
            .dispute
            .dispute_config
            .as_deref()
            .ok_or(Zkp2pError::DisputeIntentNotPending)?;
        let dispute_intent = ctx
            .accounts
            .dispute
            .dispute_intent
            .as_deref_mut()
            .ok_or(Zkp2pError::DisputeIntentNotPending)?;
        let stake_vault = ctx
            .accounts
            .dispute
            .stake_vault
            .as_deref()
            .ok_or(Zkp2pError::DisputeIntentNotPending)?;
        let position = ctx
            .accounts
            .dispute
            .stake_position
            .as_deref_mut()
            .ok_or(Zkp2pError::DisputeIntentNotPending)?;
        let stake_lock = ctx
            .accounts
            .dispute
            .stake_lock
            .as_deref_mut()
            .ok_or(Zkp2pError::DisputeIntentNotPending)?;
        let expected_dispute = Pubkey::find_program_address(
            &[
                DISPUTE_INTENT_SEED,
                dispute_config.key().as_ref(),
                &ctx.accounts.intent.intent_hash,
            ],
            &crate::ID,
        )
        .0;
        let expected_position = Pubkey::find_program_address(
            &[
                STAKE_POSITION_SEED,
                stake_vault.key().as_ref(),
                dispute_intent.stake_owner.as_ref(),
            ],
            &crate::ID,
        )
        .0;
        let expected_lock = Pubkey::find_program_address(
            &[
                STAKE_LOCK_SEED,
                stake_vault.key().as_ref(),
                &ctx.accounts.intent.intent_hash,
            ],
            &crate::ID,
        )
        .0;
        require_keys_eq!(
            dispute_config.stake_vault,
            stake_vault.key(),
            Zkp2pError::Unauthorized
        );
        require_keys_eq!(
            dispute_intent.key(),
            expected_dispute,
            Zkp2pError::Unauthorized
        );
        require_keys_eq!(position.key(), expected_position, Zkp2pError::Unauthorized);
        require_keys_eq!(stake_lock.key(), expected_lock, Zkp2pError::Unauthorized);
        require_keys_eq!(
            stake_lock.stake_owner,
            position.owner,
            Zkp2pError::UnauthorizedStakeOwner
        );
        dispute_intent.cancel()?;
        position.unlock(stake_lock.amount)?;
        stake_lock.close(ctx.accounts.owner_rent.to_account_info())?;
    } else {
        require!(
            ctx.accounts.dispute.dispute_config.is_none()
                && ctx.accounts.dispute.dispute_intent.is_none()
                && ctx.accounts.dispute.stake_vault.is_none()
                && ctx.accounts.dispute.stake_position.is_none()
                && ctx.accounts.dispute.stake_lock.is_none(),
            Zkp2pError::Unauthorized
        );
    }
    ctx.accounts
        .deposit
        .unlock(ctx.accounts.intent_lock.amount)?;
    ctx.accounts.taker_state.active_intents = ctx
        .accounts
        .taker_state
        .active_intents
        .checked_sub(1)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    Ok(())
}

fn effective_rate(
    currency: &DepositCurrency,
    oracle_quote: Option<&Account<'_, OracleQuote>>,
    configured_manager: Option<Pubkey>,
    rate_manager: Option<&Account<'_, RateManager>>,
    rate_entry: Option<&Account<'_, RateEntry>>,
    now: i64,
) -> Result<u128> {
    if let Some(expected_quote) = currency.oracle_quote {
        let supplied_quote = oracle_quote.ok_or(Zkp2pError::InvalidOracleQuote)?;
        require_keys_eq!(
            expected_quote,
            supplied_quote.key(),
            Zkp2pError::InvalidOracleQuote
        );
    } else {
        require!(oracle_quote.is_none(), Zkp2pError::InvalidOracleQuote);
    }
    let escrow_floor = currency.escrow_floor(oracle_quote.map(|quote| &**quote), now)?;
    if escrow_floor == 0 {
        return Ok(0);
    }
    match configured_manager {
        None => {
            require!(
                rate_manager.is_none() && rate_entry.is_none(),
                Zkp2pError::Unauthorized
            );
            Ok(escrow_floor)
        }
        Some(expected_manager) => {
            let manager = rate_manager.ok_or(Zkp2pError::Unauthorized)?;
            let entry = rate_entry.ok_or(Zkp2pError::CurrencyNotSupported)?;
            let canonical_manager = Pubkey::find_program_address(
                &[
                    crate::constants::RATE_MANAGER_SEED,
                    manager.config.as_ref(),
                    &manager.nonce.to_le_bytes(),
                ],
                &crate::ID,
            )
            .0;
            require_keys_eq!(manager.key(), canonical_manager, Zkp2pError::Unauthorized);
            require_keys_eq!(manager.key(), expected_manager, Zkp2pError::Unauthorized);
            require_keys_eq!(entry.rate_manager, manager.key(), Zkp2pError::Unauthorized);
            require!(
                entry.payment_method == currency.payment_method,
                Zkp2pError::PaymentMethodNotSupported
            );
            require!(
                entry.currency == currency.currency,
                Zkp2pError::CurrencyNotSupported
            );
            require!(entry.rate > 0, Zkp2pError::CurrencyNotSupported);
            Ok(entry.rate.max(escrow_floor))
        }
    }
}

fn validate_referral_fees(fees: &[ReferralFee]) -> Result<()> {
    require!(fees.len() <= 10, Zkp2pError::MaximumIntentsExceeded);
    let mut total = 0_u128;
    for (index, fee) in fees.iter().enumerate() {
        require!(fee.recipient != Pubkey::default(), Zkp2pError::ZeroAddress);
        require!(fee.fee > 0, Zkp2pError::ZeroValue);
        for other in fees.iter().skip(index.saturating_add(1)) {
            require!(other.recipient != fee.recipient, Zkp2pError::AlreadyInState);
        }
        total = total
            .checked_add(fee.fee)
            .ok_or(Zkp2pError::ArithmeticOverflow)?;
    }
    require!(total <= MAX_REFERRAL_FEE, Zkp2pError::FeeExceedsMaximum);
    Ok(())
}

fn validate_gating_signature(
    gating_service: Option<Pubkey>,
    instructions_sysvar: &AccountInfo<'_>,
    orchestrator: &Account<OrchestratorConfig>,
    deposit: &Account<Deposit>,
    taker: Pubkey,
    args: &SignalIntentArgs,
) -> Result<()> {
    let Some(gating_service) = gating_service else {
        return Ok(());
    };
    require!(
        Clock::get()?.unix_timestamp <= args.gating_signature_expiration,
        Zkp2pError::SignatureExpired
    );
    let mut message = Vec::new();
    orchestrator
        .key()
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    deposit
        .key()
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    taker
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    args.amount
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    args.recipient
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    args.payment_method
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    args.fiat_currency
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    args.conversion_rate
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    args.referral_fees
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    args.gating_signature_expiration
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    crate::ID
        .serialize(&mut message)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    let digest = keccak::hash(&message).to_bytes();

    let current = usize::from(
        load_current_index_checked(instructions_sysvar)
            .map_err(|_| error!(Zkp2pError::InvalidSignature))?,
    );
    for index in 0..current {
        let instruction = load_instruction_at_checked(index, instructions_sysvar)
            .map_err(|_| error!(Zkp2pError::InvalidSignature))?;
        if instruction.program_id == solana_program::ed25519_program::ID
            && ed25519_instruction_matches(&instruction.data, &gating_service, &digest)
        {
            return Ok(());
        }
    }
    err!(Zkp2pError::InvalidSignature)
}

fn ed25519_instruction_matches(data: &[u8], signer: &Pubkey, message: &[u8]) -> bool {
    if data.first() != Some(&1) {
        return false;
    }
    let Some(signature_instruction) = read_u16(data, 4) else {
        return false;
    };
    let Some(public_key_offset) = read_u16(data, 6) else {
        return false;
    };
    let Some(public_key_instruction) = read_u16(data, 8) else {
        return false;
    };
    let Some(message_offset) = read_u16(data, 10) else {
        return false;
    };
    let Some(message_size) = read_u16(data, 12) else {
        return false;
    };
    let Some(message_instruction) = read_u16(data, 14) else {
        return false;
    };
    if signature_instruction != u16::MAX
        || public_key_instruction != u16::MAX
        || message_instruction != u16::MAX
        || usize::from(message_size) != message.len()
    {
        return false;
    }
    let key_start = usize::from(public_key_offset);
    let key_end = key_start.saturating_add(32);
    let message_start = usize::from(message_offset);
    let message_end = message_start.saturating_add(message.len());
    data.get(key_start..key_end) == Some(signer.as_ref())
        && data.get(message_start..message_end) == Some(message)
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes: [u8; 2] = data
        .get(offset..offset.saturating_add(2))?
        .try_into()
        .ok()?;
    Some(u16::from_le_bytes(bytes))
}

struct LifecycleAdmission<'accounts, 'info> {
    policy: LifecyclePolicy,
    deposit: &'accounts Account<'info, Deposit>,
    whitelist: Option<&'accounts Account<'info, DepositWhitelist>>,
    direct_member: Option<&'accounts Account<'info, DepositWhitelistMember>>,
    allowed_group: Option<&'accounts Account<'info, AddressGroup>>,
    group_member: Option<&'accounts Account<'info, GroupMember>>,
    resolver_program: Option<&'accounts UncheckedAccount<'info>>,
    resolver_accounts: &'accounts [AccountInfo<'info>],
    dispute_intent: Option<&'accounts Account<'info, DisputeIntent>>,
    deposit_dispute_setting: Option<&'accounts Account<'info, DepositDisputeSetting>>,
    risk_window: Option<&'accounts Account<'info, RiskWindow>>,
    intent_hash: [u8; 32],
    taker: Pubkey,
    payment_method: [u8; 32],
    amount: u64,
}

fn validate_lifecycle_admission(admission: LifecycleAdmission<'_, '_>) -> Result<()> {
    let LifecycleAdmission {
        policy,
        deposit,
        whitelist,
        direct_member,
        allowed_group,
        group_member,
        resolver_program,
        resolver_accounts,
        dispute_intent,
        deposit_dispute_setting,
        risk_window,
        intent_hash,
        taker,
        payment_method,
        amount,
    } = admission;
    let no_whitelist_evidence = || {
        direct_member.is_none()
            && allowed_group.is_none()
            && group_member.is_none()
            && resolver_program.is_none()
            && resolver_accounts.is_empty()
    };
    if policy == LifecyclePolicy::None {
        require!(
            whitelist.is_none()
                && no_whitelist_evidence()
                && dispute_intent.is_none()
                && deposit_dispute_setting.is_none()
                && risk_window.is_none(),
            Zkp2pError::Unauthorized
        );
        return Ok(());
    }

    let whitelist_enabled = match whitelist {
        Some(policy_account) => {
            let expected = Pubkey::find_program_address(
                &[DEPOSIT_WHITELIST_SEED, deposit.key().as_ref()],
                &crate::ID,
            )
            .0;
            require_keys_eq!(policy_account.key(), expected, Zkp2pError::Unauthorized);
            require_keys_eq!(
                policy_account.deposit,
                deposit.key(),
                Zkp2pError::Unauthorized
            );
            policy_account.enabled
        }
        None => false,
    };
    let whitelisted = if whitelist_enabled {
        whitelist_allows(
            whitelist.ok_or(Zkp2pError::TakerNotWhitelisted)?,
            direct_member,
            allowed_group,
            group_member,
            resolver_program,
            resolver_accounts,
            taker,
        )?
    } else {
        require!(no_whitelist_evidence(), Zkp2pError::Unauthorized);
        false
    };

    if policy == LifecyclePolicy::Whitelist {
        require!(
            !whitelist_enabled || whitelisted,
            Zkp2pError::TakerNotWhitelisted
        );
        require!(
            dispute_intent.is_none() && deposit_dispute_setting.is_none() && risk_window.is_none(),
            Zkp2pError::Unauthorized
        );
        return Ok(());
    }

    if whitelisted {
        require!(
            dispute_intent.is_none() && deposit_dispute_setting.is_none() && risk_window.is_none(),
            Zkp2pError::Unauthorized
        );
        return Ok(());
    }

    let dispute_enabled = match deposit_dispute_setting {
        None => true,
        Some(setting) => {
            let expected = Pubkey::find_program_address(
                &[DEPOSIT_DISPUTE_SETTING_SEED, deposit.key().as_ref()],
                &crate::ID,
            )
            .0;
            require_keys_eq!(setting.key(), expected, Zkp2pError::Unauthorized);
            require_keys_eq!(setting.deposit, deposit.key(), Zkp2pError::Unauthorized);
            setting.enabled
        }
    };
    if !dispute_enabled {
        require!(
            dispute_intent.is_none() && risk_window.is_none(),
            Zkp2pError::Unauthorized
        );
        require!(!whitelist_enabled, Zkp2pError::TakerNotWhitelisted);
        return Ok(());
    }

    validate_dispute_admission(
        deposit,
        dispute_intent,
        risk_window,
        intent_hash,
        taker,
        payment_method,
        amount,
    )
}

fn whitelist_allows<'info>(
    whitelist: &Account<'info, DepositWhitelist>,
    direct_member: Option<&Account<'info, DepositWhitelistMember>>,
    allowed_group: Option<&Account<'info, AddressGroup>>,
    group_member: Option<&Account<'info, GroupMember>>,
    resolver_program: Option<&UncheckedAccount<'info>>,
    resolver_accounts: &[AccountInfo<'info>],
    taker: Pubkey,
) -> Result<bool> {
    let direct = direct_member.is_some_and(|member| {
        let expected = Pubkey::find_program_address(
            &[
                DEPOSIT_WHITELIST_MEMBER_SEED,
                whitelist.key().as_ref(),
                taker.as_ref(),
            ],
            &crate::ID,
        )
        .0;
        member.key() == expected
            && member.active
            && member.taker == taker
            && member.deposit_whitelist == whitelist.key()
    });
    if direct {
        require!(
            allowed_group.is_none()
                && group_member.is_none()
                && resolver_program.is_none()
                && resolver_accounts.is_empty(),
            Zkp2pError::Unauthorized
        );
        return Ok(true);
    }
    let Some(group) = allowed_group else {
        require!(
            group_member.is_none() && resolver_program.is_none() && resolver_accounts.is_empty(),
            Zkp2pError::Unauthorized
        );
        return Ok(false);
    };
    let expected_group = Pubkey::find_program_address(
        &[
            ADDRESS_GROUP_SEED,
            group.whitelist_config.as_ref(),
            &group.nonce.to_le_bytes(),
        ],
        &crate::ID,
    )
    .0;
    require_keys_eq!(group.key(), expected_group, Zkp2pError::Unauthorized);
    if !whitelist.allowed_groups.contains(&group.id) {
        return Ok(false);
    }
    let curated = group_member.is_some_and(|member| {
        let expected = Pubkey::find_program_address(
            &[GROUP_MEMBER_SEED, group.key().as_ref(), taker.as_ref()],
            &crate::ID,
        )
        .0;
        member.key() == expected
            && member.active
            && member.member == taker
            && member.group == group.key()
    });
    if curated {
        require!(
            resolver_program.is_none() && resolver_accounts.is_empty(),
            Zkp2pError::Unauthorized
        );
        return Ok(true);
    }
    resolver_says_yes(group, taker, resolver_program, resolver_accounts)
}

fn validate_dispute_admission(
    deposit: &Account<'_, Deposit>,
    dispute_intent: Option<&Account<'_, DisputeIntent>>,
    risk_window: Option<&Account<'_, RiskWindow>>,
    intent_hash: [u8; 32],
    taker: Pubkey,
    payment_method: [u8; 32],
    amount: u64,
) -> Result<()> {
    let risk = risk_window.ok_or(Zkp2pError::DisputeProtectionDisabled)?;
    let canonical_dispute = Pubkey::find_program_address(&[DISPUTE_CONFIG_SEED], &crate::ID).0;
    require_keys_eq!(
        risk.dispute_config,
        canonical_dispute,
        Zkp2pError::Unauthorized
    );
    let expected_risk = Pubkey::find_program_address(
        &[
            RISK_WINDOW_SEED,
            risk.dispute_config.as_ref(),
            &payment_method,
        ],
        &crate::ID,
    )
    .0;
    require_keys_eq!(risk.key(), expected_risk, Zkp2pError::Unauthorized);
    require!(
        risk.payment_method == payment_method,
        Zkp2pError::PaymentMethodNotSupported
    );
    if risk.seconds == 0 {
        require!(dispute_intent.is_none(), Zkp2pError::Unauthorized);
        return Ok(());
    }
    let dispute = dispute_intent.ok_or(Zkp2pError::DisputeProtectionDisabled)?;
    let expected_dispute = Pubkey::find_program_address(
        &[
            DISPUTE_INTENT_SEED,
            risk.dispute_config.as_ref(),
            &intent_hash,
        ],
        &crate::ID,
    )
    .0;
    require_keys_eq!(dispute.key(), expected_dispute, Zkp2pError::Unauthorized);
    require_keys_eq!(
        dispute.dispute_config,
        risk.dispute_config,
        Zkp2pError::Unauthorized
    );
    require!(
        dispute.intent_hash == intent_hash
            && dispute.taker == taker
            && dispute.payment_method == risk.payment_method
            && dispute.risk_window == risk.seconds
            && dispute.deposit == deposit.key()
            && dispute.locked_amount == amount,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(
        dispute.status == DisputeStatus::Pending,
        Zkp2pError::DisputeIntentNotPending
    );
    Ok(())
}

fn resolver_says_yes<'info>(
    group: &Account<'info, AddressGroup>,
    taker: Pubkey,
    resolver_program: Option<&UncheckedAccount<'info>>,
    resolver_accounts: &[AccountInfo<'info>],
) -> Result<bool> {
    let Some(expected_program) = group.resolver else {
        require!(
            resolver_program.is_none() && resolver_accounts.is_empty(),
            Zkp2pError::Unauthorized
        );
        return Ok(false);
    };
    let resolver = resolver_program.ok_or(Zkp2pError::TakerNotWhitelisted)?;
    require_keys_eq!(resolver.key(), expected_program, Zkp2pError::Unauthorized);
    require!(resolver.executable, Zkp2pError::Unauthorized);

    let metas = resolver_accounts
        .iter()
        .map(|account| solana_program::instruction::AccountMeta {
            pubkey: account.key(),
            is_signer: account.is_signer,
            is_writable: account.is_writable,
        })
        .collect();
    let mut data = b"zkp2p-whitelist-resolver-v1".to_vec();
    data.extend_from_slice(&group.id);
    data.extend_from_slice(taker.as_ref());
    let instruction = solana_program::instruction::Instruction {
        program_id: resolver.key(),
        accounts: metas,
        data,
    };
    let mut account_infos = resolver_accounts.to_vec();
    account_infos.push(resolver.to_account_info());
    solana_program::program::invoke(&instruction, &account_infos)?;
    Ok(
        solana_program::program::get_return_data().is_some_and(|(program_id, result)| {
            program_id == resolver.key() && result.as_slice() == [1]
        }),
    )
}

/// Derives the Solidity-compatible intent identifier and reduces it into the Circom scalar field.
pub fn derive_intent_hash(orchestrator: Pubkey, nonce: u64) -> [u8; 32] {
    let digest = keccak::hashv(&[orchestrator.as_ref(), &nonce.to_be_bytes()]).to_bytes();
    reduce_circom_field(digest)
}

fn reduce_circom_field(mut value: [u8; 32]) -> [u8; 32] {
    const MODULUS: [u8; 32] = [
        0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58,
        0x5d, 0x97, 0x81, 0x6a, 0x91, 0x68, 0x71, 0xca, 0x8d, 0x3c, 0x20, 0x8c, 0x16, 0xd8, 0x7c,
        0xfd, 0x47,
    ];
    while value >= MODULUS {
        let mut borrow = 0_u16;
        for (left_byte, modulus_byte) in value.iter_mut().rev().zip(MODULUS.iter().rev()) {
            let left = u16::from(*left_byte);
            let right = u16::from(*modulus_byte).saturating_add(borrow);
            if left >= right {
                *left_byte = u8::try_from(left.saturating_sub(right)).unwrap_or(0);
                borrow = 0;
            } else {
                *left_byte =
                    u8::try_from(left.saturating_add(256).saturating_sub(right)).unwrap_or(0);
                borrow = 1;
            }
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed25519_fixture(signer: Pubkey, message: &[u8]) -> Vec<u8> {
        let signature_offset = 16_u16;
        let public_key_offset = signature_offset.saturating_add(64);
        let message_offset = public_key_offset.saturating_add(32);
        let message_size = u16::try_from(message.len()).expect("small fixture");
        let mut data = Vec::new();
        data.push(1);
        data.push(0);
        for value in [
            signature_offset,
            u16::MAX,
            public_key_offset,
            u16::MAX,
            message_offset,
            message_size,
            u16::MAX,
        ] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&[7; 64]);
        data.extend_from_slice(signer.as_ref());
        data.extend_from_slice(message);
        data
    }

    #[test]
    fn circom_reduction_is_strictly_in_field() {
        let modulus = [
            0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81,
            0x58, 0x5d, 0x97, 0x81, 0x6a, 0x91, 0x68, 0x71, 0xca, 0x8d, 0x3c, 0x20, 0x8c, 0x16,
            0xd8, 0x7c, 0xfd, 0x47,
        ];
        assert!(reduce_circom_field([0xff; 32]) < modulus);
    }

    #[test]
    fn ed25519_parser_requires_exact_signer_message_and_local_offsets() {
        let signer = Pubkey::new_unique();
        let message = [9; 32];
        let data = ed25519_fixture(signer, &message);
        assert!(ed25519_instruction_matches(&data, &signer, &message));
        assert!(!ed25519_instruction_matches(
            &data,
            &Pubkey::new_unique(),
            &message
        ));
        assert!(!ed25519_instruction_matches(&data, &signer, &[8; 32]));

        let mut cross_instruction = data;
        cross_instruction
            .get_mut(8..10)
            .expect("fixture header")
            .copy_from_slice(&0_u16.to_le_bytes());
        assert!(!ed25519_instruction_matches(
            &cross_instruction,
            &signer,
            &message
        ));
    }
}
