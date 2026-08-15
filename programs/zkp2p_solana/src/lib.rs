#![allow(clippy::result_large_err)]
#![allow(missing_docs)]
//! Latest-only ZKP2P settlement program for Solana.

pub mod constants;
pub mod error;
pub mod instructions;
pub mod math;
pub mod state;

use anchor_lang::prelude::*;

pub use instructions::*;
pub use state::*;

declare_id!("5TJD8vLWqAy4hEZLnsxuFKCDnuKXkfQQWpdnqNKYoA1x");

/// Program instruction dispatch.
#[program]
pub mod zkp2p_solana {
    use super::*;

    /// Initializes the canonical latest protocol root.
    pub fn initialize_protocol(
        ctx: Context<InitializeProtocol>,
        args: InitializeProtocolArgs,
    ) -> Result<()> {
        instructions::handle_initialize_protocol(ctx, args)
    }

    /// Creates the one canonical SPL-token custody account for StakeVault liabilities.
    pub fn initialize_stake_token_vault(ctx: Context<InitializeStakeTokenVault>) -> Result<()> {
        instructions::handle_initialize_stake_token_vault(ctx)
    }

    /// Proposes, replaces, or cancels a two-step protocol authority transfer.
    pub fn propose_protocol_authority(
        ctx: Context<ProposeProtocolAuthority>,
        pending: Option<Pubkey>,
    ) -> Result<()> {
        instructions::handle_propose_protocol_authority(ctx, pending)
    }

    /// Accepts a pending protocol authority transfer.
    pub fn accept_protocol_authority(ctx: Context<AcceptProtocolAuthority>) -> Result<()> {
        instructions::handle_accept_protocol_authority(ctx)
    }

    /// Updates bounded future-facing orchestrator settings.
    pub fn configure_orchestrator(
        ctx: Context<ConfigureOrchestrator>,
        args: ConfigureOrchestratorArgs,
    ) -> Result<()> {
        instructions::handle_configure_orchestrator(ctx, args)
    }

    /// Updates bounded future-facing escrow settings.
    pub fn configure_escrow(
        ctx: Context<ConfigureEscrow>,
        args: ConfigureEscrowArgs,
    ) -> Result<()> {
        instructions::handle_configure_escrow(ctx, args)
    }

    /// Proposes or cancels a delayed StakeVault controller handover.
    pub fn propose_stake_controller(
        ctx: Context<ProposeStakeController>,
        pending: Option<Pubkey>,
    ) -> Result<()> {
        instructions::handle_propose_stake_controller(ctx, pending)
    }

    /// Accepts a mature StakeVault controller handover.
    pub fn accept_stake_controller(ctx: Context<AcceptStakeController>) -> Result<()> {
        instructions::handle_accept_stake_controller(ctx)
    }

    /// Deposits exact caller-owned canonical stake principal.
    pub fn deposit_stake(ctx: Context<DepositStake>, amount: u64) -> Result<()> {
        instructions::handle_deposit_stake(ctx, amount)
    }

    /// Withdraws caller-owned free stake principal.
    pub fn withdraw_stake(ctx: Context<WithdrawStake>, amount: u64) -> Result<()> {
        instructions::handle_withdraw_stake(ctx, amount)
    }

    /// Withdraws the caller's complete claimable balance.
    pub fn claim_stake(ctx: Context<ClaimStake>) -> Result<()> {
        instructions::handle_claim_stake(ctx)
    }

    /// Grants or revokes one taker's ability to select caller-owned stake.
    pub fn set_taker_authorization(
        ctx: Context<SetTakerAuthorization>,
        authorized: bool,
    ) -> Result<()> {
        instructions::handle_set_taker_authorization(ctx, authorized)
    }

    /// Selects a currently authorizing third-party stake owner.
    pub fn select_stake_owner(ctx: Context<SelectStakeOwner>) -> Result<()> {
        instructions::handle_select_stake_owner(ctx)
    }

    /// Restores the caller's implicit self-staking fallback.
    pub fn clear_stake_owner(ctx: Context<ClearStakeOwner>) -> Result<()> {
        instructions::handle_clear_stake_owner(ctx)
    }

    /// Lets the active controller lock existing free stake.
    pub fn controller_lock_stake(
        ctx: Context<ControllerLockStake>,
        args: ControllerLockArgs,
    ) -> Result<()> {
        instructions::handle_controller_lock_stake(ctx, args)
    }

    /// Lets the active controller fund and lock new stake atomically.
    pub fn controller_fund_lock(
        ctx: Context<ControllerFundLock>,
        args: ControllerLockArgs,
    ) -> Result<()> {
        instructions::handle_controller_fund_lock(ctx, args)
    }

    /// Increases one pre-maturity lock from the owner's free stake.
    pub fn increase_stake_lock(
        ctx: Context<ManageStakeLock>,
        lock_id: [u8; 32],
        additional_amount: u64,
    ) -> Result<()> {
        instructions::handle_increase_stake_lock(ctx, lock_id, additional_amount)
    }

    /// Shrinks or re-times one pre-maturity lock.
    pub fn resize_stake_lock(
        ctx: Context<ManageStakeLock>,
        lock_id: [u8; 32],
        new_amount: u64,
        new_matures_at: i64,
    ) -> Result<()> {
        instructions::handle_resize_stake_lock(ctx, lock_id, new_amount, new_matures_at)
    }

    /// Unlocks one complete controller lock.
    pub fn controller_unlock_stake(ctx: Context<ManageStakeLock>, lock_id: [u8; 32]) -> Result<()> {
        instructions::handle_controller_unlock_stake(ctx, lock_id)
    }

    /// Permissionlessly prepares one canonical beneficiary claim balance.
    pub fn initialize_claim_balance(ctx: Context<InitializeClaimBalance>) -> Result<()> {
        instructions::handle_initialize_claim_balance(ctx)
    }

    /// Resolves one lock into one or more beneficiary claims.
    pub fn resolve_stake_lock<'info>(
        ctx: Context<'info, ResolveStakeLock<'info>>,
        lock_id: [u8; 32],
        claims: Vec<StakeClaim>,
    ) -> Result<()> {
        instructions::handle_resolve_stake_lock(ctx, lock_id, claims)
    }

    /// Creates one delegated rate manager.
    pub fn create_rate_manager(
        ctx: Context<CreateRateManager>,
        args: CreateRateManagerArgs,
    ) -> Result<()> {
        instructions::handle_create_rate_manager(ctx, args)
    }

    /// Updates mutable manager authority and metadata.
    pub fn set_rate_manager_config(
        ctx: Context<ManageRateManager>,
        manager: Pubkey,
        fee_recipient: Option<Pubkey>,
        name: String,
        uri: String,
    ) -> Result<()> {
        instructions::handle_set_rate_manager_config(ctx, manager, fee_recipient, name, uri)
    }

    /// Updates the manager fee within its immutable ceiling.
    pub fn set_manager_fee(ctx: Context<ManageRateManager>, fee: u128) -> Result<()> {
        instructions::handle_set_manager_fee(ctx, fee)
    }

    /// Updates the minimum liquidity enforced for future deposit opt-ins.
    pub fn set_manager_min_liquidity(
        ctx: Context<ManageRateManager>,
        min_liquidity: u64,
    ) -> Result<()> {
        instructions::handle_set_manager_min_liquidity(ctx, min_liquidity)
    }

    /// Sets or disables one manager-defined payment/currency rate.
    pub fn set_manager_rate(
        ctx: Context<SetManagerRate>,
        payment_method: [u8; 32],
        currency: [u8; 32],
        rate: u128,
    ) -> Result<()> {
        instructions::handle_set_manager_rate(ctx, payment_method, currency, rate)
    }

    /// Creates and exactly funds one deposit with its first active payment tuple.
    pub fn create_deposit(ctx: Context<CreateDeposit>, args: CreateDepositArgs) -> Result<()> {
        instructions::handle_create_deposit(ctx, args)
    }

    /// Creates or refreshes one authority-namespaced market-rate quote.
    pub fn update_oracle_quote(
        ctx: Context<UpdateOracleQuote>,
        quote_id: [u8; 32],
        market_rate: u128,
        valid: bool,
    ) -> Result<()> {
        instructions::handle_update_oracle_quote(ctx, quote_id, market_rate, valid)
    }

    /// Adds exact caller-supplied principal to an existing deposit.
    pub fn add_funds(ctx: Context<AddFunds>, amount: u64) -> Result<()> {
        instructions::handle_add_funds(ctx, amount)
    }

    /// Removes currently available maker principal.
    pub fn remove_funds(ctx: Context<RemoveFunds>, amount: u64) -> Result<()> {
        instructions::handle_remove_funds(ctx, amount)
    }

    /// Returns all available maker principal and closes the deposit to new intents.
    pub fn withdraw_deposit(ctx: Context<WithdrawDeposit>) -> Result<()> {
        instructions::handle_withdraw_deposit(ctx)
    }

    /// Applies maker-controlled deposit settings.
    pub fn update_deposit(ctx: Context<UpdateDeposit>, args: UpdateDepositArgs) -> Result<()> {
        instructions::handle_update_deposit(ctx, args)
    }

    /// Creates or updates one deposit payment method.
    pub fn configure_payment_method(
        ctx: Context<ConfigurePaymentMethod>,
        args: ConfigurePaymentMethodArgs,
    ) -> Result<()> {
        instructions::handle_configure_payment_method(ctx, args)
    }

    /// Creates or updates one deposit payment/currency tuple.
    pub fn configure_currency(
        ctx: Context<ConfigureCurrency>,
        args: ConfigureCurrencyArgs,
    ) -> Result<()> {
        instructions::handle_configure_currency(ctx, args)
    }

    /// Selects or clears a delegated rate manager for one deposit.
    pub fn set_deposit_rate_manager(
        ctx: Context<SetDepositRateManager>,
        manager: Option<Pubkey>,
    ) -> Result<()> {
        instructions::handle_set_deposit_rate_manager(ctx, manager)
    }

    /// Extends one lock under the deposit's configured guardian authority.
    pub fn extend_intent_expiry(
        ctx: Context<ExtendIntentExpiry>,
        additional_time: i64,
    ) -> Result<()> {
        instructions::handle_extend_intent_expiry(ctx, additional_time)
    }

    /// Signals one intent and locks its maker liquidity atomically.
    pub fn signal_intent<'info>(
        ctx: Context<'info, SignalIntent<'info>>,
        args: SignalIntentArgs,
    ) -> Result<()> {
        instructions::handle_signal_intent(ctx, args)
    }

    /// Cancels one caller-owned intent and restores its complete lock.
    pub fn cancel_intent(ctx: Context<CancelIntent>) -> Result<()> {
        instructions::handle_cancel_intent(ctx)
    }

    /// Permissionlessly prunes one strictly expired intent and every paired lock.
    pub fn prune_expired_intent(ctx: Context<PruneExpiredIntent>) -> Result<()> {
        instructions::handle_prune_expired_intent(ctx)
    }

    /// Creates one permissionless curated address group.
    pub fn create_address_group(
        ctx: Context<CreateAddressGroup>,
        name: String,
        public: bool,
    ) -> Result<()> {
        instructions::handle_create_address_group(ctx, name, public)
    }

    /// Updates group admission, resolver, or pending curator state.
    pub fn configure_address_group(
        ctx: Context<ConfigureAddressGroup>,
        public: Option<bool>,
        resolver: Option<Option<Pubkey>>,
        pending_curator: Option<Option<Pubkey>>,
    ) -> Result<()> {
        instructions::handle_configure_address_group(ctx, public, resolver, pending_curator)
    }

    /// Accepts a pending group curator handover.
    pub fn accept_group_curator(ctx: Context<AcceptGroupCurator>) -> Result<()> {
        instructions::handle_accept_group_curator(ctx)
    }

    /// Adds or removes one curator-managed group member.
    pub fn set_group_member(ctx: Context<SetGroupMember>, active: bool) -> Result<()> {
        instructions::handle_set_group_member(ctx, active)
    }

    /// Joins or leaves one public group.
    pub fn set_self_group_member(ctx: Context<SetSelfGroupMember>, active: bool) -> Result<()> {
        instructions::handle_set_self_group_member(ctx, active)
    }

    /// Initializes the whitelist policy for one latest-stack deposit.
    pub fn initialize_deposit_whitelist(
        ctx: Context<InitializeDepositWhitelist>,
        enabled: bool,
    ) -> Result<()> {
        instructions::handle_initialize_deposit_whitelist(ctx, enabled)
    }

    /// Enables or disables one deposit whitelist.
    pub fn set_whitelist_enabled(
        ctx: Context<ConfigureDepositWhitelist>,
        enabled: bool,
    ) -> Result<()> {
        instructions::handle_set_whitelist_enabled(ctx, enabled)
    }

    /// Adds or removes one validated address group from a deposit whitelist.
    pub fn set_deposit_allowed_group(
        ctx: Context<SetDepositAllowedGroup>,
        allowed: bool,
    ) -> Result<()> {
        instructions::handle_set_deposit_allowed_group(ctx, allowed)
    }

    /// Adds or removes one direct taker from a deposit whitelist.
    pub fn set_deposit_whitelist_member(
        ctx: Context<SetDepositWhitelistMember>,
        active: bool,
    ) -> Result<()> {
        instructions::handle_set_deposit_whitelist_member(ctx, active)
    }

    /// Sets the dispute risk window for one payment method.
    pub fn set_risk_window(
        ctx: Context<SetRiskWindow>,
        payment_method: [u8; 32],
        seconds: i64,
    ) -> Result<()> {
        instructions::handle_set_risk_window(ctx, payment_method, seconds)
    }

    /// Pauses or resumes new dispute-protection admissions.
    pub fn set_dispute_admissions_paused(
        ctx: Context<SetDisputeAdmissionsPaused>,
        paused: bool,
    ) -> Result<()> {
        instructions::handle_set_dispute_admissions_paused(ctx, paused)
    }

    /// Explicitly enables or disables dispute protection for one deposit.
    pub fn set_deposit_dispute_protection(
        ctx: Context<SetDepositDisputeProtection>,
        enabled: bool,
    ) -> Result<()> {
        instructions::handle_set_deposit_dispute_protection(ctx, enabled)
    }

    /// Prepares exact stake-backed coverage for the next serialized intent.
    pub fn prepare_dispute(ctx: Context<PrepareDispute>, args: PrepareDisputeArgs) -> Result<()> {
        instructions::handle_prepare_dispute(ctx, args)
    }

    /// Cancels pending dispute coverage before cancelling its paired intent.
    pub fn cancel_dispute(ctx: Context<CancelDispute>) -> Result<()> {
        instructions::handle_cancel_dispute(ctx)
    }

    /// Permissionlessly releases stake after the settlement risk window.
    pub fn release_matured_dispute(ctx: Context<ReleaseMaturedDispute>) -> Result<()> {
        instructions::handle_release_matured_dispute(ctx)
    }

    /// Resolves threshold-signed dispute evidence into a maker stake claim.
    pub fn submit_dispute(ctx: Context<SubmitDispute>, args: SubmitDisputeArgs) -> Result<()> {
        instructions::handle_submit_dispute(ctx, args)
    }

    /// Adds or removes one payment method from the unified verifier.
    pub fn set_verifier_payment_method(
        ctx: Context<ConfigureVerifier>,
        payment_method: [u8; 32],
        enabled: bool,
    ) -> Result<()> {
        instructions::handle_set_verifier_payment_method(ctx, payment_method, enabled)
    }

    /// Adds or removes one authorized Ethereum witness.
    pub fn set_verifier_witness(
        ctx: Context<ConfigureVerifier>,
        witness: [u8; 20],
        enabled: bool,
    ) -> Result<()> {
        instructions::handle_set_verifier_witness(ctx, witness, enabled)
    }

    /// Sets the MultiAttestationVerifier signature threshold.
    pub fn set_required_signatures(ctx: Context<ConfigureVerifier>, required: u8) -> Result<()> {
        instructions::handle_set_required_signatures(ctx, required)
    }

    /// Verifies one payment proof and atomically settles its intent.
    pub fn fulfill_intent<'info>(
        ctx: Context<'info, FulfillIntent<'info>>,
        args: FulfillIntentArgs,
    ) -> Result<()> {
        instructions::handle_fulfill_intent(ctx, args)
    }

    /// Allows a deposit owner to release the complete intent without a payment proof.
    pub fn manual_release<'info>(ctx: Context<'info, ManualRelease<'info>>) -> Result<()> {
        instructions::handle_manual_release(ctx)
    }
}
