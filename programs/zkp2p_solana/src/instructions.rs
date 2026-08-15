//! Anchor instruction contexts and handlers.

use anchor_lang::prelude::*;
use anchor_spl::token_interface::Mint;
use solana_program::{keccak, slot_hashes};

use crate::{
    constants::{
        DEPLOYMENT_DOMAIN_PREFIX, DISPUTE_CONFIG_SEED, ESCROW_CONFIG_SEED, MAX_FEE, MAX_WITNESSES,
        MIN_CONTROLLER_CHANGE_DELAY_SECONDS, ORCHESTRATOR_CONFIG_SEED, PROTOCOL_SEED,
        RATE_MANAGER_CONFIG_SEED, STAKE_VAULT_CONFIG_SEED, VERIFIER_CONFIG_SEED,
        WHITELIST_CONFIG_SEED,
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
    /// This program's executable account, used to derive its canonical ProgramData.
    #[account(
        constraint = program.programdata_address()? == Some(program_data.key())
            @ Zkp2pError::Unauthorized
    )]
    pub program: Program<'info, crate::program::Zkp2pSolana>,
    /// Upgradeable-loader state proving the initializer controls this deployment.
    #[account(
        constraint = program_data.upgrade_authority_address == Some(authority.key())
            @ Zkp2pError::Unauthorized
    )]
    pub program_data: Account<'info, ProgramData>,
    /// CHECK: fixed-address canonical recent cluster-state hashes parsed with strict bounds checks.
    #[account(address = solana_program::sysvar::slot_hashes::ID)]
    pub slot_hashes: UncheckedAccount<'info>,
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
    let slot_hashes_data = ctx.accounts.slot_hashes.try_borrow_data()?;
    let (domain_seed_slot, domain_seed) = newest_slot_hash(&slot_hashes_data)?;
    let domain_chain_id = derive_deployment_domain(&crate::ID, &domain_seed);
    require!(
        domain_chain_id != [0; 32],
        Zkp2pError::InvalidDeploymentDomain
    );
    require_keys_eq!(
        *ctx.accounts.stake_mint.to_account_info().owner,
        anchor_spl::token::ID,
        Zkp2pError::Unauthorized
    );
    require!(
        ctx.accounts.stake_mint.decimals == 6,
        Zkp2pError::Unauthorized
    );
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
        args.initial_witnesses.len() <= MAX_WITNESSES,
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
    protocol.domain_seed_slot = domain_seed_slot;
    protocol.domain_seed = domain_seed;
    protocol.domain_chain_id = domain_chain_id;
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
    verifier.domain_chain_id = domain_chain_id;
    verifier.required_signatures = args.required_signatures;
    verifier.witnesses = args.initial_witnesses;
    verifier.payment_methods = Vec::new();
    verifier.bump = ctx.bumps.verifier_config;

    let orchestrator = &mut ctx.accounts.orchestrator_config;
    orchestrator.protocol = protocol.key();
    orchestrator.escrow_config = escrow.key();
    orchestrator.verifier_config = verifier.key();
    orchestrator.domain_chain_id = domain_chain_id;
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
    dispute.domain_chain_id = domain_chain_id;
    dispute.stake_vault = stake_vault.key();
    dispute.verifier_config = verifier.key();
    dispute.admissions_paused = false;
    dispute.bump = ctx.bumps.dispute_config;
    Ok(())
}

fn derive_deployment_domain(program_id: &Pubkey, domain_seed: &[u8; 32]) -> [u8; 32] {
    keccak::hashv(&[
        DEPLOYMENT_DOMAIN_PREFIX,
        program_id.as_ref(),
        domain_seed.as_ref(),
    ])
    .to_bytes()
}

fn newest_slot_hash(data: &[u8]) -> Result<(u64, [u8; 32])> {
    const HEADER_SIZE: usize = 8;
    const ENTRY_SIZE: usize = 40;
    let count_bytes: [u8; 8] = data
        .get(..HEADER_SIZE)
        .ok_or(Zkp2pError::InvalidDeploymentDomain)?
        .try_into()
        .map_err(|_| error!(Zkp2pError::InvalidDeploymentDomain))?;
    let count = usize::try_from(u64::from_le_bytes(count_bytes))
        .map_err(|_| error!(Zkp2pError::InvalidDeploymentDomain))?;
    require!(
        count > 0 && count <= slot_hashes::MAX_ENTRIES,
        Zkp2pError::InvalidDeploymentDomain
    );
    let required_len = HEADER_SIZE
        .checked_add(
            count
                .checked_mul(ENTRY_SIZE)
                .ok_or(Zkp2pError::ArithmeticOverflow)?,
        )
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    require!(
        data.len() >= required_len,
        Zkp2pError::InvalidDeploymentDomain
    );
    let slot_end = HEADER_SIZE
        .checked_add(8)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    let hash_end = slot_end
        .checked_add(32)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    let slot_bytes: [u8; 8] = data
        .get(HEADER_SIZE..slot_end)
        .ok_or(Zkp2pError::InvalidDeploymentDomain)?
        .try_into()
        .map_err(|_| error!(Zkp2pError::InvalidDeploymentDomain))?;
    let hash: [u8; 32] = data
        .get(slot_end..hash_end)
        .ok_or(Zkp2pError::InvalidDeploymentDomain)?
        .try_into()
        .map_err(|_| error!(Zkp2pError::InvalidDeploymentDomain))?;
    require!(hash != [0; 32], Zkp2pError::InvalidDeploymentDomain);
    Ok((u64::from_le_bytes(slot_bytes), hash))
}

#[cfg(test)]
mod deployment_domain_tests {
    use super::*;

    #[test]
    fn newest_slot_hash_and_domain_derivation_are_exact() {
        let slot = 42_u64;
        let seed = [7_u8; 32];
        let mut data = Vec::new();
        data.extend_from_slice(&1_u64.to_le_bytes());
        data.extend_from_slice(&slot.to_le_bytes());
        data.extend_from_slice(&seed);
        assert_eq!(
            newest_slot_hash(&data).expect("valid slot hashes"),
            (slot, seed)
        );
        assert_ne!(
            derive_deployment_domain(&crate::ID, &seed),
            derive_deployment_domain(&crate::ID, &[8_u8; 32])
        );
        assert_ne!(
            derive_deployment_domain(&crate::ID, &seed),
            derive_deployment_domain(&Pubkey::new_unique(), &seed)
        );
        assert!(newest_slot_hash(data.get(..12).expect("truncated fixture")).is_err());
        assert!(newest_slot_hash(&0_u64.to_le_bytes()).is_err());
        let mut zero_hash_data = Vec::new();
        zero_hash_data.extend_from_slice(&1_u64.to_le_bytes());
        zero_hash_data.extend_from_slice(&slot.to_le_bytes());
        zero_hash_data.extend_from_slice(&[0_u8; 32]);
        assert!(newest_slot_hash(&zero_hash_data).is_err());
    }
}
