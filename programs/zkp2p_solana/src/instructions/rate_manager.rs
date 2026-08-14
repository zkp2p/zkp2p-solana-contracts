//! Delegated RateManagerV1-equivalent instructions.

use anchor_lang::prelude::*;
use solana_keccak_hasher as keccak;

use crate::{
    constants::{MAX_FEE, RATE_ENTRY_SEED, RATE_MANAGER_CONFIG_SEED, RATE_MANAGER_SEED},
    error::Zkp2pError,
    state::{CreateRateManagerArgs, RateEntry, RateManager, RateManagerConfig},
};

/// Accounts for creating one manager configuration.
#[derive(Accounts)]
pub struct CreateRateManager<'info> {
    /// Rent payer. Manager identity is supplied explicitly, matching Solidity permissionless creation.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// Monotonic manager configuration.
    #[account(mut, seeds = [RATE_MANAGER_CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, RateManagerConfig>,
    /// Newly created manager account.
    #[account(
        init,
        payer = payer,
        space = 8 + RateManager::INIT_SPACE,
        seeds = [RATE_MANAGER_SEED, config.key().as_ref(), &config.next_manager_id.to_le_bytes()],
        bump
    )]
    pub rate_manager: Account<'info, RateManager>,
    /// System program used to create the manager PDA.
    pub system_program: Program<'info, System>,
}

/// Creates a manager with immutable fee ceiling and monotonic deterministic ID.
pub fn handle_create_rate_manager(
    ctx: Context<CreateRateManager>,
    args: CreateRateManagerArgs,
) -> Result<()> {
    require!(args.manager != Pubkey::default(), Zkp2pError::ZeroAddress);
    require!(args.name.len() <= 64, Zkp2pError::AmountAboveMaximum);
    require!(args.uri.len() <= 200, Zkp2pError::AmountAboveMaximum);
    require!(args.max_fee <= MAX_FEE, Zkp2pError::FeeExceedsMaximum);
    require!(args.fee <= args.max_fee, Zkp2pError::FeeExceedsMaximum);
    if args.fee > 0 {
        require!(args.fee_recipient.is_some(), Zkp2pError::ZeroAddress);
    }
    if let Some(recipient) = args.fee_recipient {
        require!(recipient != Pubkey::default(), Zkp2pError::ZeroAddress);
    }

    let nonce = ctx.accounts.config.next_manager_id;
    let id = keccak::hashv(&[
        crate::ID.as_ref(),
        ctx.accounts.config.key().as_ref(),
        nonce.to_le_bytes().as_ref(),
    ])
    .to_bytes();
    let rate_manager = &mut ctx.accounts.rate_manager;
    rate_manager.config = ctx.accounts.config.key();
    rate_manager.nonce = nonce;
    rate_manager.id = id;
    rate_manager.manager = args.manager;
    rate_manager.fee_recipient = args.fee_recipient;
    rate_manager.max_fee = args.max_fee;
    rate_manager.fee = args.fee;
    rate_manager.min_liquidity = args.min_liquidity;
    rate_manager.name = args.name;
    rate_manager.uri = args.uri;
    rate_manager.bump = ctx.bumps.rate_manager;
    ctx.accounts.config.next_manager_id =
        nonce.checked_add(1).ok_or(Zkp2pError::ArithmeticOverflow)?;
    Ok(())
}

/// Accounts for manager-authorized updates that do not create child PDAs.
#[derive(Accounts)]
pub struct ManageRateManager<'info> {
    /// Current manager authority.
    pub manager: Signer<'info>,
    /// Manager state.
    #[account(
        mut,
        seeds = [RATE_MANAGER_SEED, rate_manager.config.as_ref(), &rate_manager.nonce.to_le_bytes()],
        bump = rate_manager.bump,
        constraint = rate_manager.manager == manager.key() @ Zkp2pError::Unauthorized
    )]
    pub rate_manager: Account<'info, RateManager>,
}

/// Updates mutable manager identity and metadata fields.
pub fn handle_set_rate_manager_config(
    ctx: Context<ManageRateManager>,
    manager: Pubkey,
    fee_recipient: Option<Pubkey>,
    name: String,
    uri: String,
) -> Result<()> {
    require!(manager != Pubkey::default(), Zkp2pError::ZeroAddress);
    require!(name.len() <= 64, Zkp2pError::AmountAboveMaximum);
    require!(uri.len() <= 200, Zkp2pError::AmountAboveMaximum);
    if ctx.accounts.rate_manager.fee > 0 {
        require!(fee_recipient.is_some(), Zkp2pError::ZeroAddress);
    }
    if let Some(recipient) = fee_recipient {
        require!(recipient != Pubkey::default(), Zkp2pError::ZeroAddress);
    }
    let state = &mut ctx.accounts.rate_manager;
    state.manager = manager;
    state.fee_recipient = fee_recipient;
    state.name = name;
    state.uri = uri;
    Ok(())
}

/// Updates the current fee within the immutable manager ceiling.
pub fn handle_set_manager_fee(ctx: Context<ManageRateManager>, fee: u128) -> Result<()> {
    require!(
        fee <= ctx.accounts.rate_manager.max_fee,
        Zkp2pError::FeeExceedsMaximum
    );
    if fee > 0 {
        require!(
            ctx.accounts.rate_manager.fee_recipient.is_some(),
            Zkp2pError::ZeroAddress
        );
    }
    ctx.accounts.rate_manager.fee = fee;
    Ok(())
}

/// Updates the opt-in liquidity floor for future deposits.
pub fn handle_set_manager_min_liquidity(
    ctx: Context<ManageRateManager>,
    min_liquidity: u64,
) -> Result<()> {
    ctx.accounts.rate_manager.min_liquidity = min_liquidity;
    Ok(())
}

/// Accounts for setting one payment/currency rate.
#[derive(Accounts)]
#[instruction(payment_method: [u8; 32], currency: [u8; 32])]
pub struct SetManagerRate<'info> {
    /// Current manager authority and rent payer.
    #[account(mut)]
    pub manager: Signer<'info>,
    /// Parent manager state.
    #[account(
        seeds = [RATE_MANAGER_SEED, rate_manager.config.as_ref(), &rate_manager.nonce.to_le_bytes()],
        bump = rate_manager.bump,
        constraint = rate_manager.manager == manager.key() @ Zkp2pError::Unauthorized
    )]
    pub rate_manager: Account<'info, RateManager>,
    /// One manager/method/currency rate PDA.
    #[account(
        init_if_needed,
        payer = manager,
        space = 8 + RateEntry::INIT_SPACE,
        seeds = [RATE_ENTRY_SEED, rate_manager.key().as_ref(), &payment_method, &currency],
        bump
    )]
    pub rate_entry: Account<'info, RateEntry>,
    /// System program used on first rate creation.
    pub system_program: Program<'info, System>,
}

/// Sets or disables one manager rate; zero is the canonical disabled value.
pub fn handle_set_manager_rate(
    ctx: Context<SetManagerRate>,
    payment_method: [u8; 32],
    currency: [u8; 32],
    rate: u128,
) -> Result<()> {
    require!(payment_method != [0; 32], Zkp2pError::ZeroValue);
    require!(currency != [0; 32], Zkp2pError::ZeroValue);
    let entry = &mut ctx.accounts.rate_entry;
    if entry.rate_manager == Pubkey::default() {
        entry.rate_manager = ctx.accounts.rate_manager.key();
        entry.payment_method = payment_method;
        entry.currency = currency;
        entry.bump = ctx.bumps.rate_entry;
    }
    entry.rate = rate;
    Ok(())
}
