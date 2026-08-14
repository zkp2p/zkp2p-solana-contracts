//! StakeVault user custody and delegation instructions.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::{
    constants::{
        CLAIM_BALANCE_SEED, STAKE_LOCK_SEED, STAKE_POSITION_SEED, STAKE_SELECTION_SEED,
        STAKE_TOKEN_VAULT_SEED, STAKE_VAULT_CONFIG_SEED, TAKER_AUTHORIZATION_SEED,
    },
    error::Zkp2pError,
    state::{
        ClaimBalance, StakeLock, StakePosition, StakeSelection, StakeVaultConfig,
        TakerAuthorization,
    },
};

/// Accounts for creating the one canonical stake-custody token account.
#[derive(Accounts)]
pub struct InitializeStakeTokenVault<'info> {
    /// Permissionless rent payer.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// Canonical stake-vault authority.
    #[account(seeds = [STAKE_VAULT_CONFIG_SEED], bump = vault.bump)]
    pub vault: Account<'info, StakeVaultConfig>,
    /// Canonical stake mint.
    #[account(address = vault.stake_mint)]
    pub mint: InterfaceAccount<'info, Mint>,
    /// One canonical custody account for all stake and claim liabilities.
    #[account(
        init,
        payer = payer,
        seeds = [STAKE_TOKEN_VAULT_SEED, vault.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault,
        token::token_program = token_program
    )]
    pub vault_token: InterfaceAccount<'info, TokenAccount>,
    /// SPL Token or Token-2022 program.
    pub token_program: Interface<'info, TokenInterface>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Creates the canonical stake custody account; Anchor performs the complete initialization.
pub fn handle_initialize_stake_token_vault(_ctx: Context<InitializeStakeTokenVault>) -> Result<()> {
    Ok(())
}

/// Accounts for depositing canonical stake principal.
#[derive(Accounts)]
pub struct DepositStake<'info> {
    /// Stake owner and token authority.
    #[account(mut)]
    pub owner: Signer<'info>,
    /// Canonical stake-vault configuration and token authority PDA.
    #[account(mut, seeds = [STAKE_VAULT_CONFIG_SEED], bump = vault.bump)]
    pub vault: Box<Account<'info, StakeVaultConfig>>,
    /// Aggregate owner position, created on first deposit.
    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + StakePosition::INIT_SPACE,
        seeds = [STAKE_POSITION_SEED, vault.key().as_ref(), owner.key().as_ref()],
        bump
    )]
    pub position: Box<Account<'info, StakePosition>>,
    /// Canonical stake mint.
    #[account(address = vault.stake_mint)]
    pub mint: InterfaceAccount<'info, Mint>,
    /// Owner's source token account.
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: InterfaceAccount<'info, TokenAccount>,
    /// Program-owned stake custody account.
    #[account(
        mut,
        seeds = [STAKE_TOKEN_VAULT_SEED, vault.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault,
        token::token_program = token_program
    )]
    pub vault_token: InterfaceAccount<'info, TokenAccount>,
    /// SPL Token or Token-2022 program.
    pub token_program: Interface<'info, TokenInterface>,
    /// System program used on first deposit.
    pub system_program: Program<'info, System>,
}

/// Deposits an exact nonzero amount as caller-owned free stake.
pub fn handle_deposit_stake(ctx: Context<DepositStake>, amount: u64) -> Result<()> {
    require!(amount > 0, Zkp2pError::ZeroValue);

    let balance_before = ctx.accounts.vault_token.amount;
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.owner_token.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.vault_token.to_account_info(),
                authority: ctx.accounts.owner.to_account_info(),
            },
        ),
        amount,
        ctx.accounts.mint.decimals,
    )?;
    ctx.accounts.vault_token.reload()?;
    let received = ctx
        .accounts
        .vault_token
        .amount
        .checked_sub(balance_before)
        .ok_or(Zkp2pError::InvalidTokenBalanceDelta)?;
    require!(received == amount, Zkp2pError::InvalidTokenBalanceDelta);

    let position = &mut ctx.accounts.position;
    if position.owner == Pubkey::default() {
        position.vault = ctx.accounts.vault.key();
        position.owner = ctx.accounts.owner.key();
        position.bump = ctx.bumps.position;
    }
    position.balance = position
        .balance
        .checked_add(amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    ctx.accounts.vault.total_staked = ctx
        .accounts
        .vault
        .total_staked
        .checked_add(amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    Ok(())
}

/// Accounts for withdrawing caller-owned free stake.
#[derive(Accounts)]
pub struct WithdrawStake<'info> {
    /// Stake owner and destination token authority.
    pub owner: Signer<'info>,
    /// Canonical stake-vault configuration and token authority PDA.
    #[account(mut, seeds = [STAKE_VAULT_CONFIG_SEED], bump = vault.bump)]
    pub vault: Account<'info, StakeVaultConfig>,
    /// Caller's aggregate stake position.
    #[account(
        mut,
        seeds = [STAKE_POSITION_SEED, vault.key().as_ref(), owner.key().as_ref()],
        bump = position.bump,
        has_one = vault,
        constraint = position.owner == owner.key() @ Zkp2pError::Unauthorized
    )]
    pub position: Account<'info, StakePosition>,
    /// Canonical stake mint.
    #[account(address = vault.stake_mint)]
    pub mint: InterfaceAccount<'info, Mint>,
    /// Program-owned stake custody account.
    #[account(
        mut,
        seeds = [STAKE_TOKEN_VAULT_SEED, vault.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault,
        token::token_program = token_program
    )]
    pub vault_token: InterfaceAccount<'info, TokenAccount>,
    /// Owner's destination token account.
    #[account(mut, token::mint = mint, token::authority = owner, token::token_program = token_program)]
    pub owner_token: InterfaceAccount<'info, TokenAccount>,
    /// SPL Token or Token-2022 program.
    pub token_program: Interface<'info, TokenInterface>,
}

/// Withdraws only caller-owned free principal.
pub fn handle_withdraw_stake(ctx: Context<WithdrawStake>, amount: u64) -> Result<()> {
    require!(amount > 0, Zkp2pError::ZeroValue);
    let free = ctx
        .accounts
        .position
        .free()
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    require!(free >= amount, Zkp2pError::InsufficientFreeStake);

    ctx.accounts.position.balance = ctx
        .accounts
        .position
        .balance
        .checked_sub(amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    ctx.accounts.vault.total_staked = ctx
        .accounts
        .vault
        .total_staked
        .checked_sub(amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;

    let signer_seeds: &[&[u8]] = &[STAKE_VAULT_CONFIG_SEED, &[ctx.accounts.vault.bump]];
    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.vault_token.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.owner_token.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
        ctx.accounts.mint.decimals,
    )
}

/// Accounts for withdrawing a beneficiary's complete claim balance.
#[derive(Accounts)]
pub struct ClaimStake<'info> {
    /// Claim beneficiary and destination authority.
    pub beneficiary: Signer<'info>,
    /// Canonical stake-vault configuration and token authority PDA.
    #[account(mut, seeds = [STAKE_VAULT_CONFIG_SEED], bump = vault.bump)]
    pub vault: Account<'info, StakeVaultConfig>,
    /// Complete beneficiary claim balance.
    #[account(
        mut,
        seeds = [CLAIM_BALANCE_SEED, vault.key().as_ref(), beneficiary.key().as_ref()],
        bump = claim.bump,
        has_one = vault,
        constraint = claim.beneficiary == beneficiary.key() @ Zkp2pError::Unauthorized
    )]
    pub claim: Account<'info, ClaimBalance>,
    /// Canonical stake mint.
    #[account(address = vault.stake_mint)]
    pub mint: InterfaceAccount<'info, Mint>,
    /// Program-owned stake custody account.
    #[account(
        mut,
        seeds = [STAKE_TOKEN_VAULT_SEED, vault.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault,
        token::token_program = token_program
    )]
    pub vault_token: InterfaceAccount<'info, TokenAccount>,
    /// Beneficiary destination token account.
    #[account(mut, token::mint = mint, token::authority = beneficiary, token::token_program = token_program)]
    pub beneficiary_token: InterfaceAccount<'info, TokenAccount>,
    /// SPL Token or Token-2022 program.
    pub token_program: Interface<'info, TokenInterface>,
}

/// Withdraws the caller's full, nonzero, immediately claimable balance.
pub fn handle_claim_stake(ctx: Context<ClaimStake>) -> Result<()> {
    let amount = ctx.accounts.claim.amount;
    require!(amount > 0, Zkp2pError::ZeroValue);
    ctx.accounts.claim.amount = 0;
    ctx.accounts.vault.total_claimable = ctx
        .accounts
        .vault
        .total_claimable
        .checked_sub(amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;

    let signer_seeds: &[&[u8]] = &[STAKE_VAULT_CONFIG_SEED, &[ctx.accounts.vault.bump]];
    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.vault_token.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.beneficiary_token.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            &[signer_seeds],
        ),
        amount,
        ctx.accounts.mint.decimals,
    )
}

/// Accounts for changing one owner-to-taker authorization.
#[derive(Accounts)]
pub struct SetTakerAuthorization<'info> {
    /// Stake owner granting or revoking authority.
    #[account(mut)]
    pub stake_owner: Signer<'info>,
    /// Taker whose authority changes.
    /// CHECK: identity only; cannot authorize itself or the zero key.
    pub taker: UncheckedAccount<'info>,
    /// Owner/taker authorization PDA.
    #[account(
        init_if_needed,
        payer = stake_owner,
        space = 8 + TakerAuthorization::INIT_SPACE,
        seeds = [TAKER_AUTHORIZATION_SEED, stake_owner.key().as_ref(), taker.key().as_ref()],
        bump
    )]
    pub authorization: Account<'info, TakerAuthorization>,
    /// Taker selection PDA, created so revocation can always clear it atomically.
    #[account(
        init_if_needed,
        payer = stake_owner,
        space = 8 + StakeSelection::INIT_SPACE,
        seeds = [STAKE_SELECTION_SEED, taker.key().as_ref()],
        bump
    )]
    pub selection: Account<'info, StakeSelection>,
    /// System program used when either PDA is first created.
    pub system_program: Program<'info, System>,
}

/// Grants or revokes a taker's ability to select the caller's stake.
pub fn handle_set_taker_authorization(
    ctx: Context<SetTakerAuthorization>,
    authorized: bool,
) -> Result<()> {
    require!(
        ctx.accounts.taker.key() != Pubkey::default()
            && ctx.accounts.taker.key() != ctx.accounts.stake_owner.key(),
        Zkp2pError::ZeroAddress
    );
    let authorization = &mut ctx.accounts.authorization;
    if authorization.stake_owner == Pubkey::default() {
        authorization.stake_owner = ctx.accounts.stake_owner.key();
        authorization.taker = ctx.accounts.taker.key();
        authorization.bump = ctx.bumps.authorization;
    }
    authorization.authorized = authorized;

    let selection = &mut ctx.accounts.selection;
    if selection.taker == Pubkey::default() {
        selection.taker = ctx.accounts.taker.key();
        selection.selected_owner = None;
        selection.bump = ctx.bumps.selection;
    }
    if !authorized && selection.selected_owner == Some(ctx.accounts.stake_owner.key()) {
        selection.selected_owner = None;
    }
    Ok(())
}

/// Accounts for selecting an authorized third-party stake owner.
#[derive(Accounts)]
pub struct SelectStakeOwner<'info> {
    /// Taker choosing collateral.
    #[account(mut)]
    pub taker: Signer<'info>,
    /// Selected third-party owner.
    /// CHECK: identity is bound by the authorization PDA seeds.
    pub stake_owner: UncheckedAccount<'info>,
    /// Live owner/taker authorization.
    #[account(
        seeds = [TAKER_AUTHORIZATION_SEED, stake_owner.key().as_ref(), taker.key().as_ref()],
        bump = authorization.bump,
        constraint = authorization.stake_owner == stake_owner.key() @ Zkp2pError::UnauthorizedStakeOwner,
        constraint = authorization.taker == taker.key() @ Zkp2pError::UnauthorizedStakeOwner,
        constraint = authorization.authorized @ Zkp2pError::UnauthorizedStakeOwner
    )]
    pub authorization: Account<'info, TakerAuthorization>,
    /// Taker selection PDA.
    #[account(
        init_if_needed,
        payer = taker,
        space = 8 + StakeSelection::INIT_SPACE,
        seeds = [STAKE_SELECTION_SEED, taker.key().as_ref()],
        bump
    )]
    pub selection: Account<'info, StakeSelection>,
    /// System program used when the selection is first created.
    pub system_program: Program<'info, System>,
}

/// Selects a currently authorizing third-party owner.
pub fn handle_select_stake_owner(ctx: Context<SelectStakeOwner>) -> Result<()> {
    require!(
        ctx.accounts.stake_owner.key() != Pubkey::default()
            && ctx.accounts.stake_owner.key() != ctx.accounts.taker.key(),
        Zkp2pError::ZeroAddress
    );
    let selection = &mut ctx.accounts.selection;
    if selection.taker == Pubkey::default() {
        selection.taker = ctx.accounts.taker.key();
        selection.bump = ctx.bumps.selection;
    }
    selection.selected_owner = Some(ctx.accounts.stake_owner.key());
    Ok(())
}

/// Accounts for restoring a taker's implicit self-staking fallback.
#[derive(Accounts)]
pub struct ClearStakeOwner<'info> {
    /// Taker clearing its selection.
    pub taker: Signer<'info>,
    /// Existing taker selection PDA.
    #[account(
        mut,
        seeds = [STAKE_SELECTION_SEED, taker.key().as_ref()],
        bump = selection.bump,
        constraint = selection.taker == taker.key() @ Zkp2pError::Unauthorized
    )]
    pub selection: Account<'info, StakeSelection>,
}

/// Restores caller-owned stake as the live collateral source.
pub fn handle_clear_stake_owner(ctx: Context<ClearStakeOwner>) -> Result<()> {
    ctx.accounts.selection.selected_owner = None;
    Ok(())
}

/// Controller-defined lock creation arguments.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControllerLockArgs {
    /// Globally unique controller-defined lock identifier.
    pub lock_id: [u8; 32],
    /// Principal to lock.
    pub amount: u64,
    /// Strictly future maturity boundary.
    pub matures_at: i64,
}

/// Accounts for a controller locking existing stake principal.
#[derive(Accounts)]
#[instruction(args: ControllerLockArgs)]
pub struct ControllerLockStake<'info> {
    /// Current lock-policy controller and rent payer.
    #[account(mut, address = vault.controller)]
    pub controller: Signer<'info>,
    /// Canonical stake vault.
    #[account(seeds = [STAKE_VAULT_CONFIG_SEED], bump = vault.bump)]
    pub vault: Box<Account<'info, StakeVaultConfig>>,
    /// Collateral owner identity.
    /// CHECK: Bound to the position PDA.
    pub stake_owner: UncheckedAccount<'info>,
    /// Existing owner position.
    #[account(
        mut,
        seeds = [STAKE_POSITION_SEED, vault.key().as_ref(), stake_owner.key().as_ref()],
        bump = position.bump,
        constraint = position.owner == stake_owner.key() @ Zkp2pError::UnauthorizedStakeOwner
    )]
    pub position: Box<Account<'info, StakePosition>>,
    /// New globally unique lock.
    #[account(
        init,
        payer = controller,
        space = 8 + StakeLock::INIT_SPACE,
        seeds = [STAKE_LOCK_SEED, vault.key().as_ref(), &args.lock_id],
        bump
    )]
    pub stake_lock: Box<Account<'info, StakeLock>>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Locks existing free stake under a strictly future controller-defined maturity.
pub fn handle_controller_lock_stake(
    ctx: Context<ControllerLockStake>,
    args: ControllerLockArgs,
) -> Result<()> {
    require!(args.lock_id != [0; 32], Zkp2pError::ZeroValue);
    require!(
        args.matures_at > Clock::get()?.unix_timestamp,
        Zkp2pError::InvalidMaturity
    );
    ctx.accounts.position.lock(args.amount)?;
    let lock = &mut ctx.accounts.stake_lock;
    lock.vault = ctx.accounts.vault.key();
    lock.id = args.lock_id;
    lock.stake_owner = ctx.accounts.stake_owner.key();
    lock.amount = args.amount;
    lock.matures_at = args.matures_at;
    lock.bump = ctx.bumps.stake_lock;
    Ok(())
}

/// Accounts for controller-funded stake lock creation.
#[derive(Accounts)]
#[instruction(args: ControllerLockArgs)]
pub struct ControllerFundLock<'info> {
    /// Current controller, token authority, and rent payer.
    #[account(mut, address = vault.controller)]
    pub controller: Signer<'info>,
    /// Canonical mutable stake vault.
    #[account(mut, seeds = [STAKE_VAULT_CONFIG_SEED], bump = vault.bump)]
    pub vault: Box<Account<'info, StakeVaultConfig>>,
    /// Beneficial stake owner identity.
    /// CHECK: Identity only and bound to the position PDA.
    pub stake_owner: UncheckedAccount<'info>,
    /// Owner position, created if needed.
    #[account(
        init_if_needed,
        payer = controller,
        space = 8 + StakePosition::INIT_SPACE,
        seeds = [STAKE_POSITION_SEED, vault.key().as_ref(), stake_owner.key().as_ref()],
        bump
    )]
    pub position: Box<Account<'info, StakePosition>>,
    /// New globally unique lock.
    #[account(
        init,
        payer = controller,
        space = 8 + StakeLock::INIT_SPACE,
        seeds = [STAKE_LOCK_SEED, vault.key().as_ref(), &args.lock_id],
        bump
    )]
    pub stake_lock: Box<Account<'info, StakeLock>>,
    /// Canonical stake mint.
    #[account(address = vault.stake_mint)]
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    /// Controller source tokens.
    #[account(mut, token::mint = mint, token::authority = controller, token::token_program = token_program)]
    pub controller_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Program stake custody.
    #[account(
        mut,
        seeds = [STAKE_TOKEN_VAULT_SEED, vault.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = vault,
        token::token_program = token_program
    )]
    pub vault_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// SPL Token or Token-2022 program.
    pub token_program: Interface<'info, TokenInterface>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Atomically adopts exact controller tokens as owner stake and locks the complete amount.
pub fn handle_controller_fund_lock(
    ctx: Context<ControllerFundLock>,
    args: ControllerLockArgs,
) -> Result<()> {
    require!(args.lock_id != [0; 32], Zkp2pError::ZeroValue);
    require!(args.amount > 0, Zkp2pError::ZeroValue);
    require!(
        args.matures_at > Clock::get()?.unix_timestamp,
        Zkp2pError::InvalidMaturity
    );
    let before = ctx.accounts.vault_token.amount;
    token_interface::transfer_checked(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            TransferChecked {
                from: ctx.accounts.controller_token.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.vault_token.to_account_info(),
                authority: ctx.accounts.controller.to_account_info(),
            },
        ),
        args.amount,
        ctx.accounts.mint.decimals,
    )?;
    ctx.accounts.vault_token.reload()?;
    require!(
        ctx.accounts.vault_token.amount.checked_sub(before) == Some(args.amount),
        Zkp2pError::InvalidTokenBalanceDelta
    );

    let position = &mut ctx.accounts.position;
    if position.owner == Pubkey::default() {
        position.vault = ctx.accounts.vault.key();
        position.owner = ctx.accounts.stake_owner.key();
        position.bump = ctx.bumps.position;
    }
    position.balance = position
        .balance
        .checked_add(args.amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    position.lock(args.amount)?;
    ctx.accounts.vault.total_staked = ctx
        .accounts
        .vault
        .total_staked
        .checked_add(args.amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;

    let lock = &mut ctx.accounts.stake_lock;
    lock.vault = ctx.accounts.vault.key();
    lock.id = args.lock_id;
    lock.stake_owner = ctx.accounts.stake_owner.key();
    lock.amount = args.amount;
    lock.matures_at = args.matures_at;
    lock.bump = ctx.bumps.stake_lock;
    Ok(())
}

/// Accounts for controller resize, increase, unlock, or single-claim resolution.
#[derive(Accounts)]
#[instruction(lock_id: [u8; 32])]
pub struct ManageStakeLock<'info> {
    /// Current lock-policy controller and closed-account rent recipient.
    #[account(mut, address = vault.controller)]
    pub controller: Signer<'info>,
    /// Canonical stake vault.
    #[account(seeds = [STAKE_VAULT_CONFIG_SEED], bump = vault.bump)]
    pub vault: Account<'info, StakeVaultConfig>,
    /// Lock owner's position.
    #[account(
        mut,
        seeds = [STAKE_POSITION_SEED, vault.key().as_ref(), stake_lock.stake_owner.as_ref()],
        bump = position.bump
    )]
    pub position: Account<'info, StakePosition>,
    /// Existing controller lock.
    #[account(
        mut,
        seeds = [STAKE_LOCK_SEED, vault.key().as_ref(), &lock_id],
        bump = stake_lock.bump,
        constraint = stake_lock.id == lock_id @ Zkp2pError::LockNotFound
    )]
    pub stake_lock: Account<'info, StakeLock>,
}

/// Increases a pre-maturity lock from the owner's free stake.
pub fn handle_increase_stake_lock(
    ctx: Context<ManageStakeLock>,
    _lock_id: [u8; 32],
    additional_amount: u64,
) -> Result<()> {
    require!(additional_amount > 0, Zkp2pError::ZeroValue);
    require!(
        Clock::get()?.unix_timestamp < ctx.accounts.stake_lock.matures_at,
        Zkp2pError::LockAlreadyMatured
    );
    ctx.accounts.position.lock(additional_amount)?;
    ctx.accounts.stake_lock.amount = ctx
        .accounts
        .stake_lock
        .amount
        .checked_add(additional_amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    Ok(())
}

/// Shrinks and re-times a pre-maturity lock without permitting an increase.
pub fn handle_resize_stake_lock(
    ctx: Context<ManageStakeLock>,
    _lock_id: [u8; 32],
    new_amount: u64,
    new_matures_at: i64,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(
        now < ctx.accounts.stake_lock.matures_at,
        Zkp2pError::LockAlreadyMatured
    );
    require!(new_amount > 0, Zkp2pError::ZeroValue);
    require!(
        new_amount <= ctx.accounts.stake_lock.amount,
        Zkp2pError::AmountAboveMaximum
    );
    require!(new_matures_at > now, Zkp2pError::InvalidMaturity);
    let unlocked = ctx
        .accounts
        .stake_lock
        .amount
        .checked_sub(new_amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    if unlocked > 0 {
        ctx.accounts.position.unlock(unlocked)?;
    }
    ctx.accounts.stake_lock.amount = new_amount;
    ctx.accounts.stake_lock.matures_at = new_matures_at;
    Ok(())
}

/// Unlocks a complete lock before or after maturity and returns principal to free stake.
pub fn handle_controller_unlock_stake(
    ctx: Context<ManageStakeLock>,
    _lock_id: [u8; 32],
) -> Result<()> {
    let amount = ctx.accounts.stake_lock.amount;
    ctx.accounts.position.unlock(amount)?;
    ctx.accounts
        .stake_lock
        .close(ctx.accounts.controller.to_account_info())
}

/// One controller-directed beneficiary allocation.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct StakeClaim {
    /// Nonzero beneficiary.
    pub beneficiary: Pubkey,
    /// Nonzero claim amount.
    pub amount: u64,
}

/// Accounts for permissionlessly preparing a beneficiary claim PDA.
#[derive(Accounts)]
pub struct InitializeClaimBalance<'info> {
    /// Rent payer.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// Canonical stake vault.
    #[account(seeds = [STAKE_VAULT_CONFIG_SEED], bump = vault.bump)]
    pub vault: Account<'info, StakeVaultConfig>,
    /// Future claim beneficiary.
    /// CHECK: Identity only and bound to the claim PDA.
    pub beneficiary: UncheckedAccount<'info>,
    /// Prepared claim balance.
    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + ClaimBalance::INIT_SPACE,
        seeds = [CLAIM_BALANCE_SEED, vault.key().as_ref(), beneficiary.key().as_ref()],
        bump
    )]
    pub claim: Account<'info, ClaimBalance>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Creates an empty canonical claim balance for later controller resolution.
pub fn handle_initialize_claim_balance(ctx: Context<InitializeClaimBalance>) -> Result<()> {
    require!(
        ctx.accounts.beneficiary.key() != Pubkey::default(),
        Zkp2pError::InvalidClaim
    );
    let claim = &mut ctx.accounts.claim;
    if claim.beneficiary == Pubkey::default() {
        claim.vault = ctx.accounts.vault.key();
        claim.beneficiary = ctx.accounts.beneficiary.key();
        claim.amount = 0;
        claim.bump = ctx.bumps.claim;
    }
    Ok(())
}

/// Accounts for resolving one lock into one or more prepared beneficiary claims.
#[derive(Accounts)]
#[instruction(lock_id: [u8; 32])]
pub struct ResolveStakeLock<'info> {
    /// Current controller and closed-account rent recipient.
    #[account(mut, address = vault.controller)]
    pub controller: Signer<'info>,
    /// Canonical mutable stake vault.
    #[account(mut, seeds = [STAKE_VAULT_CONFIG_SEED], bump = vault.bump)]
    pub vault: Account<'info, StakeVaultConfig>,
    /// Lock owner's aggregate stake.
    #[account(
        mut,
        seeds = [STAKE_POSITION_SEED, vault.key().as_ref(), stake_lock.stake_owner.as_ref()],
        bump = position.bump
    )]
    pub position: Account<'info, StakePosition>,
    /// Existing controller lock.
    #[account(
        mut,
        seeds = [STAKE_LOCK_SEED, vault.key().as_ref(), &lock_id],
        bump = stake_lock.bump
    )]
    pub stake_lock: Account<'info, StakeLock>,
}

/// Resolves at most the complete lock into prepared beneficiary claims and frees any remainder.
pub fn handle_resolve_stake_lock<'info>(
    ctx: Context<'info, ResolveStakeLock<'info>>,
    _lock_id: [u8; 32],
    claims: Vec<StakeClaim>,
) -> Result<()> {
    require!(!claims.is_empty(), Zkp2pError::InvalidClaim);
    require!(
        claims.len() == ctx.remaining_accounts.len(),
        Zkp2pError::ArrayLengthMismatch
    );
    let mut total_claims = 0_u64;
    for (index, allocation) in claims.iter().enumerate() {
        require!(
            allocation.beneficiary != Pubkey::default() && allocation.amount > 0,
            Zkp2pError::InvalidClaim
        );
        require!(
            !claims
                .get(..index)
                .ok_or(Zkp2pError::ArithmeticOverflow)?
                .iter()
                .any(|prior| prior.beneficiary == allocation.beneficiary),
            Zkp2pError::InvalidClaim
        );
        total_claims = total_claims
            .checked_add(allocation.amount)
            .ok_or(Zkp2pError::ArithmeticOverflow)?;
    }
    let lock_amount = ctx.accounts.stake_lock.amount;
    ctx.accounts.position.resolve(lock_amount, total_claims)?;
    ctx.accounts.vault.total_staked = ctx
        .accounts
        .vault
        .total_staked
        .checked_sub(total_claims)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    ctx.accounts.vault.total_claimable = ctx
        .accounts
        .vault
        .total_claimable
        .checked_add(total_claims)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;

    for (allocation, account_info) in claims.iter().zip(ctx.remaining_accounts.iter()) {
        require!(account_info.is_writable, Zkp2pError::Unauthorized);
        let expected = Pubkey::find_program_address(
            &[
                CLAIM_BALANCE_SEED,
                ctx.accounts.vault.key().as_ref(),
                allocation.beneficiary.as_ref(),
            ],
            &crate::ID,
        )
        .0;
        require_keys_eq!(account_info.key(), expected, Zkp2pError::InvalidClaim);
        let mut claim = Account::<ClaimBalance>::try_from(account_info)?;
        require_keys_eq!(
            claim.vault,
            ctx.accounts.vault.key(),
            Zkp2pError::InvalidClaim
        );
        require_keys_eq!(
            claim.beneficiary,
            allocation.beneficiary,
            Zkp2pError::InvalidClaim
        );
        claim.amount = claim
            .amount
            .checked_add(allocation.amount)
            .ok_or(Zkp2pError::ArithmeticOverflow)?;
        claim.exit(&crate::ID)?;
    }
    ctx.accounts
        .stake_lock
        .close(ctx.accounts.controller.to_account_info())
}
