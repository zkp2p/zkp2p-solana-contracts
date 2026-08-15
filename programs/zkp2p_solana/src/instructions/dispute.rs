//! Stake-backed dispute-protection policy instructions.

use anchor_lang::prelude::*;
use solana_program::sysvar::instructions::{
    load_current_index_checked, load_instruction_at_checked,
};

use crate::{
    constants::{
        CLAIM_BALANCE_SEED, DEPOSIT_DISPUTE_SETTING_SEED, DISPUTE_ATTESTATION_TYPEHASH,
        DISPUTE_CONFIG_SEED, DISPUTE_INTENT_SEED, DISPUTE_NULLIFIER_SEED,
        DISPUTE_VERIFIER_NAME_HASH, EIP712_DOMAIN_TYPEHASH, EIP712_VERSION_ONE_HASH,
        INTENT_PAYMENT_BINDING_SEED, MAX_RISK_WINDOW_SECONDS, NEVER_MATURES,
        ORCHESTRATOR_CONFIG_SEED, PAYMENT_BINDING_SEED, PROTOCOL_SEED, RISK_WINDOW_SEED,
        STAKE_LOCK_SEED, STAKE_POSITION_SEED, STAKE_SELECTION_SEED, STAKE_VAULT_CONFIG_SEED,
        VERIFIER_CONFIG_SEED,
    },
    error::Zkp2pError,
    instructions::{derive_intent_hash, verify_witness_threshold},
    state::{
        ClaimBalance, Deposit, DepositDisputeSetting, DisputeConfig, DisputeIntent,
        DisputeNullifier, DisputeStatus, Intent, IntentPaymentBinding, OrchestratorConfig,
        PaymentBinding, ProtocolConfig, RiskWindow, StakeLock, StakePosition, StakeSelection,
        StakeVaultConfig, VerifierConfig,
    },
};
use solana_keccak_hasher as keccak;

/// Accounts for governance configuration of one payment-method risk window.
#[derive(Accounts)]
#[instruction(payment_method: [u8; 32])]
pub struct SetRiskWindow<'info> {
    /// Protocol governance authority and rent payer.
    #[account(mut, address = protocol.authority)]
    pub authority: Signer<'info>,
    /// Protocol root.
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Account<'info, ProtocolConfig>,
    /// Dispute component.
    #[account(seeds = [DISPUTE_CONFIG_SEED], bump = dispute_config.bump)]
    pub dispute_config: Account<'info, DisputeConfig>,
    /// Method-specific window PDA.
    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + RiskWindow::INIT_SPACE,
        seeds = [RISK_WINDOW_SEED, dispute_config.key().as_ref(), &payment_method],
        bump
    )]
    pub risk_window: Account<'info, RiskWindow>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Sets a bounded risk window; zero preserves pass-through semantics.
pub fn handle_set_risk_window(
    ctx: Context<SetRiskWindow>,
    payment_method: [u8; 32],
    seconds: i64,
) -> Result<()> {
    require!(payment_method != [0; 32], Zkp2pError::ZeroValue);
    require!(
        (0..=MAX_RISK_WINDOW_SECONDS).contains(&seconds),
        Zkp2pError::AmountAboveMaximum
    );
    let risk = &mut ctx.accounts.risk_window;
    risk.dispute_config = ctx.accounts.dispute_config.key();
    risk.payment_method = payment_method;
    risk.seconds = seconds;
    risk.bump = ctx.bumps.risk_window;
    Ok(())
}

/// Accounts for changing global dispute admissions.
#[derive(Accounts)]
pub struct SetDisputeAdmissionsPaused<'info> {
    /// Protocol governance authority.
    #[account(address = protocol.authority)]
    pub authority: Signer<'info>,
    /// Protocol root.
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Account<'info, ProtocolConfig>,
    /// Dispute component.
    #[account(mut, seeds = [DISPUTE_CONFIG_SEED], bump = dispute_config.bump)]
    pub dispute_config: Account<'info, DisputeConfig>,
}

/// Pauses or resumes new covered admissions without affecting exits.
pub fn handle_set_dispute_admissions_paused(
    ctx: Context<SetDisputeAdmissionsPaused>,
    paused: bool,
) -> Result<()> {
    ctx.accounts.dispute_config.admissions_paused = paused;
    Ok(())
}

/// Accounts for setting one deposit's default-on dispute protection opt-out.
#[derive(Accounts)]
pub struct SetDepositDisputeProtection<'info> {
    /// Deposit owner or delegate and rent payer.
    #[account(mut)]
    pub authority: Signer<'info>,
    /// Existing deposit.
    #[account(
        constraint = deposit.depositor == authority.key() || deposit.delegate == Some(authority.key()) @ Zkp2pError::Unauthorized
    )]
    pub deposit: Account<'info, Deposit>,
    /// Setting PDA; absence before this instruction means enabled.
    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + DepositDisputeSetting::INIT_SPACE,
        seeds = [DEPOSIT_DISPUTE_SETTING_SEED, deposit.key().as_ref()],
        bump
    )]
    pub setting: Account<'info, DepositDisputeSetting>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Explicitly enables or disables coverage for one deposit.
pub fn handle_set_deposit_dispute_protection(
    ctx: Context<SetDepositDisputeProtection>,
    enabled: bool,
) -> Result<()> {
    ctx.accounts.setting.deposit = ctx.accounts.deposit.key();
    ctx.accounts.setting.enabled = enabled;
    ctx.accounts.setting.bump = ctx.bumps.setting;
    Ok(())
}

/// Arguments that bind pre-signal collateral to the upcoming orchestrator intent.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrepareDisputeArgs {
    /// Client-derived intent hash cross-checked against the serialized counter.
    pub expected_intent_hash: [u8; 32],
    /// Payment method used by the upcoming intent.
    pub payment_method: [u8; 32],
    /// Complete upcoming intent amount.
    pub amount: u64,
}

/// Accounts for atomically preparing stake-backed coverage before signal admission.
#[derive(Accounts)]
#[instruction(args: PrepareDisputeArgs)]
pub struct PrepareDispute<'info> {
    /// Upcoming intent owner and rent payer.
    #[account(mut)]
    pub taker: Signer<'info>,
    /// Serialized intent nonce source.
    #[account(seeds = [ORCHESTRATOR_CONFIG_SEED], bump = orchestrator.bump)]
    pub orchestrator: Box<Account<'info, OrchestratorConfig>>,
    /// Dispute component.
    #[account(seeds = [DISPUTE_CONFIG_SEED], bump = dispute_config.bump)]
    pub dispute_config: Box<Account<'info, DisputeConfig>>,
    /// Canonical stake vault.
    #[account(
        mut,
        seeds = [STAKE_VAULT_CONFIG_SEED],
        bump = stake_vault.bump,
        constraint = dispute_config.stake_vault == stake_vault.key() @ Zkp2pError::Unauthorized,
        constraint = stake_vault.controller == dispute_config.key() @ Zkp2pError::Unauthorized
    )]
    pub stake_vault: Box<Account<'info, StakeVaultConfig>>,
    /// Selected deposit.
    #[account(constraint = deposit.token_mint == stake_vault.stake_mint @ Zkp2pError::IntentTokenMismatch)]
    pub deposit: Box<Account<'info, Deposit>>,
    /// Optional explicit opt-out/opt-in; absence means enabled.
    pub deposit_setting: Option<Box<Account<'info, DepositDisputeSetting>>>,
    /// Nonzero method risk window.
    #[account(
        seeds = [RISK_WINDOW_SEED, dispute_config.key().as_ref(), &args.payment_method],
        bump = risk_window.bump,
        constraint = risk_window.dispute_config == dispute_config.key() @ Zkp2pError::Unauthorized
    )]
    pub risk_window: Box<Account<'info, RiskWindow>>,
    /// Optional taker selection; absence means self-owned collateral.
    #[account(seeds = [STAKE_SELECTION_SEED, taker.key().as_ref()], bump = selection.bump)]
    pub selection: Option<Box<Account<'info, StakeSelection>>>,
    /// Selected collateral owner identity.
    /// CHECK: Bound to selection/self and stake position below.
    pub stake_owner: UncheckedAccount<'info>,
    /// Selected owner's aggregate stake.
    #[account(
        mut,
        seeds = [STAKE_POSITION_SEED, stake_vault.key().as_ref(), stake_owner.key().as_ref()],
        bump = position.bump,
        constraint = position.owner == stake_owner.key() @ Zkp2pError::UnauthorizedStakeOwner
    )]
    pub position: Box<Account<'info, StakePosition>>,
    /// New stake lock.
    #[account(
        init,
        payer = taker,
        space = 8 + StakeLock::INIT_SPACE,
        seeds = [STAKE_LOCK_SEED, stake_vault.key().as_ref(), &args.expected_intent_hash],
        bump
    )]
    pub stake_lock: Box<Account<'info, StakeLock>>,
    /// New dispute lifecycle account.
    #[account(
        init,
        payer = taker,
        space = 8 + DisputeIntent::INIT_SPACE,
        seeds = [DISPUTE_INTENT_SEED, dispute_config.key().as_ref(), &args.expected_intent_hash],
        bump
    )]
    pub dispute_intent: Box<Account<'info, DisputeIntent>>,
    /// Instruction sysvar proving that matching signal admission follows atomically.
    /// CHECK: Anchor enforces the canonical sysvar address and only instruction data is read.
    #[account(address = solana_program::sysvar::instructions::ID)]
    pub instructions_sysvar: UncheckedAccount<'info>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Locks the exact upcoming intent amount under the snapshotted stake owner and risk window.
pub fn handle_prepare_dispute(
    ctx: Context<PrepareDispute>,
    args: PrepareDisputeArgs,
) -> Result<()> {
    require!(
        !ctx.accounts.dispute_config.admissions_paused,
        Zkp2pError::AdmissionsPaused
    );
    require!(args.amount > 0, Zkp2pError::ZeroValue);
    require!(ctx.accounts.risk_window.seconds > 0, Zkp2pError::ZeroValue);
    require!(
        ctx.accounts.orchestrator.lifecycle_policy == crate::LifecyclePolicy::WhitelistAndDispute,
        Zkp2pError::DisputeProtectionDisabled
    );
    require_atomic_signal_follows(
        &ctx.accounts.instructions_sysvar,
        args,
        ctx.accounts.taker.key(),
        ctx.accounts.orchestrator.key(),
        ctx.accounts.deposit.key(),
        ctx.accounts.dispute_intent.key(),
    )?;
    if let Some(setting) = &ctx.accounts.deposit_setting {
        require_keys_eq!(
            setting.deposit,
            ctx.accounts.deposit.key(),
            Zkp2pError::DepositNotFound
        );
        require!(setting.enabled, Zkp2pError::DisputeProtectionDisabled);
    }
    let expected = derive_intent_hash(
        ctx.accounts.orchestrator.key(),
        ctx.accounts.orchestrator.next_intent_id,
    );
    require!(
        expected == args.expected_intent_hash,
        Zkp2pError::DataHashMismatch
    );
    let selected_owner = ctx
        .accounts
        .selection
        .as_ref()
        .and_then(|selection| selection.selected_owner)
        .unwrap_or(ctx.accounts.taker.key());
    require_keys_eq!(
        selected_owner,
        ctx.accounts.stake_owner.key(),
        Zkp2pError::UnauthorizedStakeOwner
    );
    ctx.accounts.position.lock(args.amount)?;

    let stake_lock = &mut ctx.accounts.stake_lock;
    stake_lock.vault = ctx.accounts.stake_vault.key();
    stake_lock.id = args.expected_intent_hash;
    stake_lock.stake_owner = selected_owner;
    stake_lock.amount = args.amount;
    stake_lock.matures_at = NEVER_MATURES;
    stake_lock.bump = ctx.bumps.stake_lock;

    let dispute = &mut ctx.accounts.dispute_intent;
    dispute.dispute_config = ctx.accounts.dispute_config.key();
    dispute.intent_hash = args.expected_intent_hash;
    dispute.deposit = ctx.accounts.deposit.key();
    dispute.taker = ctx.accounts.taker.key();
    dispute.stake_owner = selected_owner;
    dispute.depositor = ctx.accounts.deposit.depositor;
    dispute.payment_method = args.payment_method;
    dispute.locked_amount = args.amount;
    dispute.status = DisputeStatus::Pending;
    dispute.risk_window = ctx.accounts.risk_window.seconds;
    dispute.release_eligible_at = 0;
    dispute.release_amount = 0;
    dispute.bump = ctx.bumps.dispute_intent;
    Ok(())
}

fn require_atomic_signal_follows(
    instructions_sysvar: &AccountInfo<'_>,
    prepared: PrepareDisputeArgs,
    taker: Pubkey,
    orchestrator: Pubkey,
    deposit: Pubkey,
    dispute_intent: Pubkey,
) -> Result<()> {
    let current = usize::from(
        load_current_index_checked(instructions_sysvar)
            .map_err(|_| error!(Zkp2pError::Unauthorized))?,
    );
    let next = current
        .checked_add(1)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    let instruction = load_instruction_at_checked(next, instructions_sysvar)
        .map_err(|_| error!(Zkp2pError::Unauthorized))?;
    require_keys_eq!(instruction.program_id, crate::ID, Zkp2pError::Unauthorized);
    // `SignalIntent` account ordering is part of the public IDL. Bind every account
    // that connects the prepared collateral to the following admission.
    for (index, expected) in [
        (0, taker),
        (1, orchestrator),
        (3, deposit),
        (14, dispute_intent),
    ] {
        let supplied = instruction
            .accounts
            .get(index)
            .ok_or(Zkp2pError::Unauthorized)?;
        require_keys_eq!(supplied.pubkey, expected, Zkp2pError::Unauthorized);
    }
    require!(
        instruction
            .accounts
            .first()
            .is_some_and(|account| account.is_signer && account.is_writable),
        Zkp2pError::Unauthorized
    );
    let discriminator = crate::instruction::SignalIntent::DISCRIMINATOR;
    let encoded_args = instruction
        .data
        .strip_prefix(discriminator)
        .ok_or(Zkp2pError::Unauthorized)?;
    let signal = crate::SignalIntentArgs::try_from_slice(encoded_args)
        .map_err(|_| error!(Zkp2pError::Unauthorized))?;
    require!(
        signal.expected_intent_hash == prepared.expected_intent_hash
            && signal.payment_method == prepared.payment_method
            && signal.amount == prepared.amount,
        Zkp2pError::IntentSnapshotMismatch
    );
    Ok(())
}

/// Accounts for cancelling pending collateral alongside an owner intent cancellation.
#[derive(Accounts)]
pub struct CancelDispute<'info> {
    /// Original intent owner and stake-lock rent recipient.
    #[account(mut, address = intent.owner)]
    pub owner: Signer<'info>,
    /// Active intent.
    pub intent: Account<'info, Intent>,
    /// Stake vault.
    #[account(seeds = [STAKE_VAULT_CONFIG_SEED], bump = stake_vault.bump)]
    pub stake_vault: Account<'info, StakeVaultConfig>,
    /// Stake owner position.
    #[account(
        mut,
        seeds = [STAKE_POSITION_SEED, stake_vault.key().as_ref(), dispute_intent.stake_owner.as_ref()],
        bump = position.bump
    )]
    pub position: Account<'info, StakePosition>,
    /// Paired collateral lock.
    #[account(
        mut,
        close = owner,
        seeds = [STAKE_LOCK_SEED, stake_vault.key().as_ref(), &intent.intent_hash],
        bump = stake_lock.bump,
        constraint = stake_lock.stake_owner == position.owner @ Zkp2pError::UnauthorizedStakeOwner
    )]
    pub stake_lock: Account<'info, StakeLock>,
    /// Pending dispute lifecycle state.
    #[account(
        mut,
        seeds = [DISPUTE_INTENT_SEED, dispute_intent.dispute_config.as_ref(), &intent.intent_hash],
        bump = dispute_intent.bump,
        constraint = dispute_intent.intent_hash == intent.intent_hash @ Zkp2pError::IntentSnapshotMismatch
    )]
    pub dispute_intent: Account<'info, DisputeIntent>,
    /// Instruction sysvar proving that the matching escrow cancellation follows atomically.
    /// CHECK: Anchor enforces the canonical sysvar address and only instruction data is read.
    #[account(address = solana_program::sysvar::instructions::ID)]
    pub instructions_sysvar: UncheckedAccount<'info>,
}

/// Cancels pending coverage and restores complete free stake.
pub fn handle_cancel_dispute(ctx: Context<CancelDispute>) -> Result<()> {
    require_atomic_cancel_follows(
        &ctx.accounts.instructions_sysvar,
        &ctx.accounts.intent,
        ctx.accounts.intent.key(),
        ctx.accounts.owner.key(),
        ctx.accounts.dispute_intent.key(),
    )?;
    ctx.accounts.dispute_intent.cancel()?;
    ctx.accounts.position.unlock(ctx.accounts.stake_lock.amount)
}

fn require_atomic_cancel_follows(
    instructions_sysvar: &AccountInfo<'_>,
    intent: &Intent,
    intent_key: Pubkey,
    owner: Pubkey,
    dispute_intent: Pubkey,
) -> Result<()> {
    let current = usize::from(
        load_current_index_checked(instructions_sysvar)
            .map_err(|_| error!(Zkp2pError::Unauthorized))?,
    );
    let next = current
        .checked_add(1)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    let instruction = load_instruction_at_checked(next, instructions_sysvar)
        .map_err(|_| error!(Zkp2pError::Unauthorized))?;
    require_keys_eq!(instruction.program_id, crate::ID, Zkp2pError::Unauthorized);
    let expected_lock = Pubkey::find_program_address(
        &[
            crate::constants::ESCROW_INTENT_LOCK_SEED,
            intent.deposit.as_ref(),
            &intent.intent_hash,
        ],
        &crate::ID,
    )
    .0;
    let expected_taker_state = Pubkey::find_program_address(
        &[
            crate::constants::TAKER_INTENT_STATE_SEED,
            intent.orchestrator.as_ref(),
            owner.as_ref(),
        ],
        &crate::ID,
    )
    .0;
    for (index, expected) in [
        (0, owner),
        (1, intent.orchestrator),
        (2, intent_key),
        (3, intent.deposit),
        (4, expected_lock),
        (5, dispute_intent),
        (6, expected_taker_state),
    ] {
        let supplied = instruction
            .accounts
            .get(index)
            .ok_or(Zkp2pError::Unauthorized)?;
        require_keys_eq!(supplied.pubkey, expected, Zkp2pError::Unauthorized);
    }
    require!(
        instruction
            .accounts
            .first()
            .is_some_and(|account| account.is_signer && account.is_writable),
        Zkp2pError::Unauthorized
    );
    require!(
        instruction.data.as_slice() == crate::instruction::CancelIntent::DISCRIMINATOR,
        Zkp2pError::Unauthorized
    );
    Ok(())
}

/// Accounts for permissionlessly releasing mature, settled collateral.
#[derive(Accounts)]
pub struct ReleaseMaturedDispute<'info> {
    /// Permissionless caller and stake-lock rent recipient.
    #[account(mut)]
    pub caller: Signer<'info>,
    /// Stake vault.
    #[account(seeds = [STAKE_VAULT_CONFIG_SEED], bump = stake_vault.bump)]
    pub stake_vault: Account<'info, StakeVaultConfig>,
    /// Stake owner position.
    #[account(
        mut,
        seeds = [STAKE_POSITION_SEED, stake_vault.key().as_ref(), dispute_intent.stake_owner.as_ref()],
        bump = position.bump
    )]
    pub position: Account<'info, StakePosition>,
    /// Mature collateral lock.
    #[account(
        mut,
        close = caller,
        seeds = [STAKE_LOCK_SEED, stake_vault.key().as_ref(), &dispute_intent.intent_hash],
        bump = stake_lock.bump
    )]
    pub stake_lock: Account<'info, StakeLock>,
    /// Settled dispute lifecycle state.
    #[account(
        mut,
        seeds = [DISPUTE_INTENT_SEED, dispute_intent.dispute_config.as_ref(), &dispute_intent.intent_hash],
        bump = dispute_intent.bump
    )]
    pub dispute_intent: Account<'info, DisputeIntent>,
}

/// Releases a settled lock at or after its snapshotted risk-window boundary.
pub fn handle_release_matured_dispute(ctx: Context<ReleaseMaturedDispute>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let amount = ctx.accounts.dispute_intent.release(now)?;
    require!(
        amount == ctx.accounts.stake_lock.amount,
        Zkp2pError::IntentSnapshotMismatch
    );
    ctx.accounts.position.unlock(amount)
}

/// Provider identifiers and amount metadata signed into dispute evidence.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisputeDetails {
    /// Original payment method.
    pub payment_method: [u8; 32],
    /// Original provider payment identifier.
    pub original_payment_id: [u8; 32],
    /// Provider dispute or reversal identifier.
    pub dispute_id: [u8; 32],
    /// Original off-chain payment amount.
    pub payment_amount: u128,
    /// Original off-chain payment currency.
    pub payment_currency: [u8; 32],
}

/// Threshold-signed evidence for one settled covered intent.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct DisputeAttestation {
    /// Covered intent identifier.
    pub intent_hash: [u8; 32],
    /// Keccak commitment to canonical Borsh dispute details.
    pub data_hash: [u8; 32],
    /// Ethereum-format recoverable witness signatures.
    pub signatures: Vec<[u8; 65]>,
    /// Canonical dispute details.
    pub details: DisputeDetails,
}

/// Dispute submission with client-derived PDA seed cross-checks.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct SubmitDisputeArgs {
    /// Signed dispute evidence.
    pub attestation: DisputeAttestation,
    /// Expected keccak(method || original payment ID).
    pub expected_payment_nullifier: [u8; 32],
    /// Expected keccak(method || dispute ID).
    pub expected_dispute_nullifier: [u8; 32],
}

/// Accounts for atomically resolving valid dispute evidence into a maker claim.
#[derive(Accounts)]
#[instruction(args: SubmitDisputeArgs)]
pub struct SubmitDispute<'info> {
    /// Permissionless evidence submitter and rent payer/recipient.
    #[account(mut)]
    pub caller: Signer<'info>,
    /// Dispute component.
    #[account(seeds = [DISPUTE_CONFIG_SEED], bump = dispute_config.bump)]
    pub dispute_config: Box<Account<'info, DisputeConfig>>,
    /// Shared threshold witness configuration.
    #[account(
        seeds = [VERIFIER_CONFIG_SEED],
        bump = verifier.bump,
        constraint = dispute_config.verifier_config == verifier.key() @ Zkp2pError::Unauthorized
    )]
    pub verifier: Box<Account<'info, VerifierConfig>>,
    /// Settled coverage state.
    #[account(
        mut,
        seeds = [DISPUTE_INTENT_SEED, dispute_config.key().as_ref(), &args.attestation.intent_hash],
        bump = dispute_intent.bump,
        constraint = dispute_intent.dispute_config == dispute_config.key() @ Zkp2pError::Unauthorized
    )]
    pub dispute_intent: Box<Account<'info, DisputeIntent>>,
    /// Original payment-nullifier binding.
    #[account(
        seeds = [PAYMENT_BINDING_SEED, verifier.key().as_ref(), &args.expected_payment_nullifier],
        bump = payment_binding.bump
    )]
    pub payment_binding: Box<Account<'info, PaymentBinding>>,
    /// Reverse intent-to-payment binding.
    #[account(
        seeds = [INTENT_PAYMENT_BINDING_SEED, verifier.key().as_ref(), &args.attestation.intent_hash],
        bump = intent_payment_binding.bump
    )]
    pub intent_payment_binding: Box<Account<'info, IntentPaymentBinding>>,
    /// Dedicated immutable dispute replay marker.
    #[account(
        init,
        payer = caller,
        space = 8 + DisputeNullifier::INIT_SPACE,
        seeds = [DISPUTE_NULLIFIER_SEED, dispute_config.key().as_ref(), &args.expected_dispute_nullifier],
        bump
    )]
    pub dispute_nullifier: Box<Account<'info, DisputeNullifier>>,
    /// Canonical stake vault.
    #[account(
        mut,
        seeds = [STAKE_VAULT_CONFIG_SEED],
        bump = stake_vault.bump,
        constraint = dispute_config.stake_vault == stake_vault.key() @ Zkp2pError::Unauthorized,
        constraint = stake_vault.controller == dispute_config.key() @ Zkp2pError::Unauthorized
    )]
    pub stake_vault: Box<Account<'info, StakeVaultConfig>>,
    /// Collateral owner's aggregate stake.
    #[account(
        mut,
        seeds = [STAKE_POSITION_SEED, stake_vault.key().as_ref(), dispute_intent.stake_owner.as_ref()],
        bump = position.bump
    )]
    pub position: Box<Account<'info, StakePosition>>,
    /// Exact settled collateral lock, removed after resolution.
    #[account(
        mut,
        close = caller,
        seeds = [STAKE_LOCK_SEED, stake_vault.key().as_ref(), &args.attestation.intent_hash],
        bump = stake_lock.bump,
        constraint = stake_lock.stake_owner == dispute_intent.stake_owner @ Zkp2pError::UnauthorizedStakeOwner
    )]
    pub stake_lock: Box<Account<'info, StakeLock>>,
    /// Maker's immediately claimable compensation.
    #[account(
        init_if_needed,
        payer = caller,
        space = 8 + ClaimBalance::INIT_SPACE,
        seeds = [CLAIM_BALANCE_SEED, stake_vault.key().as_ref(), dispute_intent.depositor.as_ref()],
        bump
    )]
    pub claim: Box<Account<'info, ClaimBalance>>,
    /// System program.
    pub system_program: Program<'info, System>,
}

/// Verifies evidence and atomically converts exact settled collateral into a maker liability.
pub fn handle_submit_dispute(ctx: Context<SubmitDispute>, args: SubmitDisputeArgs) -> Result<()> {
    let attestation = &args.attestation;
    let details = &attestation.details;
    require!(
        details.payment_method == ctx.accounts.dispute_intent.payment_method,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(
        details.original_payment_id != [0; 32],
        Zkp2pError::ZeroValue
    );
    require!(details.dispute_id != [0; 32], Zkp2pError::ZeroValue);
    require!(details.payment_amount > 0, Zkp2pError::ZeroValue);
    require!(details.payment_currency != [0; 32], Zkp2pError::ZeroValue);

    require!(
        dispute_data_hash(details) == attestation.data_hash,
        Zkp2pError::DataHashMismatch
    );
    let payment_nullifier =
        keccak::hashv(&[&details.payment_method, &details.original_payment_id]).to_bytes();
    let dispute_nullifier =
        keccak::hashv(&[&details.payment_method, &details.dispute_id]).to_bytes();
    require!(
        payment_nullifier == args.expected_payment_nullifier,
        Zkp2pError::DataHashMismatch
    );
    require!(
        dispute_nullifier == args.expected_dispute_nullifier,
        Zkp2pError::DataHashMismatch
    );
    require!(
        ctx.accounts.payment_binding.nullifier == payment_nullifier,
        Zkp2pError::InvalidPaymentBinding
    );
    require!(
        ctx.accounts.payment_binding.intent_hash == attestation.intent_hash,
        Zkp2pError::InvalidPaymentBinding
    );
    require!(
        ctx.accounts.intent_payment_binding.intent_hash == attestation.intent_hash,
        Zkp2pError::InvalidPaymentBinding
    );
    require!(
        ctx.accounts.intent_payment_binding.nullifier == payment_nullifier,
        Zkp2pError::InvalidPaymentBinding
    );

    let digest = dispute_attestation_digest(
        ctx.accounts.dispute_config.key(),
        ctx.accounts.dispute_config.domain_chain_id,
        attestation.intent_hash,
        attestation.data_hash,
    );
    verify_witness_threshold(&ctx.accounts.verifier, digest, &attestation.signatures)?;

    let amount = ctx.accounts.dispute_intent.dispute()?;
    require!(
        amount == ctx.accounts.stake_lock.amount,
        Zkp2pError::IntentSnapshotMismatch
    );
    ctx.accounts
        .position
        .resolve(ctx.accounts.stake_lock.amount, amount)?;
    ctx.accounts.stake_vault.total_staked = ctx
        .accounts
        .stake_vault
        .total_staked
        .checked_sub(amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    ctx.accounts.stake_vault.total_claimable = ctx
        .accounts
        .stake_vault
        .total_claimable
        .checked_add(amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;

    let claim = &mut ctx.accounts.claim;
    if claim.beneficiary == Pubkey::default() {
        claim.vault = ctx.accounts.stake_vault.key();
        claim.beneficiary = ctx.accounts.dispute_intent.depositor;
        claim.bump = ctx.bumps.claim;
    }
    claim.amount = claim
        .amount
        .checked_add(amount)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;

    ctx.accounts.dispute_nullifier.dispute_config = ctx.accounts.dispute_config.key();
    ctx.accounts.dispute_nullifier.nullifier = dispute_nullifier;
    ctx.accounts.dispute_nullifier.intent_hash = attestation.intent_hash;
    ctx.accounts.dispute_nullifier.bump = ctx.bumps.dispute_nullifier;
    Ok(())
}

fn dispute_data_hash(details: &DisputeDetails) -> [u8; 32] {
    let payment_amount = details.payment_amount.to_le_bytes();
    keccak::hashv(&[
        &details.payment_method,
        &details.original_payment_id,
        &details.dispute_id,
        &payment_amount,
        &details.payment_currency,
    ])
    .to_bytes()
}

/// Returns the EIP-712 digest for Solana dispute verifier evidence.
pub fn dispute_attestation_digest(
    dispute_config: Pubkey,
    domain_chain_id: u64,
    intent_hash: [u8; 32],
    data_hash: [u8; 32],
) -> [u8; 32] {
    let account_hash = keccak::hash(dispute_config.as_ref()).to_bytes();
    let mut address_word = [0_u8; 32];
    address_word[12..].copy_from_slice(&account_hash[12..]);
    let mut chain_id = [0_u8; 32];
    chain_id[24..].copy_from_slice(&domain_chain_id.to_be_bytes());
    let domain_separator = keccak::hashv(&[
        &EIP712_DOMAIN_TYPEHASH,
        &DISPUTE_VERIFIER_NAME_HASH,
        &EIP712_VERSION_ONE_HASH,
        &chain_id,
        &address_word,
    ])
    .to_bytes();
    let struct_hash =
        keccak::hashv(&[&DISPUTE_ATTESTATION_TYPEHASH, &intent_hash, &data_hash]).to_bytes();
    keccak::hashv(&[&[0x19, 0x01], &domain_separator, &struct_hash]).to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prehashed_eip712_constants_match_canonical_strings() {
        assert_eq!(
            DISPUTE_VERIFIER_NAME_HASH,
            keccak::hash(b"ZKP2P DisputeVerifier").to_bytes()
        );
        assert_eq!(
            DISPUTE_ATTESTATION_TYPEHASH,
            keccak::hash(b"DisputeAttestation(bytes32 intentHash,bytes32 dataHash)").to_bytes()
        );
    }

    #[test]
    fn dispute_digest_is_bound_to_domain_intent_and_payload() {
        let config = Pubkey::new_unique();
        let baseline = dispute_attestation_digest(config, 1, [1; 32], [2; 32]);
        assert_ne!(
            baseline,
            dispute_attestation_digest(Pubkey::new_unique(), 1, [1; 32], [2; 32])
        );
        assert_ne!(
            baseline,
            dispute_attestation_digest(config, 1, [3; 32], [2; 32])
        );
        assert_ne!(
            baseline,
            dispute_attestation_digest(config, 1, [1; 32], [4; 32])
        );
        assert_ne!(
            baseline,
            dispute_attestation_digest(config, 2, [1; 32], [2; 32])
        );
    }

    #[test]
    fn fixed_width_hash_matches_canonical_borsh_payload() {
        let details = DisputeDetails {
            payment_method: [1; 32],
            original_payment_id: [2; 32],
            dispute_id: [3; 32],
            payment_amount: u128::MAX,
            payment_currency: [4; 32],
        };
        let mut canonical = Vec::new();
        details
            .serialize(&mut canonical)
            .expect("serialize details");
        assert_eq!(
            dispute_data_hash(&details),
            keccak::hash(&canonical).to_bytes()
        );
    }
}
