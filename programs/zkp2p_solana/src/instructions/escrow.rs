//! EscrowV2-equivalent deposit custody and configuration instructions.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::{
    constants::{
        DEPOSIT_CURRENCY_SEED, DEPOSIT_SEED, DEPOSIT_VAULT_SEED, ESCROW_CONFIG_SEED,
        ESCROW_INTENT_LOCK_SEED, MAX_INTENT_LIFETIME_SECONDS, ORACLE_QUOTE_SEED,
        PAYMENT_METHOD_SEED, RATE_MANAGER_SEED,
    },
    error::Zkp2pError,
    state::{
        AmountRange, CreateDepositArgs, Deposit, DepositCurrency, DepositPaymentMethod,
        EscrowConfig, EscrowIntentLock, OracleQuote, RateManager,
    },
};

/// Accounts for creating or refreshing one authority-namespaced oracle quote.
#[derive(Accounts)]
#[instruction(quote_id: [u8; 32])]
pub struct UpdateOracleQuote<'info> {
    /// Quote authority and rent payer.
    #[account(mut)]
    pub authority: Signer<'info>,
    /// Program-owned quote selected by deposits that trust this authority and identifier.
    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + OracleQuote::INIT_SPACE,
        seeds = [ORACLE_QUOTE_SEED, authority.key().as_ref(), &quote_id],
        bump
    )]
    pub oracle_quote: Account<'info, OracleQuote>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Writes a timestamped market rate; invalidation may set zero but valid quotes must be nonzero.
pub fn handle_update_oracle_quote(
    ctx: Context<UpdateOracleQuote>,
    _quote_id: [u8; 32],
    market_rate: u128,
    valid: bool,
) -> Result<()> {
    require!(!valid || market_rate > 0, Zkp2pError::ZeroValue);
    let quote = &mut ctx.accounts.oracle_quote;
    if quote.authority == Pubkey::default() {
        quote.authority = ctx.accounts.authority.key();
        quote.bump = ctx.bumps.oracle_quote;
    }
    require_keys_eq!(
        quote.authority,
        ctx.accounts.authority.key(),
        Zkp2pError::Unauthorized
    );
    quote.market_rate = market_rate;
    quote.updated_at = Clock::get()?.unix_timestamp;
    quote.valid = valid;
    Ok(())
}

/// Accounts for one funded deposit and its first active payment/currency tuple.
#[derive(Accounts)]
#[instruction(args: CreateDepositArgs)]
pub struct CreateDeposit<'info> {
    /// Maker, rent payer, and source-token authority.
    #[account(mut)]
    pub depositor: Signer<'info>,
    /// Escrow component configuration.
    #[account(mut, seeds = [ESCROW_CONFIG_SEED], bump = escrow_config.bump)]
    pub escrow_config: Box<Account<'info, EscrowConfig>>,
    /// Monotonic deposit PDA.
    #[account(
        init,
        payer = depositor,
        space = 8 + Deposit::INIT_SPACE,
        seeds = [DEPOSIT_SEED, escrow_config.key().as_ref(), &escrow_config.next_deposit_id.to_le_bytes()],
        bump
    )]
    pub deposit: Box<Account<'info, Deposit>>,
    /// First payment-method PDA.
    #[account(
        init,
        payer = depositor,
        space = 8 + DepositPaymentMethod::INIT_SPACE,
        seeds = [PAYMENT_METHOD_SEED, deposit.key().as_ref(), &args.payment_method],
        bump
    )]
    pub payment_method: Box<Account<'info, DepositPaymentMethod>>,
    /// First method/currency PDA.
    #[account(
        init,
        payer = depositor,
        space = 8 + DepositCurrency::INIT_SPACE,
        seeds = [DEPOSIT_CURRENCY_SEED, deposit.key().as_ref(), &args.payment_method, &args.currency],
        bump
    )]
    pub currency: Box<Account<'info, DepositCurrency>>,
    /// Canonical escrow mint.
    #[account(address = escrow_config.token_mint)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    /// Maker's source token account.
    #[account(mut, token::mint = mint, token::authority = depositor, token::token_program = token_program)]
    pub depositor_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Dedicated deposit custody account, controlled by the deposit PDA.
    #[account(
        init,
        payer = depositor,
        seeds = [DEPOSIT_VAULT_SEED, deposit.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = deposit,
        token::token_program = token_program
    )]
    pub deposit_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// SPL Token or Token-2022 program.
    pub token_program: Interface<'info, TokenInterface>,
    /// System program used to create protocol PDAs.
    pub system_program: Program<'info, System>,
}

/// Creates and funds a deposit with one immediately usable payment/currency tuple.
pub fn handle_create_deposit(ctx: Context<CreateDeposit>, args: CreateDepositArgs) -> Result<()> {
    require!(!ctx.accounts.escrow_config.paused, Zkp2pError::Paused);
    require!(args.amount > 0, Zkp2pError::ZeroValue);
    require!(args.intent_amount_range.min > 0, Zkp2pError::ZeroValue);
    require!(
        args.intent_amount_range.min <= args.intent_amount_range.max,
        Zkp2pError::InvalidRange
    );
    require!(
        args.amount >= args.intent_amount_range.min,
        Zkp2pError::AmountBelowMinimum
    );
    require!(args.payment_method != [0; 32], Zkp2pError::ZeroValue);
    require!(args.payee_details != [0; 32], Zkp2pError::ZeroValue);
    require!(args.currency != [0; 32], Zkp2pError::ZeroValue);
    require!(
        args.fixed_min_rate > 0 || args.oracle_quote.is_some(),
        Zkp2pError::CurrencyNotSupported
    );
    require!(
        i32::from(args.spread_bps) > -10_000,
        Zkp2pError::InvalidSpread
    );
    if let Some(delegate) = args.delegate {
        require!(
            delegate != Pubkey::default() && delegate != ctx.accounts.depositor.key(),
            Zkp2pError::ZeroAddress
        );
    }
    if let Some(guardian) = args.intent_guardian {
        require!(guardian != Pubkey::default(), Zkp2pError::ZeroAddress);
    }
    if let Some(gating_service) = args.gating_service {
        require!(gating_service != Pubkey::default(), Zkp2pError::ZeroAddress);
    }
    if let Some(oracle_quote) = args.oracle_quote {
        require!(oracle_quote != Pubkey::default(), Zkp2pError::ZeroAddress);
        require!(args.max_staleness > 0, Zkp2pError::ZeroValue);
    }

    let id = ctx.accounts.escrow_config.next_deposit_id;
    let deposit = &mut ctx.accounts.deposit;
    deposit.escrow_config = ctx.accounts.escrow_config.key();
    deposit.id = id;
    deposit.depositor = ctx.accounts.depositor.key();
    deposit.delegate = args.delegate;
    deposit.token_mint = ctx.accounts.mint.key();
    deposit.intent_amount_range = args.intent_amount_range;
    deposit.accepting_intents = true;
    deposit.remaining_deposits = args.amount;
    deposit.outstanding_intent_amount = 0;
    deposit.active_intents = 0;
    deposit.intent_guardian = args.intent_guardian;
    deposit.retain_on_empty = args.retain_on_empty;
    deposit.rate_manager = None;
    deposit.bump = ctx.bumps.deposit;
    deposit.vault_authority_bump = ctx.bumps.deposit_vault;

    let payment_method = &mut ctx.accounts.payment_method;
    payment_method.deposit = deposit.key();
    payment_method.payment_method = args.payment_method;
    payment_method.payee_details = args.payee_details;
    payment_method.gating_service = args.gating_service;
    payment_method.active = true;
    payment_method.bump = ctx.bumps.payment_method;

    let currency = &mut ctx.accounts.currency;
    currency.deposit = deposit.key();
    currency.payment_method = args.payment_method;
    currency.currency = args.currency;
    currency.fixed_min_rate = args.fixed_min_rate;
    currency.oracle_quote = args.oracle_quote;
    currency.spread_bps = args.spread_bps;
    currency.max_staleness = args.max_staleness;
    currency.listed = true;
    currency.bump = ctx.bumps.currency;

    ctx.accounts.escrow_config.next_deposit_id =
        id.checked_add(1).ok_or(Zkp2pError::ArithmeticOverflow)?;

    let balance_before = ctx.accounts.deposit_vault.amount;
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.depositor_token.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.deposit_vault.to_account_info(),
                authority: ctx.accounts.depositor.to_account_info(),
            },
        ),
        args.amount,
        ctx.accounts.mint.decimals,
    )?;
    ctx.accounts.deposit_vault.reload()?;
    let received = ctx
        .accounts
        .deposit_vault
        .amount
        .checked_sub(balance_before)
        .ok_or(Zkp2pError::InvalidTokenBalanceDelta)?;
    require!(
        received == args.amount,
        Zkp2pError::InvalidTokenBalanceDelta
    );
    Ok(())
}

/// Accounts for permissionlessly increasing a deposit with exact token accounting.
#[derive(Accounts)]
pub struct AddFunds<'info> {
    /// Account supplying the tokens.
    #[account(mut)]
    pub funder: Signer<'info>,
    /// Canonical unpaused escrow component.
    #[account(
        seeds = [ESCROW_CONFIG_SEED],
        bump = escrow_config.bump,
        constraint = !escrow_config.paused @ Zkp2pError::Paused
    )]
    pub escrow_config: Box<Account<'info, EscrowConfig>>,
    /// Existing deposit.
    #[account(
        mut,
        has_one = token_mint,
        constraint = deposit.escrow_config == escrow_config.key() @ Zkp2pError::DepositNotFound
    )]
    pub deposit: Box<Account<'info, Deposit>>,
    /// Canonical deposit mint.
    #[account(address = deposit.token_mint)]
    pub token_mint: Box<InterfaceAccount<'info, Mint>>,
    /// Funder-owned source account.
    #[account(mut, token::mint = token_mint, token::authority = funder, token::token_program = token_program)]
    pub funder_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Deposit custody account.
    #[account(mut, seeds = [DEPOSIT_VAULT_SEED, deposit.key().as_ref()], bump = deposit.vault_authority_bump)]
    pub deposit_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// SPL Token or Token-2022 program.
    pub token_program: Interface<'info, TokenInterface>,
}

/// Adds exact received principal to a deposit. Any token holder may fund it.
pub fn handle_add_funds(ctx: Context<AddFunds>, amount: u64) -> Result<()> {
    require!(amount > 0, Zkp2pError::ZeroValue);
    let before = ctx.accounts.deposit_vault.amount;
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.funder_token.to_account_info(),
                mint: ctx.accounts.token_mint.to_account_info(),
                to: ctx.accounts.deposit_vault.to_account_info(),
                authority: ctx.accounts.funder.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.token_mint.decimals,
    )?;
    ctx.accounts.deposit_vault.reload()?;
    let received = ctx
        .accounts
        .deposit_vault
        .amount
        .checked_sub(before)
        .ok_or(Zkp2pError::InvalidTokenBalanceDelta)?;
    require!(received == amount, Zkp2pError::InvalidTokenBalanceDelta);
    ctx.accounts.deposit.remaining_deposits = ctx
        .accounts
        .deposit
        .remaining_deposits
        .checked_add(received)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    Ok(())
}

/// Accounts for removing currently available maker liquidity.
#[derive(Accounts)]
pub struct RemoveFunds<'info> {
    /// Deposit owner.
    #[account(mut, address = deposit.depositor)]
    pub depositor: Signer<'info>,
    /// Canonical unpaused escrow component.
    #[account(
        seeds = [ESCROW_CONFIG_SEED],
        bump = escrow_config.bump,
        constraint = !escrow_config.paused @ Zkp2pError::Paused
    )]
    pub escrow_config: Box<Account<'info, EscrowConfig>>,
    /// Existing deposit.
    #[account(
        mut,
        has_one = token_mint,
        constraint = deposit.escrow_config == escrow_config.key() @ Zkp2pError::DepositNotFound
    )]
    pub deposit: Box<Account<'info, Deposit>>,
    /// Canonical deposit mint.
    #[account(address = deposit.token_mint)]
    pub token_mint: Box<InterfaceAccount<'info, Mint>>,
    /// Deposit custody account.
    #[account(mut, seeds = [DEPOSIT_VAULT_SEED, deposit.key().as_ref()], bump = deposit.vault_authority_bump)]
    pub deposit_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Owner destination account.
    #[account(mut, token::mint = token_mint, token::authority = depositor, token::token_program = token_program)]
    pub depositor_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// SPL Token or Token-2022 program.
    pub token_program: Interface<'info, TokenInterface>,
}

/// Removes available principal. Expired locks can be pruned earlier in the same transaction.
pub fn handle_remove_funds(ctx: Context<RemoveFunds>, amount: u64) -> Result<()> {
    require!(amount > 0, Zkp2pError::ZeroValue);
    require!(
        ctx.accounts.deposit.remaining_deposits >= amount,
        Zkp2pError::InsufficientBalance
    );
    ctx.accounts.deposit.remaining_deposits = ctx
        .accounts
        .deposit
        .remaining_deposits
        .checked_sub(amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    transfer_from_deposit(
        &ctx.accounts.deposit,
        &ctx.accounts.deposit_vault,
        &ctx.accounts.depositor_token,
        &ctx.accounts.token_mint,
        &ctx.accounts.token_program,
        amount,
    )
}

/// Accounts for returning all currently available liquidity and closing admissions.
#[derive(Accounts)]
pub struct WithdrawDeposit<'info> {
    /// Deposit owner.
    #[account(mut, address = deposit.depositor)]
    pub depositor: Signer<'info>,
    /// Canonical escrow settings, including the configured dust policy.
    #[account(
        seeds = [ESCROW_CONFIG_SEED],
        bump = escrow_config.bump,
        constraint = deposit.escrow_config == escrow_config.key() @ Zkp2pError::DepositNotFound
    )]
    pub escrow_config: Box<Account<'info, EscrowConfig>>,
    /// Existing deposit.
    #[account(mut, has_one = token_mint)]
    pub deposit: Box<Account<'info, Deposit>>,
    /// Canonical deposit mint.
    #[account(address = deposit.token_mint)]
    pub token_mint: Box<InterfaceAccount<'info, Mint>>,
    /// Deposit custody account.
    #[account(mut, seeds = [DEPOSIT_VAULT_SEED, deposit.key().as_ref()], bump = deposit.vault_authority_bump)]
    pub deposit_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Owner destination account.
    #[account(mut, token::mint = token_mint, token::authority = depositor, token::token_program = token_program)]
    pub depositor_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Configured dust recipient's token account for this mint.
    #[account(
        mut,
        token::mint = token_mint,
        token::authority = escrow_config.dust_recipient,
        token::token_program = token_program
    )]
    pub dust_recipient_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// SPL Token or Token-2022 program.
    pub token_program: Interface<'info, TokenInterface>,
}

/// Returns all available principal and closes liability-free, non-retained deposits.
pub fn handle_withdraw_deposit(ctx: Context<WithdrawDeposit>) -> Result<()> {
    let amount = ctx.accounts.deposit.remaining_deposits;
    ctx.accounts.deposit.remaining_deposits = 0;
    ctx.accounts.deposit.accepting_intents = false;
    if amount > 0 {
        transfer_from_deposit(
            &ctx.accounts.deposit,
            &ctx.accounts.deposit_vault,
            &ctx.accounts.depositor_token,
            &ctx.accounts.token_mint,
            &ctx.accounts.token_program,
            amount,
        )?;
    }

    if ctx.accounts.deposit.active_intents == 0
        && ctx.accounts.deposit.outstanding_intent_amount == 0
        && !ctx.accounts.deposit.retain_on_empty
    {
        ctx.accounts.deposit_vault.reload()?;
        let residual = ctx.accounts.deposit_vault.amount;
        if residual > ctx.accounts.escrow_config.dust_threshold {
            ctx.accounts.deposit.retain_on_empty = true;
            return Ok(());
        }
        if residual > 0 {
            transfer_from_deposit(
                &ctx.accounts.deposit,
                &ctx.accounts.deposit_vault,
                &ctx.accounts.dust_recipient_token,
                &ctx.accounts.token_mint,
                &ctx.accounts.token_program,
                residual,
            )?;
        }
        let id = ctx.accounts.deposit.id.to_le_bytes();
        let bump = [ctx.accounts.deposit.bump];
        let signer_seeds: &[&[u8]] = &[
            DEPOSIT_SEED,
            ctx.accounts.deposit.escrow_config.as_ref(),
            &id,
            &bump,
        ];
        token_interface::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            token_interface::CloseAccount {
                account: ctx.accounts.deposit_vault.to_account_info(),
                destination: ctx.accounts.depositor.to_account_info(),
                authority: ctx.accounts.deposit.to_account_info(),
            },
            &[signer_seeds],
        ))?;
        ctx.accounts
            .deposit
            .close(ctx.accounts.depositor.to_account_info())?;
    }
    Ok(())
}

/// Mutable maker-controlled deposit settings.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct UpdateDepositArgs {
    /// Optional replacement delegate; nested option distinguishes no change from removal.
    pub delegate: Option<Option<Pubkey>>,
    /// Optional replacement intent guardian.
    pub intent_guardian: Option<Option<Pubkey>>,
    /// Optional replacement intent range.
    pub intent_amount_range: Option<AmountRange>,
    /// Optional admissions state.
    pub accepting_intents: Option<bool>,
    /// Optional retain-on-empty state.
    pub retain_on_empty: Option<bool>,
}

/// Accounts for updating deposit-owned configuration.
#[derive(Accounts)]
pub struct UpdateDeposit<'info> {
    /// Owner or currently configured delegate.
    pub authority: Signer<'info>,
    /// Canonical unpaused escrow component.
    #[account(
        seeds = [ESCROW_CONFIG_SEED],
        bump = escrow_config.bump,
        constraint = !escrow_config.paused @ Zkp2pError::Paused
    )]
    pub escrow_config: Account<'info, EscrowConfig>,
    /// Existing deposit.
    #[account(
        mut,
        constraint = deposit.escrow_config == escrow_config.key() @ Zkp2pError::DepositNotFound,
        constraint = deposit.depositor == authority.key() || deposit.delegate == Some(authority.key()) @ Zkp2pError::Unauthorized
    )]
    pub deposit: Account<'info, Deposit>,
}

/// Applies a bounded deposit configuration update.
pub fn handle_update_deposit(ctx: Context<UpdateDeposit>, args: UpdateDepositArgs) -> Result<()> {
    let deposit = &mut ctx.accounts.deposit;
    if let Some(delegate) = args.delegate {
        require!(
            ctx.accounts.authority.key() == deposit.depositor,
            Zkp2pError::Unauthorized
        );
        if let Some(key) = delegate {
            require!(
                key != Pubkey::default() && key != deposit.depositor,
                Zkp2pError::ZeroAddress
            );
        }
        deposit.delegate = delegate;
    }
    if let Some(guardian) = args.intent_guardian {
        if let Some(key) = guardian {
            require!(key != Pubkey::default(), Zkp2pError::ZeroAddress);
        }
        deposit.intent_guardian = guardian;
    }
    if let Some(range) = args.intent_amount_range {
        require!(range.min > 0, Zkp2pError::ZeroValue);
        require!(range.min <= range.max, Zkp2pError::InvalidRange);
        deposit.intent_amount_range = range;
    }
    if let Some(accepting) = args.accepting_intents {
        require!(
            deposit.accepting_intents != accepting,
            Zkp2pError::AlreadyInState
        );
        if accepting {
            require!(
                deposit.remaining_deposits >= deposit.intent_amount_range.min,
                Zkp2pError::InsufficientBalance
            );
        }
        deposit.accepting_intents = accepting;
    }
    if let Some(retain) = args.retain_on_empty {
        require!(
            deposit.retain_on_empty != retain,
            Zkp2pError::AlreadyInState
        );
        deposit.retain_on_empty = retain;
    }
    Ok(())
}

/// Payment-method mutation payload.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConfigurePaymentMethodArgs {
    /// Payment method identifier.
    pub payment_method: [u8; 32],
    /// Hashed payee identifier.
    pub payee_details: [u8; 32],
    /// Optional gating authority.
    pub gating_service: Option<Pubkey>,
    /// Active state.
    pub active: bool,
}

/// Accounts for creating or updating one deposit payment method.
#[derive(Accounts)]
#[instruction(args: ConfigurePaymentMethodArgs)]
pub struct ConfigurePaymentMethod<'info> {
    /// Owner or delegate and rent payer.
    #[account(mut)]
    pub authority: Signer<'info>,
    /// Canonical unpaused escrow component.
    #[account(
        seeds = [ESCROW_CONFIG_SEED],
        bump = escrow_config.bump,
        constraint = !escrow_config.paused @ Zkp2pError::Paused
    )]
    pub escrow_config: Account<'info, EscrowConfig>,
    /// Existing deposit.
    #[account(
        constraint = deposit.escrow_config == escrow_config.key() @ Zkp2pError::DepositNotFound,
        constraint = deposit.depositor == authority.key() || deposit.delegate == Some(authority.key()) @ Zkp2pError::Unauthorized
    )]
    pub deposit: Account<'info, Deposit>,
    /// Method PDA.
    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + DepositPaymentMethod::INIT_SPACE,
        seeds = [PAYMENT_METHOD_SEED, deposit.key().as_ref(), &args.payment_method],
        bump
    )]
    pub payment_method: Account<'info, DepositPaymentMethod>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Creates or updates a payment method without leaving stale fields.
pub fn handle_configure_payment_method(
    ctx: Context<ConfigurePaymentMethod>,
    args: ConfigurePaymentMethodArgs,
) -> Result<()> {
    require!(args.payment_method != [0; 32], Zkp2pError::ZeroValue);
    require!(args.payee_details != [0; 32], Zkp2pError::ZeroValue);
    if let Some(gating) = args.gating_service {
        require!(gating != Pubkey::default(), Zkp2pError::ZeroAddress);
    }
    let method = &mut ctx.accounts.payment_method;
    method.deposit = ctx.accounts.deposit.key();
    method.payment_method = args.payment_method;
    method.payee_details = args.payee_details;
    method.gating_service = args.gating_service;
    method.active = args.active;
    method.bump = ctx.bumps.payment_method;
    Ok(())
}

/// Currency mutation payload.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConfigureCurrencyArgs {
    /// Payment method identifier.
    pub payment_method: [u8; 32],
    /// Currency identifier.
    pub currency: [u8; 32],
    /// Fixed minimum rate.
    pub fixed_min_rate: u128,
    /// Optional oracle quote.
    pub oracle_quote: Option<Pubkey>,
    /// Signed oracle spread.
    pub spread_bps: i16,
    /// Maximum quote age.
    pub max_staleness: u32,
    /// Listed state.
    pub listed: bool,
}

/// Accounts for creating or updating one deposit currency.
#[derive(Accounts)]
#[instruction(args: ConfigureCurrencyArgs)]
pub struct ConfigureCurrency<'info> {
    /// Owner or delegate and rent payer.
    #[account(mut)]
    pub authority: Signer<'info>,
    /// Canonical unpaused escrow component.
    #[account(
        seeds = [ESCROW_CONFIG_SEED],
        bump = escrow_config.bump,
        constraint = !escrow_config.paused @ Zkp2pError::Paused
    )]
    pub escrow_config: Account<'info, EscrowConfig>,
    /// Existing deposit.
    #[account(
        constraint = deposit.escrow_config == escrow_config.key() @ Zkp2pError::DepositNotFound,
        constraint = deposit.depositor == authority.key() || deposit.delegate == Some(authority.key()) @ Zkp2pError::Unauthorized
    )]
    pub deposit: Account<'info, Deposit>,
    /// Existing parent method.
    #[account(
        seeds = [PAYMENT_METHOD_SEED, deposit.key().as_ref(), &args.payment_method],
        bump = payment_method.bump,
        constraint = payment_method.deposit == deposit.key() @ Zkp2pError::PaymentMethodNotSupported
    )]
    pub payment_method: Account<'info, DepositPaymentMethod>,
    /// Currency PDA.
    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + DepositCurrency::INIT_SPACE,
        seeds = [DEPOSIT_CURRENCY_SEED, deposit.key().as_ref(), &args.payment_method, &args.currency],
        bump
    )]
    pub deposit_currency: Account<'info, DepositCurrency>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Creates or updates a fixed/oracle-backed currency tuple.
pub fn handle_configure_currency(
    ctx: Context<ConfigureCurrency>,
    args: ConfigureCurrencyArgs,
) -> Result<()> {
    require!(args.currency != [0; 32], Zkp2pError::ZeroValue);
    require!(
        !args.listed || args.fixed_min_rate > 0 || args.oracle_quote.is_some(),
        Zkp2pError::CurrencyNotSupported
    );
    require!(
        i32::from(args.spread_bps) > -10_000,
        Zkp2pError::InvalidSpread
    );
    if let Some(quote) = args.oracle_quote {
        require!(quote != Pubkey::default(), Zkp2pError::ZeroAddress);
        require!(args.max_staleness > 0, Zkp2pError::ZeroValue);
    }
    let currency = &mut ctx.accounts.deposit_currency;
    currency.deposit = ctx.accounts.deposit.key();
    currency.payment_method = args.payment_method;
    currency.currency = args.currency;
    currency.fixed_min_rate = args.fixed_min_rate;
    currency.oracle_quote = args.oracle_quote;
    currency.spread_bps = args.spread_bps;
    currency.max_staleness = args.max_staleness;
    currency.listed = args.listed;
    currency.bump = ctx.bumps.deposit_currency;
    Ok(())
}

/// Accounts for opting into or clearing a delegated rate manager.
#[derive(Accounts)]
pub struct SetDepositRateManager<'info> {
    /// Owner or delegate.
    pub authority: Signer<'info>,
    /// Canonical unpaused escrow component.
    #[account(
        seeds = [ESCROW_CONFIG_SEED],
        bump = escrow_config.bump,
        constraint = !escrow_config.paused @ Zkp2pError::Paused
    )]
    pub escrow_config: Account<'info, EscrowConfig>,
    /// Existing deposit.
    #[account(
        mut,
        constraint = deposit.escrow_config == escrow_config.key() @ Zkp2pError::DepositNotFound,
        constraint = deposit.depositor == authority.key() || deposit.delegate == Some(authority.key()) @ Zkp2pError::Unauthorized
    )]
    pub deposit: Account<'info, Deposit>,
    /// Optional manager PDA; required when selecting a manager.
    pub rate_manager: Option<Account<'info, RateManager>>,
}

/// Selects a live manager after enforcing its liquidity threshold, or clears it.
pub fn handle_set_deposit_rate_manager(
    ctx: Context<SetDepositRateManager>,
    manager: Option<Pubkey>,
) -> Result<()> {
    match manager {
        None => ctx.accounts.deposit.rate_manager = None,
        Some(key) => {
            let rate_manager = ctx
                .accounts
                .rate_manager
                .as_ref()
                .ok_or(Zkp2pError::Unauthorized)?;
            let canonical_manager = Pubkey::find_program_address(
                &[
                    RATE_MANAGER_SEED,
                    rate_manager.config.as_ref(),
                    &rate_manager.nonce.to_le_bytes(),
                ],
                &crate::ID,
            )
            .0;
            require_keys_eq!(
                rate_manager.key(),
                canonical_manager,
                Zkp2pError::Unauthorized
            );
            require_keys_eq!(rate_manager.key(), key, Zkp2pError::Unauthorized);
            let total = ctx
                .accounts
                .deposit
                .total_liquidity()
                .ok_or(Zkp2pError::ArithmeticOverflow)?;
            require!(
                total >= rate_manager.min_liquidity,
                Zkp2pError::InsufficientBalance
            );
            ctx.accounts.deposit.rate_manager = Some(key);
        }
    }
    Ok(())
}

/// Accounts for an orchestrator-created escrow lock.
#[derive(Accounts)]
#[instruction(intent_hash: [u8; 32])]
pub struct LockFunds<'info> {
    /// Canonical orchestrator configuration PDA.
    #[account(seeds = [crate::constants::ORCHESTRATOR_CONFIG_SEED], bump = orchestrator.bump)]
    pub orchestrator: Account<'info, crate::state::OrchestratorConfig>,
    /// Transaction payer. The orchestrator instruction constrains this signer to the taker.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// Escrow configuration.
    #[account(seeds = [ESCROW_CONFIG_SEED], bump = escrow_config.bump)]
    pub escrow_config: Account<'info, EscrowConfig>,
    /// Deposit being locked.
    #[account(mut, constraint = deposit.escrow_config == escrow_config.key() @ Zkp2pError::DepositNotFound)]
    pub deposit: Account<'info, Deposit>,
    /// New one-intent lock PDA.
    #[account(
        init,
        payer = payer,
        space = 8 + EscrowIntentLock::INIT_SPACE,
        seeds = [ESCROW_INTENT_LOCK_SEED, deposit.key().as_ref(), &intent_hash],
        bump
    )]
    pub intent_lock: Account<'info, EscrowIntentLock>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Locks available maker principal under a canonical orchestrator intent hash.
pub fn handle_lock_funds(
    ctx: Context<LockFunds>,
    intent_hash: [u8; 32],
    amount: u64,
) -> Result<()> {
    require!(!ctx.accounts.escrow_config.paused, Zkp2pError::Paused);
    require!(intent_hash != [0; 32], Zkp2pError::ZeroValue);
    ctx.accounts
        .deposit
        .lock(amount, ctx.accounts.escrow_config.max_intents_per_deposit)?;
    let now = Clock::get()?.unix_timestamp;
    let expiry = now
        .checked_add(ctx.accounts.escrow_config.intent_expiration_period)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    let lock = &mut ctx.accounts.intent_lock;
    lock.deposit = ctx.accounts.deposit.key();
    lock.intent_hash = intent_hash;
    lock.orchestrator = ctx.accounts.orchestrator.key();
    lock.amount = amount;
    lock.timestamp = now;
    lock.expiry_time = expiry;
    lock.bump = ctx.bumps.intent_lock;
    Ok(())
}

/// Accounts for cancelling or permissionlessly pruning one escrow lock.
#[derive(Accounts)]
pub struct UnlockFunds<'info> {
    /// Lock-owning orchestrator or permissionless expiry caller.
    #[account(mut)]
    pub authority: Signer<'info>,
    /// Deposit holding the lock.
    #[account(mut)]
    pub deposit: Account<'info, Deposit>,
    /// Lock to close after accounting is restored.
    #[account(
        mut,
        close = authority,
        seeds = [ESCROW_INTENT_LOCK_SEED, deposit.key().as_ref(), &intent_lock.intent_hash],
        bump = intent_lock.bump,
        constraint = intent_lock.deposit == deposit.key() @ Zkp2pError::IntentNotFound
    )]
    pub intent_lock: Account<'info, EscrowIntentLock>,
}

/// Unlocks a lock when invoked by its orchestrator, or by anyone strictly after expiry.
pub fn handle_unlock_funds(ctx: Context<UnlockFunds>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let caller = ctx.accounts.authority.key();
    require!(
        caller == ctx.accounts.intent_lock.orchestrator
            || now > ctx.accounts.intent_lock.expiry_time,
        Zkp2pError::Unauthorized
    );
    ctx.accounts.deposit.unlock(ctx.accounts.intent_lock.amount)
}

/// Accounts for the configured guardian extending one live lock.
#[derive(Accounts)]
pub struct ExtendIntentExpiry<'info> {
    /// Configured guardian.
    pub guardian: Signer<'info>,
    /// Parent deposit.
    #[account(constraint = deposit.intent_guardian == Some(guardian.key()) @ Zkp2pError::Unauthorized)]
    pub deposit: Account<'info, Deposit>,
    /// Live lock.
    #[account(
        mut,
        seeds = [ESCROW_INTENT_LOCK_SEED, deposit.key().as_ref(), &intent_lock.intent_hash],
        bump = intent_lock.bump,
        constraint = intent_lock.deposit == deposit.key() @ Zkp2pError::IntentNotFound
    )]
    pub intent_lock: Account<'info, EscrowIntentLock>,
}

/// Extends expiry without exceeding the lifetime cap measured from signal time.
pub fn handle_extend_intent_expiry(
    ctx: Context<ExtendIntentExpiry>,
    additional_time: i64,
) -> Result<()> {
    require!(additional_time > 0, Zkp2pError::ZeroValue);
    let new_expiry = ctx
        .accounts
        .intent_lock
        .expiry_time
        .checked_add(additional_time)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    let maximum = ctx
        .accounts
        .intent_lock
        .timestamp
        .checked_add(MAX_INTENT_LIFETIME_SECONDS)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    require!(new_expiry <= maximum, Zkp2pError::AmountAboveMaximum);
    ctx.accounts.intent_lock.expiry_time = new_expiry;
    Ok(())
}

fn transfer_from_deposit<'info>(
    deposit: &Account<'info, Deposit>,
    deposit_vault: &InterfaceAccount<'info, TokenAccount>,
    destination: &InterfaceAccount<'info, TokenAccount>,
    mint: &InterfaceAccount<'info, Mint>,
    token_program: &Interface<'info, TokenInterface>,
    amount: u64,
) -> Result<()> {
    let id = deposit.id.to_le_bytes();
    let bump = [deposit.bump];
    let seeds: &[&[u8]] = &[DEPOSIT_SEED, deposit.escrow_config.as_ref(), &id, &bump];
    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            token_program.key(),
            TransferChecked {
                from: deposit_vault.to_account_info(),
                mint: mint.to_account_info(),
                to: destination.to_account_info(),
                authority: deposit.to_account_info(),
            },
            &[seeds],
        ),
        amount,
        mint.decimals,
    )
}
