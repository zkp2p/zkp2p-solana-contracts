//! Anchor instruction contexts and handlers.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::Mint;

use crate::{
    constants::{
        DISPUTE_CONFIG_SEED, ESCROW_CONFIG_SEED, MAX_FEE, MIN_CONTROLLER_CHANGE_DELAY_SECONDS,
        ORCHESTRATOR_CONFIG_SEED, PROTOCOL_SEED, RATE_MANAGER_CONFIG_SEED, STAKE_VAULT_CONFIG_SEED,
        VERIFIER_CONFIG_SEED, WHITELIST_CONFIG_SEED,
    },
    error::Zkp2pError,
    state::{
        DisputeConfig, EscrowConfig, InitializeProtocolArgs, LifecyclePolicy, OrchestratorConfig,
        ProtocolConfig, RateManagerConfig, StakeVaultConfig, VerifierConfig, WhitelistConfig,
    },
};

pub mod stake;
pub use stake::*;
pub mod rate_manager;
pub use rate_manager::*;
pub mod escrow;
pub use escrow::*;
pub mod orchestrator;
pub use orchestrator::*;
pub mod whitelist;
pub use whitelist::*;
pub mod dispute;
pub use dispute::*;
pub mod verifier;
pub use verifier::*;
pub mod settlement;
pub use settlement::*;
pub mod governance;
pub use governance::*;

/// Accounts required to initialize the protocol root.
#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    /// Governance authority and rent payer.
    #[account(mut)]
    pub authority: Signer<'info>,
    /// Canonical root PDA.
    #[account(
        init,
        payer = authority,
        space = 8 + ProtocolConfig::INIT_SPACE,
        seeds = [PROTOCOL_SEED],
        bump
    )]
    pub protocol: Account<'info, ProtocolConfig>,
    /// Canonical mint used by escrow and stake custody.
    pub stake_mint: InterfaceAccount<'info, Mint>,
    /// Escrow configuration PDA.
    #[account(
        init,
        payer = authority,
        space = 8 + EscrowConfig::INIT_SPACE,
        seeds = [ESCROW_CONFIG_SEED],
        bump
    )]
    pub escrow_config: Account<'info, EscrowConfig>,
    /// Unified verifier configuration PDA.
    #[account(
        init,
        payer = authority,
        space = 8 + VerifierConfig::INIT_SPACE,
        seeds = [VERIFIER_CONFIG_SEED],
        bump
    )]
    pub verifier_config: Account<'info, VerifierConfig>,
    /// Orchestrator configuration PDA.
    #[account(
        init,
        payer = authority,
        space = 8 + OrchestratorConfig::INIT_SPACE,
        seeds = [ORCHESTRATOR_CONFIG_SEED],
        bump
    )]
    pub orchestrator_config: Account<'info, OrchestratorConfig>,
    /// Stake-vault configuration PDA.
    #[account(
        init,
        payer = authority,
        space = 8 + StakeVaultConfig::INIT_SPACE,
        seeds = [STAKE_VAULT_CONFIG_SEED],
        bump
    )]
    pub stake_vault_config: Account<'info, StakeVaultConfig>,
    /// Rate-manager configuration PDA.
    #[account(
        init,
        payer = authority,
        space = 8 + RateManagerConfig::INIT_SPACE,
        seeds = [RATE_MANAGER_CONFIG_SEED],
        bump
    )]
    pub rate_manager_config: Account<'info, RateManagerConfig>,
    /// Whitelist configuration PDA.
    #[account(
        init,
        payer = authority,
        space = 8 + WhitelistConfig::INIT_SPACE,
        seeds = [WHITELIST_CONFIG_SEED],
        bump
    )]
    pub whitelist_config: Account<'info, WhitelistConfig>,
    /// Dispute configuration PDA.
    #[account(
        init,
        payer = authority,
        space = 8 + DisputeConfig::INIT_SPACE,
        seeds = [DISPUTE_CONFIG_SEED],
        bump
    )]
    pub dispute_config: Account<'info, DisputeConfig>,
    /// System program used to create the root PDA.
    pub system_program: Program<'info, System>,
}

/// Creates the only supported protocol version.
pub fn handle_initialize_protocol(
    ctx: Context<InitializeProtocol>,
    args: InitializeProtocolArgs,
) -> Result<()> {
    require!(args.protocol_fee <= MAX_FEE, Zkp2pError::FeeExceedsMaximum);
    require!(
        args.protocol_fee_recipient != Pubkey::default(),
        Zkp2pError::ZeroAddress
    );
    require!(args.intent_expiration_period > 0, Zkp2pError::ZeroValue);
    require!(
        args.intent_expiration_period <= crate::constants::MAX_INTENT_LIFETIME_SECONDS,
        Zkp2pError::AmountAboveMaximum
    );
    require!(args.max_intents_per_deposit > 0, Zkp2pError::ZeroValue);
    require!(
        args.controller_change_delay >= MIN_CONTROLLER_CHANGE_DELAY_SECONDS,
        Zkp2pError::ControllerNotReady
    );
    require!(!args.initial_witnesses.is_empty(), Zkp2pError::ZeroValue);
    require!(
        usize::from(args.required_signatures) <= args.initial_witnesses.len()
            && args.required_signatures > 0,
        Zkp2pError::InvalidSignature
    );
    require!(
        args.initial_witnesses.len() <= 16,
        Zkp2pError::AmountAboveMaximum
    );
    for witness_index in 0..args.initial_witnesses.len() {
        let witness = args
            .initial_witnesses
            .get(witness_index)
            .ok_or(Zkp2pError::ArithmeticOverflow)?;
        require!(*witness != [0; 20], Zkp2pError::ZeroAddress);
        require!(
            !args
                .initial_witnesses
                .get(..witness_index)
                .ok_or(Zkp2pError::ArithmeticOverflow)?
                .contains(witness),
            Zkp2pError::InvalidSignature
        );
    }

    let protocol = &mut ctx.accounts.protocol;
    protocol.authority = ctx.accounts.authority.key();
    protocol.pending_authority = None;
    protocol.version = 1;
    protocol.bump = ctx.bumps.protocol;

    let escrow = &mut ctx.accounts.escrow_config;
    escrow.protocol = protocol.key();
    escrow.token_mint = ctx.accounts.stake_mint.key();
    escrow.dust_recipient = args.protocol_fee_recipient;
    escrow.dust_threshold = 0;
    escrow.max_intents_per_deposit = args.max_intents_per_deposit;
    escrow.intent_expiration_period = args.intent_expiration_period;
    escrow.next_deposit_id = 0;
    escrow.paused = false;
    escrow.bump = ctx.bumps.escrow_config;

    let verifier = &mut ctx.accounts.verifier_config;
    verifier.protocol = protocol.key();
    verifier.required_signatures = args.required_signatures;
    verifier.witnesses = args.initial_witnesses;
    verifier.payment_methods = Vec::new();
    verifier.bump = ctx.bumps.verifier_config;

    let orchestrator = &mut ctx.accounts.orchestrator_config;
    orchestrator.protocol = protocol.key();
    orchestrator.escrow_config = escrow.key();
    orchestrator.verifier_config = verifier.key();
    orchestrator.protocol_fee = args.protocol_fee;
    orchestrator.protocol_fee_recipient = args.protocol_fee_recipient;
    orchestrator.lifecycle_policy = LifecyclePolicy::WhitelistAndDispute;
    orchestrator.allow_multiple_intents = false;
    orchestrator.next_intent_id = 0;
    orchestrator.paused = false;
    orchestrator.bump = ctx.bumps.orchestrator_config;

    let stake_vault = &mut ctx.accounts.stake_vault_config;
    stake_vault.protocol = protocol.key();
    stake_vault.stake_mint = ctx.accounts.stake_mint.key();
    stake_vault.controller = ctx.accounts.dispute_config.key();
    stake_vault.pending_controller = None;
    stake_vault.pending_controller_valid_at = 0;
    stake_vault.controller_change_delay = args.controller_change_delay;
    stake_vault.total_staked = 0;
    stake_vault.total_claimable = 0;
    stake_vault.bump = ctx.bumps.stake_vault_config;
    stake_vault.vault_authority_bump = ctx.bumps.stake_vault_config;

    let rate_manager = &mut ctx.accounts.rate_manager_config;
    rate_manager.protocol = protocol.key();
    rate_manager.next_manager_id = 0;
    rate_manager.bump = ctx.bumps.rate_manager_config;

    let whitelist = &mut ctx.accounts.whitelist_config;
    whitelist.protocol = protocol.key();
    whitelist.next_group_id = 0;
    whitelist.bump = ctx.bumps.whitelist_config;

    let dispute = &mut ctx.accounts.dispute_config;
    dispute.protocol = protocol.key();
    dispute.stake_vault = stake_vault.key();
    dispute.verifier_config = verifier.key();
    dispute.admissions_paused = false;
    dispute.bump = ctx.bumps.dispute_config;
    Ok(())
}
