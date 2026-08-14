//! Two-step protocol ownership and latest-component governance.

use anchor_lang::prelude::*;

use crate::{
    constants::{
        ESCROW_CONFIG_SEED, MAX_FEE, MAX_INTENT_LIFETIME_SECONDS, ORCHESTRATOR_CONFIG_SEED,
        PROTOCOL_SEED, STAKE_VAULT_CONFIG_SEED,
    },
    error::Zkp2pError,
    state::{EscrowConfig, LifecyclePolicy, OrchestratorConfig, ProtocolConfig, StakeVaultConfig},
};

/// Accounts for proposing or cancelling protocol ownership transfer.
#[derive(Accounts)]
pub struct ProposeProtocolAuthority<'info> {
    /// Current governance authority.
    #[account(address = protocol.authority)]
    pub authority: Signer<'info>,
    /// Canonical mutable protocol root.
    #[account(mut, seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Account<'info, ProtocolConfig>,
}

/// Sets a nonzero pending authority, or cancels the current proposal with `None`.
pub fn handle_propose_protocol_authority(
    ctx: Context<ProposeProtocolAuthority>,
    pending: Option<Pubkey>,
) -> Result<()> {
    if let Some(candidate) = pending {
        require!(candidate != Pubkey::default(), Zkp2pError::ZeroAddress);
        require_keys_neq!(
            candidate,
            ctx.accounts.protocol.authority,
            Zkp2pError::AlreadyInState
        );
    }
    require!(
        ctx.accounts.protocol.pending_authority != pending,
        Zkp2pError::AlreadyInState
    );
    ctx.accounts.protocol.pending_authority = pending;
    Ok(())
}

/// Accounts for accepting protocol ownership.
#[derive(Accounts)]
pub struct AcceptProtocolAuthority<'info> {
    /// Proposed governance authority.
    pub pending_authority: Signer<'info>,
    /// Canonical mutable protocol root.
    #[account(
        mut,
        seeds = [PROTOCOL_SEED],
        bump = protocol.bump,
        constraint = protocol.pending_authority == Some(pending_authority.key()) @ Zkp2pError::Unauthorized
    )]
    pub protocol: Account<'info, ProtocolConfig>,
}

/// Completes a two-step protocol ownership transfer.
pub fn handle_accept_protocol_authority(ctx: Context<AcceptProtocolAuthority>) -> Result<()> {
    ctx.accounts.protocol.authority = ctx.accounts.pending_authority.key();
    ctx.accounts.protocol.pending_authority = None;
    Ok(())
}

/// Mutable OrchestratorV3-equivalent governance parameters.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConfigureOrchestratorArgs {
    /// New fee, when supplied.
    pub protocol_fee: Option<u128>,
    /// New nonzero fee recipient, when supplied.
    pub protocol_fee_recipient: Option<Pubkey>,
    /// Lifecycle policy snapshotted by future intents.
    pub lifecycle_policy: Option<LifecyclePolicy>,
    /// Whether ordinary takers may hold multiple live intents.
    pub allow_multiple_intents: Option<bool>,
    /// New admission/fulfillment pause state.
    pub paused: Option<bool>,
}

/// Accounts for protocol-governed orchestrator mutation.
#[derive(Accounts)]
pub struct ConfigureOrchestrator<'info> {
    /// Protocol governance authority.
    #[account(address = protocol.authority)]
    pub authority: Signer<'info>,
    /// Protocol root.
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Account<'info, ProtocolConfig>,
    /// Canonical orchestrator configuration.
    #[account(
        mut,
        seeds = [ORCHESTRATOR_CONFIG_SEED],
        bump = orchestrator.bump,
        constraint = orchestrator.protocol == protocol.key() @ Zkp2pError::Unauthorized
    )]
    pub orchestrator: Account<'info, OrchestratorConfig>,
}

/// Applies bounded orchestrator settings without altering already-snapshotted intents.
pub fn handle_configure_orchestrator(
    ctx: Context<ConfigureOrchestrator>,
    args: ConfigureOrchestratorArgs,
) -> Result<()> {
    if let Some(fee) = args.protocol_fee {
        require!(fee <= MAX_FEE, Zkp2pError::FeeExceedsMaximum);
        ctx.accounts.orchestrator.protocol_fee = fee;
    }
    if let Some(recipient) = args.protocol_fee_recipient {
        require!(recipient != Pubkey::default(), Zkp2pError::ZeroAddress);
        ctx.accounts.orchestrator.protocol_fee_recipient = recipient;
    }
    if let Some(policy) = args.lifecycle_policy {
        ctx.accounts.orchestrator.lifecycle_policy = policy;
    }
    if let Some(allow_multiple) = args.allow_multiple_intents {
        ctx.accounts.orchestrator.allow_multiple_intents = allow_multiple;
    }
    if let Some(paused) = args.paused {
        ctx.accounts.orchestrator.paused = paused;
    }
    Ok(())
}

/// Mutable EscrowV2-equivalent governance parameters.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct ConfigureEscrowArgs {
    /// Nonzero dust recipient, when supplied.
    pub dust_recipient: Option<Pubkey>,
    /// Dust sweep ceiling.
    pub dust_threshold: Option<u64>,
    /// Positive maximum active locks per deposit.
    pub max_intents_per_deposit: Option<u16>,
    /// Positive lock expiry not exceeding the absolute lifetime cap.
    pub intent_expiration_period: Option<i64>,
    /// New deposit/lock admission pause state.
    pub paused: Option<bool>,
}

/// Accounts for protocol-governed escrow mutation.
#[derive(Accounts)]
pub struct ConfigureEscrow<'info> {
    /// Protocol governance authority.
    #[account(address = protocol.authority)]
    pub authority: Signer<'info>,
    /// Protocol root.
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Account<'info, ProtocolConfig>,
    /// Canonical escrow configuration.
    #[account(
        mut,
        seeds = [ESCROW_CONFIG_SEED],
        bump = escrow.bump,
        constraint = escrow.protocol == protocol.key() @ Zkp2pError::Unauthorized
    )]
    pub escrow: Account<'info, EscrowConfig>,
}

/// Applies bounded escrow settings without mutating live lock snapshots.
pub fn handle_configure_escrow(
    ctx: Context<ConfigureEscrow>,
    args: ConfigureEscrowArgs,
) -> Result<()> {
    if let Some(recipient) = args.dust_recipient {
        require!(recipient != Pubkey::default(), Zkp2pError::ZeroAddress);
        ctx.accounts.escrow.dust_recipient = recipient;
    }
    if let Some(threshold) = args.dust_threshold {
        ctx.accounts.escrow.dust_threshold = threshold;
    }
    if let Some(maximum) = args.max_intents_per_deposit {
        require!(maximum > 0, Zkp2pError::ZeroValue);
        ctx.accounts.escrow.max_intents_per_deposit = maximum;
    }
    if let Some(expiry) = args.intent_expiration_period {
        require!(expiry > 0, Zkp2pError::ZeroValue);
        require!(
            expiry <= MAX_INTENT_LIFETIME_SECONDS,
            Zkp2pError::AmountAboveMaximum
        );
        ctx.accounts.escrow.intent_expiration_period = expiry;
    }
    if let Some(paused) = args.paused {
        ctx.accounts.escrow.paused = paused;
    }
    Ok(())
}

/// Accounts for proposing or cancelling a delayed stake-controller handover.
#[derive(Accounts)]
pub struct ProposeStakeController<'info> {
    /// Protocol governance authority.
    #[account(address = protocol.authority)]
    pub authority: Signer<'info>,
    /// Protocol root.
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Account<'info, ProtocolConfig>,
    /// Canonical stake vault.
    #[account(
        mut,
        seeds = [STAKE_VAULT_CONFIG_SEED],
        bump = vault.bump,
        constraint = vault.protocol == protocol.key() @ Zkp2pError::Unauthorized
    )]
    pub vault: Account<'info, StakeVaultConfig>,
}

/// Starts a delayed nonzero controller handover, or cancels it with `None`.
pub fn handle_propose_stake_controller(
    ctx: Context<ProposeStakeController>,
    pending: Option<Pubkey>,
) -> Result<()> {
    match pending {
        Some(candidate) => {
            require!(candidate != Pubkey::default(), Zkp2pError::ZeroAddress);
            require_keys_neq!(
                candidate,
                ctx.accounts.vault.controller,
                Zkp2pError::AlreadyInState
            );
            ctx.accounts.vault.pending_controller = Some(candidate);
            ctx.accounts.vault.pending_controller_valid_at = Clock::get()?
                .unix_timestamp
                .checked_add(ctx.accounts.vault.controller_change_delay)
                .ok_or(Zkp2pError::ArithmeticOverflow)?;
        }
        None => {
            require!(
                ctx.accounts.vault.pending_controller.is_some(),
                Zkp2pError::AlreadyInState
            );
            ctx.accounts.vault.pending_controller = None;
            ctx.accounts.vault.pending_controller_valid_at = 0;
        }
    }
    Ok(())
}

/// Accounts for accepting a mature stake-controller proposal.
#[derive(Accounts)]
pub struct AcceptStakeController<'info> {
    /// Proposed controller.
    pub pending_controller: Signer<'info>,
    /// Canonical stake vault.
    #[account(
        mut,
        seeds = [STAKE_VAULT_CONFIG_SEED],
        bump = vault.bump,
        constraint = vault.pending_controller == Some(pending_controller.key()) @ Zkp2pError::Unauthorized
    )]
    pub vault: Account<'info, StakeVaultConfig>,
}

/// Completes a mature two-step stake-controller handover.
pub fn handle_accept_stake_controller(ctx: Context<AcceptStakeController>) -> Result<()> {
    require!(
        Clock::get()?.unix_timestamp >= ctx.accounts.vault.pending_controller_valid_at,
        Zkp2pError::ControllerNotReady
    );
    ctx.accounts.vault.controller = ctx.accounts.pending_controller.key();
    ctx.accounts.vault.pending_controller = None;
    ctx.accounts.vault.pending_controller_valid_at = 0;
    Ok(())
}
