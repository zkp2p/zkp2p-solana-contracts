//! Proof fulfillment and depositor manual-release settlement paths.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

use crate::{
    constants::{
        DEPOSIT_SEED, ESCROW_INTENT_LOCK_SEED, INTENT_PAYMENT_BINDING_SEED, INTENT_SEED,
        ORCHESTRATOR_CONFIG_SEED, PAYMENT_BINDING_SEED, TAKER_INTENT_STATE_SEED,
        VERIFIER_CONFIG_SEED,
    },
    error::Zkp2pError,
    instructions::verify_payment_attestation,
    math::precise_mul_floor,
    state::{
        Deposit, DisputeIntent, EscrowIntentLock, Intent, IntentPaymentBinding, OrchestratorConfig,
        PaymentBinding, StakeLock, StakePosition, StakeVaultConfig, TakerIntentState,
        VerifierConfig,
    },
};

use super::PaymentAttestation;

/// Fulfillment payload with a client-derived nullifier cross-check.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct FulfillIntentArgs {
    /// Threshold-signed payment attestation.
    pub attestation: PaymentAttestation,
    /// Expected keccak(method || payment ID), used to seed immutable bindings.
    pub expected_nullifier: [u8; 32],
}

/// Token accounts grouped to keep the SVM account decoder below its fixed stack-frame limit.
#[derive(Accounts)]
pub struct SettlementTokenAccounts<'info> {
    /// Canonical escrow mint.
    pub mint: Box<InterfaceAccount<'info, Mint>>,
    /// Deposit token custody.
    #[account(mut)]
    pub deposit_vault: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Final recipient token account.
    #[account(mut)]
    pub recipient_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Protocol fee recipient token account.
    #[account(mut)]
    pub protocol_fee_token: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Manager fee recipient token account when a nonzero manager fee was snapshotted.
    #[account(mut)]
    pub manager_fee_token: Option<Box<InterfaceAccount<'info, TokenAccount>>>,
    /// SPL Token or Token-2022 program.
    pub token_program: Interface<'info, TokenInterface>,
}

/// Optional dispute accounts grouped independently from the settlement core.
#[derive(Accounts)]
pub struct SettlementDisputeAccounts<'info> {
    /// Covered dispute state, if any.
    #[account(mut)]
    pub dispute_intent: Option<Box<Account<'info, DisputeIntent>>>,
    /// Stake vault used by covered settlement.
    pub stake_vault: Option<Box<Account<'info, StakeVaultConfig>>>,
    /// Selected stake owner's position.
    #[account(mut)]
    pub stake_position: Option<Box<Account<'info, StakePosition>>>,
    /// Active collateral lock resized by settlement.
    #[account(mut)]
    pub stake_lock: Option<Box<Account<'info, StakeLock>>>,
}

/// Accounts for proof fulfillment, fee distribution, and lifecycle settlement.
#[derive(Accounts)]
#[instruction(args: FulfillIntentArgs)]
pub struct FulfillIntent<'info> {
    /// Permissionless proof submitter and binding rent payer.
    #[account(mut)]
    pub caller: Signer<'info>,
    /// Original intent owner receives closed-account rent.
    #[account(mut, address = intent.owner)]
    pub owner_rent: SystemAccount<'info>,
    /// Canonical orchestrator configuration.
    #[account(
        seeds = [ORCHESTRATOR_CONFIG_SEED],
        bump = orchestrator.bump,
        constraint = !orchestrator.paused @ Zkp2pError::Paused
    )]
    pub orchestrator: Box<Account<'info, OrchestratorConfig>>,
    /// Canonical unified verifier configuration.
    #[account(
        seeds = [VERIFIER_CONFIG_SEED],
        bump = verifier.bump,
        constraint = orchestrator.verifier_config == verifier.key() @ Zkp2pError::Unauthorized
    )]
    pub verifier: Box<Account<'info, VerifierConfig>>,
    /// Active intent, closed only after every effect succeeds.
    #[account(
        mut,
        close = owner_rent,
        seeds = [INTENT_SEED, orchestrator.key().as_ref(), &intent.nonce.to_le_bytes()],
        bump = intent.bump,
        constraint = intent.intent_hash == args.attestation.intent_hash @ Zkp2pError::IntentSnapshotMismatch
    )]
    pub intent: Box<Account<'info, Intent>>,
    /// Parent maker deposit.
    #[account(mut, address = intent.deposit)]
    pub deposit: Box<Account<'info, Deposit>>,
    /// Paired escrow lock.
    #[account(
        mut,
        close = owner_rent,
        seeds = [ESCROW_INTENT_LOCK_SEED, deposit.key().as_ref(), &intent.intent_hash],
        bump = intent_lock.bump,
        constraint = intent_lock.intent_hash == intent.intent_hash @ Zkp2pError::IntentSnapshotMismatch
    )]
    pub intent_lock: Box<Account<'info, EscrowIntentLock>>,
    /// Taker live-intent counter.
    #[account(
        mut,
        seeds = [TAKER_INTENT_STATE_SEED, orchestrator.key().as_ref(), intent.owner.as_ref()],
        bump = taker_state.bump
    )]
    pub taker_state: Box<Account<'info, TakerIntentState>>,
    /// Immutable payment-nullifier binding.
    #[account(
        init,
        payer = caller,
        space = 8 + PaymentBinding::INIT_SPACE,
        seeds = [PAYMENT_BINDING_SEED, verifier.key().as_ref(), &args.expected_nullifier],
        bump
    )]
    pub payment_binding: Box<Account<'info, PaymentBinding>>,
    /// Immutable reverse intent binding.
    #[account(
        init,
        payer = caller,
        space = 8 + IntentPaymentBinding::INIT_SPACE,
        seeds = [INTENT_PAYMENT_BINDING_SEED, verifier.key().as_ref(), &intent.intent_hash],
        bump
    )]
    pub intent_payment_binding: Box<Account<'info, IntentPaymentBinding>>,
    /// Token transfer accounts.
    pub tokens: SettlementTokenAccounts<'info>,
    /// Optional dispute lifecycle accounts.
    pub dispute: SettlementDisputeAccounts<'info>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Verifies and consumes one payment, then settles principal and fees atomically.
pub fn handle_fulfill_intent<'info>(
    ctx: Context<'info, FulfillIntent<'info>>,
    args: FulfillIntentArgs,
) -> Result<()> {
    let nullifier = verify_payment_attestation(
        ctx.accounts.verifier.key(),
        &ctx.accounts.verifier,
        &ctx.accounts.intent,
        &args.attestation,
    )?;
    require!(
        nullifier == args.expected_nullifier,
        Zkp2pError::DataHashMismatch
    );
    let release_amount = args
        .attestation
        .release_amount
        .min(ctx.accounts.intent.amount);
    settle_dispute_if_covered(
        &ctx.accounts.intent,
        ctx.accounts.dispute.dispute_intent.as_deref_mut(),
        ctx.accounts.dispute.stake_vault.as_deref(),
        ctx.accounts.dispute.stake_position.as_deref_mut(),
        ctx.accounts.dispute.stake_lock.as_deref_mut(),
        release_amount,
    )?;
    ctx.accounts
        .deposit
        .settle(ctx.accounts.intent_lock.amount, release_amount)?;

    ctx.accounts.payment_binding.verifier = ctx.accounts.verifier.key();
    ctx.accounts.payment_binding.nullifier = nullifier;
    ctx.accounts.payment_binding.intent_hash = ctx.accounts.intent.intent_hash;
    ctx.accounts.payment_binding.bump = ctx.bumps.payment_binding;
    ctx.accounts.intent_payment_binding.verifier = ctx.accounts.verifier.key();
    ctx.accounts.intent_payment_binding.intent_hash = ctx.accounts.intent.intent_hash;
    ctx.accounts.intent_payment_binding.nullifier = nullifier;
    ctx.accounts.intent_payment_binding.bump = ctx.bumps.intent_payment_binding;

    distribute_funds(
        &ctx.accounts.intent,
        &ctx.accounts.orchestrator,
        &ctx.accounts.deposit,
        &mut ctx.accounts.tokens,
        ctx.remaining_accounts,
        release_amount,
    )?;
    ctx.accounts.taker_state.active_intents = ctx
        .accounts
        .taker_state
        .active_intents
        .checked_sub(1)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    Ok(())
}

/// Accounts for a maker-authorized full manual release without a payment nullifier.
#[derive(Accounts)]
pub struct ManualRelease<'info> {
    /// Deposit owner authorizing the release.
    #[account(address = deposit.depositor)]
    pub depositor: Signer<'info>,
    /// Original taker receives closed-account rent.
    #[account(mut, address = intent.owner)]
    pub owner_rent: SystemAccount<'info>,
    /// Canonical orchestrator configuration.
    #[account(seeds = [ORCHESTRATOR_CONFIG_SEED], bump = orchestrator.bump)]
    pub orchestrator: Box<Account<'info, OrchestratorConfig>>,
    /// Active intent.
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
    /// Paired escrow lock.
    #[account(
        mut,
        close = owner_rent,
        seeds = [ESCROW_INTENT_LOCK_SEED, deposit.key().as_ref(), &intent.intent_hash],
        bump = intent_lock.bump,
        constraint = intent_lock.intent_hash == intent.intent_hash @ Zkp2pError::IntentSnapshotMismatch
    )]
    pub intent_lock: Box<Account<'info, EscrowIntentLock>>,
    /// Taker live-intent counter.
    #[account(
        mut,
        seeds = [TAKER_INTENT_STATE_SEED, orchestrator.key().as_ref(), intent.owner.as_ref()],
        bump = taker_state.bump
    )]
    pub taker_state: Box<Account<'info, TakerIntentState>>,
    /// Token transfer accounts.
    pub tokens: SettlementTokenAccounts<'info>,
    /// Optional dispute lifecycle accounts.
    pub dispute: SettlementDisputeAccounts<'info>,
}

/// Releases the complete locked amount using the same fee and lifecycle plan as proof fulfillment.
pub fn handle_manual_release<'info>(ctx: Context<'info, ManualRelease<'info>>) -> Result<()> {
    let release_amount = ctx.accounts.intent.amount;
    settle_dispute_if_covered(
        &ctx.accounts.intent,
        ctx.accounts.dispute.dispute_intent.as_deref_mut(),
        ctx.accounts.dispute.stake_vault.as_deref(),
        ctx.accounts.dispute.stake_position.as_deref_mut(),
        ctx.accounts.dispute.stake_lock.as_deref_mut(),
        release_amount,
    )?;
    ctx.accounts
        .deposit
        .settle(ctx.accounts.intent_lock.amount, release_amount)?;
    distribute_funds(
        &ctx.accounts.intent,
        &ctx.accounts.orchestrator,
        &ctx.accounts.deposit,
        &mut ctx.accounts.tokens,
        ctx.remaining_accounts,
        release_amount,
    )?;
    ctx.accounts.taker_state.active_intents = ctx
        .accounts
        .taker_state
        .active_intents
        .checked_sub(1)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    Ok(())
}

fn distribute_funds<'info>(
    intent: &Intent,
    orchestrator: &OrchestratorConfig,
    deposit: &Account<'info, Deposit>,
    tokens: &mut SettlementTokenAccounts<'info>,
    remaining_accounts: &'info [AccountInfo<'info>],
    release_amount: u64,
) -> Result<()> {
    require_keys_eq!(
        tokens.mint.key(),
        deposit.token_mint,
        Zkp2pError::IntentTokenMismatch
    );
    let expected_vault = Pubkey::find_program_address(
        &[crate::constants::DEPOSIT_VAULT_SEED, deposit.key().as_ref()],
        &crate::ID,
    )
    .0;
    require_keys_eq!(
        tokens.deposit_vault.key(),
        expected_vault,
        Zkp2pError::Unauthorized
    );
    require_keys_eq!(
        tokens.deposit_vault.mint,
        tokens.mint.key(),
        Zkp2pError::IntentTokenMismatch
    );
    require_keys_eq!(
        tokens.deposit_vault.owner,
        deposit.key(),
        Zkp2pError::Unauthorized
    );
    require_keys_eq!(
        tokens.recipient_token.mint,
        tokens.mint.key(),
        Zkp2pError::IntentTokenMismatch
    );
    require_keys_eq!(
        tokens.recipient_token.owner,
        intent.recipient,
        Zkp2pError::Unauthorized
    );
    require_keys_eq!(
        tokens.protocol_fee_token.mint,
        tokens.mint.key(),
        Zkp2pError::IntentTokenMismatch
    );
    require_keys_eq!(
        tokens.protocol_fee_token.owner,
        orchestrator.protocol_fee_recipient,
        Zkp2pError::Unauthorized
    );
    require!(
        remaining_accounts.len() == intent.referral_fees.len(),
        Zkp2pError::ArrayLengthMismatch
    );
    let protocol_fee = precise_mul_floor(release_amount, orchestrator.protocol_fee)?;
    let manager_fee = precise_mul_floor(release_amount, intent.manager_fee)?;
    let mut total_fees = protocol_fee
        .checked_add(manager_fee)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    if protocol_fee > 0 {
        transfer_exact(
            deposit,
            &mut tokens.deposit_vault,
            &mut tokens.protocol_fee_token,
            &tokens.mint,
            &tokens.token_program,
            protocol_fee,
        )?;
    }

    if manager_fee > 0 {
        let expected_recipient = intent
            .manager_fee_recipient
            .ok_or(Zkp2pError::ZeroAddress)?;
        let destination = tokens
            .manager_fee_token
            .as_deref_mut()
            .ok_or(Zkp2pError::ZeroAddress)?;
        require_keys_eq!(
            destination.mint,
            tokens.mint.key(),
            Zkp2pError::IntentTokenMismatch
        );
        require_keys_eq!(
            destination.owner,
            expected_recipient,
            Zkp2pError::Unauthorized
        );
        transfer_exact(
            deposit,
            &mut tokens.deposit_vault,
            destination,
            &tokens.mint,
            &tokens.token_program,
            manager_fee,
        )?;
    } else {
        require!(tokens.manager_fee_token.is_none(), Zkp2pError::Unauthorized);
    }

    for (fee, account_info) in intent.referral_fees.iter().zip(remaining_accounts.iter()) {
        let amount = precise_mul_floor(release_amount, fee.fee)?;
        total_fees = total_fees
            .checked_add(amount)
            .ok_or(Zkp2pError::ArithmeticOverflow)?;
        let mut destination = InterfaceAccount::<TokenAccount>::try_from(account_info)?;
        require_keys_eq!(
            destination.mint,
            tokens.mint.key(),
            Zkp2pError::IntentTokenMismatch
        );
        require_keys_eq!(destination.owner, fee.recipient, Zkp2pError::Unauthorized);
        if amount > 0 {
            transfer_exact(
                deposit,
                &mut tokens.deposit_vault,
                &mut destination,
                &tokens.mint,
                &tokens.token_program,
                amount,
            )?;
        }
    }
    let net = release_amount
        .checked_sub(total_fees)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    transfer_exact(
        deposit,
        &mut tokens.deposit_vault,
        &mut tokens.recipient_token,
        &tokens.mint,
        &tokens.token_program,
        net,
    )?;
    Ok(())
}

fn settle_dispute_if_covered(
    intent: &Intent,
    dispute: Option<&mut Account<'_, DisputeIntent>>,
    stake_vault: Option<&Account<'_, StakeVaultConfig>>,
    position: Option<&mut Account<'_, StakePosition>>,
    stake_lock: Option<&mut Account<'_, StakeLock>>,
    release_amount: u64,
) -> Result<()> {
    if !intent.dispute_covered {
        require!(
            dispute.is_none()
                && stake_vault.is_none()
                && position.is_none()
                && stake_lock.is_none(),
            Zkp2pError::Unauthorized
        );
        return Ok(());
    }
    let dispute = dispute.ok_or(Zkp2pError::DisputeProtectionDisabled)?;
    let vault = stake_vault.ok_or(Zkp2pError::DisputeProtectionDisabled)?;
    let position = position.ok_or(Zkp2pError::DisputeProtectionDisabled)?;
    let lock = stake_lock.ok_or(Zkp2pError::DisputeProtectionDisabled)?;
    let expected_dispute_config =
        Pubkey::find_program_address(&[crate::constants::DISPUTE_CONFIG_SEED], &crate::ID).0;
    let expected_dispute = Pubkey::find_program_address(
        &[
            crate::constants::DISPUTE_INTENT_SEED,
            dispute.dispute_config.as_ref(),
            &intent.intent_hash,
        ],
        &crate::ID,
    )
    .0;
    let expected_vault =
        Pubkey::find_program_address(&[crate::constants::STAKE_VAULT_CONFIG_SEED], &crate::ID).0;
    let expected_position = Pubkey::find_program_address(
        &[
            crate::constants::STAKE_POSITION_SEED,
            vault.key().as_ref(),
            dispute.stake_owner.as_ref(),
        ],
        &crate::ID,
    )
    .0;
    let expected_lock = Pubkey::find_program_address(
        &[
            crate::constants::STAKE_LOCK_SEED,
            vault.key().as_ref(),
            &intent.intent_hash,
        ],
        &crate::ID,
    )
    .0;
    require_keys_eq!(
        dispute.dispute_config,
        expected_dispute_config,
        Zkp2pError::Unauthorized
    );
    require_keys_eq!(dispute.key(), expected_dispute, Zkp2pError::Unauthorized);
    require_keys_eq!(vault.key(), expected_vault, Zkp2pError::Unauthorized);
    require_keys_eq!(
        vault.controller,
        expected_dispute_config,
        Zkp2pError::Unauthorized
    );
    require_keys_eq!(position.key(), expected_position, Zkp2pError::Unauthorized);
    require_keys_eq!(lock.key(), expected_lock, Zkp2pError::Unauthorized);
    require!(
        dispute.intent_hash == intent.intent_hash,
        Zkp2pError::IntentSnapshotMismatch
    );
    require_keys_eq!(
        dispute.deposit,
        intent.deposit,
        Zkp2pError::IntentSnapshotMismatch
    );
    require_keys_eq!(
        dispute.taker,
        intent.owner,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(
        lock.id == intent.intent_hash,
        Zkp2pError::IntentSnapshotMismatch
    );
    require_keys_eq!(lock.vault, vault.key(), Zkp2pError::Unauthorized);
    require_keys_eq!(
        lock.stake_owner,
        position.owner,
        Zkp2pError::UnauthorizedStakeOwner
    );
    require!(
        release_amount <= lock.amount,
        Zkp2pError::AmountAboveMaximum
    );
    let old_amount = lock.amount;
    let maturity = dispute.settle(release_amount, Clock::get()?.unix_timestamp)?;
    let unlocked = old_amount
        .checked_sub(release_amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    if unlocked > 0 {
        position.unlock(unlocked)?;
    }
    lock.amount = release_amount;
    lock.matures_at = maturity;
    Ok(())
}

fn transfer_exact<'info>(
    deposit: &Account<'info, Deposit>,
    source: &mut InterfaceAccount<'info, TokenAccount>,
    destination: &mut InterfaceAccount<'info, TokenAccount>,
    mint: &InterfaceAccount<'info, Mint>,
    token_program: &Interface<'info, TokenInterface>,
    amount: u64,
) -> Result<()> {
    require!(amount > 0, Zkp2pError::ZeroValue);
    let source_before = source.amount;
    let destination_before = destination.amount;
    let id = deposit.id.to_le_bytes();
    let bump = [deposit.bump];
    let seeds: &[&[u8]] = &[DEPOSIT_SEED, deposit.escrow_config.as_ref(), &id, &bump];
    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            token_program.key(),
            TransferChecked {
                from: source.to_account_info(),
                mint: mint.to_account_info(),
                to: destination.to_account_info(),
                authority: deposit.to_account_info(),
            },
            &[seeds],
        ),
        amount,
        mint.decimals,
    )?;
    source.reload()?;
    destination.reload()?;
    let debited = source_before
        .checked_sub(source.amount)
        .ok_or(Zkp2pError::InvalidTokenBalanceDelta)?;
    let credited = destination
        .amount
        .checked_sub(destination_before)
        .ok_or(Zkp2pError::InvalidTokenBalanceDelta)?;
    require!(
        debited == amount && credited == amount,
        Zkp2pError::InvalidTokenBalanceDelta
    );
    Ok(())
}
