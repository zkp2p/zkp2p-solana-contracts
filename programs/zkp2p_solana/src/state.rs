//! Program-owned account types.

use anchor_lang::prelude::*;

/// Root authority and component version marker.
#[account]
#[derive(InitSpace)]
pub struct ProtocolConfig {
    /// Governance authority.
    pub authority: Pubkey,
    /// Pending two-step authority.
    pub pending_authority: Option<Pubkey>,
    /// Schema version. Only the latest version is supported.
    pub version: u8,
    /// PDA bump.
    pub bump: u8,
}

/// Initializes every logical component against one canonical stake mint.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct InitializeProtocolArgs {
    /// Protocol fee at 1e18 precision.
    pub protocol_fee: u128,
    /// Recipient of protocol fees.
    pub protocol_fee_recipient: Pubkey,
    /// Escrow intent expiry in seconds.
    pub intent_expiration_period: i64,
    /// Maximum live intents for one deposit.
    pub max_intents_per_deposit: u16,
    /// Stake-vault controller handover delay.
    pub controller_change_delay: i64,
    /// Initial authorized Ethereum witness addresses.
    pub initial_witnesses: Vec<[u8; 20]>,
    /// Initial witness threshold.
    pub required_signatures: u8,
}

/// Lifecycle policy snapshotted by a newly signaled intent.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, InitSpace, PartialEq, Eq)]
pub enum LifecyclePolicy {
    /// No whitelist or dispute callbacks.
    None,
    /// Persistent whitelist admission only.
    Whitelist,
    /// Whitelist fast lane plus stake-backed dispute protection.
    WhitelistAndDispute,
}

/// Escrow-wide configuration.
#[account]
#[derive(InitSpace)]
pub struct EscrowConfig {
    /// Protocol root that governs this component.
    pub protocol: Pubkey,
    /// Canonical token mint accepted by the latest stack.
    pub token_mint: Pubkey,
    /// Recipient of swept deposit dust.
    pub dust_recipient: Pubkey,
    /// Maximum amount swept when an empty deposit closes.
    pub dust_threshold: u64,
    /// Maximum simultaneously active locks on one deposit.
    pub max_intents_per_deposit: u16,
    /// Default intent lock lifetime in seconds.
    pub intent_expiration_period: i64,
    /// Monotonic deposit identifier source.
    pub next_deposit_id: u64,
    /// Whether new deposits and locks are paused.
    pub paused: bool,
    /// PDA bump.
    pub bump: u8,
}

/// Orchestrator-wide configuration.
#[account]
#[derive(InitSpace)]
pub struct OrchestratorConfig {
    /// Protocol root that governs this component.
    pub protocol: Pubkey,
    /// Escrow component used by this orchestrator.
    pub escrow_config: Pubkey,
    /// Unified verifier component used by this orchestrator.
    pub verifier_config: Pubkey,
    /// Fee paid from each released amount at 1e18 precision.
    pub protocol_fee: u128,
    /// Recipient of protocol fees.
    pub protocol_fee_recipient: Pubkey,
    /// Policy snapshotted by future intents.
    pub lifecycle_policy: LifecyclePolicy,
    /// Whether ordinary takers may have multiple live intents.
    pub allow_multiple_intents: bool,
    /// Monotonic intent identifier source.
    pub next_intent_id: u64,
    /// Whether new signals and proof fulfillments are paused.
    pub paused: bool,
    /// PDA bump.
    pub bump: u8,
}

/// Inclusive per-intent amount bounds.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, InitSpace, PartialEq, Eq)]
pub struct AmountRange {
    /// Smallest admissible lock.
    pub min: u64,
    /// Largest admissible lock.
    pub max: u64,
}

/// Escrow deposit and liquidity accounting.
#[account]
#[derive(InitSpace)]
pub struct Deposit {
    /// Escrow component that owns this deposit.
    pub escrow_config: Pubkey,
    /// Monotonic identifier within the escrow component.
    pub id: u64,
    /// Maker who owns configuration and withdrawal authority.
    pub depositor: Pubkey,
    /// Optional configuration delegate.
    pub delegate: Option<Pubkey>,
    /// Canonical SPL token mint.
    pub token_mint: Pubkey,
    /// Per-intent amount bounds.
    pub intent_amount_range: AmountRange,
    /// Whether new locks may be admitted.
    pub accepting_intents: bool,
    /// Immediately available liquidity.
    pub remaining_deposits: u64,
    /// Principal committed across active locks.
    pub outstanding_intent_amount: u64,
    /// Number of active lock PDAs.
    pub active_intents: u16,
    /// Optional authority that may extend lock expiry.
    pub intent_guardian: Option<Pubkey>,
    /// Whether an empty deposit preserves its configuration.
    pub retain_on_empty: bool,
    /// Optional delegated rate manager account.
    pub rate_manager: Option<Pubkey>,
    /// PDA bump.
    pub bump: u8,
    /// Authority bump for the deposit token vault.
    pub vault_authority_bump: u8,
}

impl Deposit {
    /// Returns total maker principal still represented by this deposit.
    pub fn total_liquidity(&self) -> Option<u64> {
        self.remaining_deposits
            .checked_add(self.outstanding_intent_amount)
    }

    /// Moves available liquidity into a new escrow lock.
    pub fn lock(&mut self, amount: u64, maximum_intents: u16) -> Result<()> {
        require!(
            self.accepting_intents,
            crate::error::Zkp2pError::DepositNotAcceptingIntents
        );
        require!(
            amount >= self.intent_amount_range.min,
            crate::error::Zkp2pError::AmountBelowMinimum
        );
        require!(
            amount <= self.intent_amount_range.max,
            crate::error::Zkp2pError::AmountAboveMaximum
        );
        require!(
            self.remaining_deposits >= amount,
            crate::error::Zkp2pError::InsufficientBalance
        );
        let new_count = self
            .active_intents
            .checked_add(1)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        require!(
            new_count <= maximum_intents,
            crate::error::Zkp2pError::MaximumIntentsExceeded
        );
        self.remaining_deposits = self
            .remaining_deposits
            .checked_sub(amount)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        self.outstanding_intent_amount = self
            .outstanding_intent_amount
            .checked_add(amount)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        self.active_intents = new_count;
        Ok(())
    }

    /// Cancels a lock and returns its complete principal to available liquidity.
    pub fn unlock(&mut self, amount: u64) -> Result<()> {
        self.outstanding_intent_amount = self
            .outstanding_intent_amount
            .checked_sub(amount)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        self.remaining_deposits = self
            .remaining_deposits
            .checked_add(amount)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        self.active_intents = self
            .active_intents
            .checked_sub(1)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        Ok(())
    }

    /// Settles a lock and returns any unreleased remainder to available liquidity.
    pub fn settle(&mut self, locked_amount: u64, release_amount: u64) -> Result<()> {
        require!(release_amount > 0, crate::error::Zkp2pError::ZeroValue);
        require!(
            release_amount <= locked_amount,
            crate::error::Zkp2pError::AmountAboveMaximum
        );
        self.outstanding_intent_amount = self
            .outstanding_intent_amount
            .checked_sub(locked_amount)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        let remainder = locked_amount
            .checked_sub(release_amount)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        self.remaining_deposits = self
            .remaining_deposits
            .checked_add(remainder)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        self.active_intents = self
            .active_intents
            .checked_sub(1)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        Ok(())
    }
}

/// Payment-method configuration for one deposit.
#[account]
#[derive(InitSpace)]
pub struct DepositPaymentMethod {
    /// Parent deposit.
    pub deposit: Pubkey,
    /// Payment-method identifier.
    pub payment_method: [u8; 32],
    /// Hashed payee identifier.
    pub payee_details: [u8; 32],
    /// Optional ed25519 gating authority.
    pub gating_service: Option<Pubkey>,
    /// Whether future intents may use this method.
    pub active: bool,
    /// PDA bump.
    pub bump: u8,
}

/// Arguments for creating one funded deposit with its first active payment tuple.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct CreateDepositArgs {
    /// Exact token amount transferred into custody.
    pub amount: u64,
    /// Inclusive per-intent amount range.
    pub intent_amount_range: AmountRange,
    /// Optional configuration delegate.
    pub delegate: Option<Pubkey>,
    /// Optional intent-expiry guardian.
    pub intent_guardian: Option<Pubkey>,
    /// Preserve configuration when liquidity becomes empty.
    pub retain_on_empty: bool,
    /// First active payment method.
    pub payment_method: [u8; 32],
    /// Hashed payee identifier.
    pub payee_details: [u8; 32],
    /// Optional intent gating authority.
    pub gating_service: Option<Pubkey>,
    /// First listed fiat currency.
    pub currency: [u8; 32],
    /// Fixed minimum conversion rate.
    pub fixed_min_rate: u128,
    /// Optional trusted oracle quote account.
    pub oracle_quote: Option<Pubkey>,
    /// Signed market-rate spread in basis points.
    pub spread_bps: i16,
    /// Maximum oracle staleness in seconds.
    pub max_staleness: u32,
}

/// Latest quote written by a trusted oracle authority.
#[account]
#[derive(InitSpace)]
pub struct OracleQuote {
    /// Authority permitted to refresh the quote.
    pub authority: Pubkey,
    /// Market rate at 1e18 precision.
    pub market_rate: u128,
    /// Quote update time in Unix seconds.
    pub updated_at: i64,
    /// Whether the adapter considers the quote valid.
    pub valid: bool,
    /// PDA bump.
    pub bump: u8,
}

/// Rate configuration for one deposit, payment method, and fiat currency.
#[account]
#[derive(InitSpace)]
pub struct DepositCurrency {
    /// Parent deposit.
    pub deposit: Pubkey,
    /// Payment-method identifier.
    pub payment_method: [u8; 32],
    /// Fiat-currency identifier.
    pub currency: [u8; 32],
    /// Fixed floor at 1e18 precision.
    pub fixed_min_rate: u128,
    /// Optional trusted oracle quote account.
    pub oracle_quote: Option<Pubkey>,
    /// Signed spread in basis points.
    pub spread_bps: i16,
    /// Maximum quote age in seconds.
    pub max_staleness: u32,
    /// Whether this tuple is listed.
    pub listed: bool,
    /// PDA bump.
    pub bump: u8,
}

impl DepositCurrency {
    /// Returns the escrow floor, halting at zero for an unusable configured oracle.
    pub fn escrow_floor(&self, quote: Option<&OracleQuote>, now: i64) -> Result<u128> {
        if !self.listed {
            return Ok(0);
        }
        let spread_rate = match self.oracle_quote {
            None => 0,
            Some(_) => {
                let oracle = quote.ok_or(crate::error::Zkp2pError::InvalidOracleQuote)?;
                if !oracle.valid
                    || oracle.market_rate == 0
                    || oracle.updated_at <= 0
                    || oracle.updated_at > now
                {
                    return Ok(0);
                }
                let age = now
                    .checked_sub(oracle.updated_at)
                    .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
                if age > i64::from(self.max_staleness) {
                    return Ok(0);
                }
                crate::math::spread_rate_ceil(oracle.market_rate, self.spread_bps)?
            }
        };
        Ok(self.fixed_min_rate.max(spread_rate))
    }
}

/// Escrow-owned lock for one orchestrator intent.
#[account]
#[derive(InitSpace)]
pub struct EscrowIntentLock {
    /// Parent deposit.
    pub deposit: Pubkey,
    /// Canonical intent identifier.
    pub intent_hash: [u8; 32],
    /// Orchestrator configuration that created this lock.
    pub orchestrator: Pubkey,
    /// Locked token amount.
    pub amount: u64,
    /// Signal time in Unix seconds.
    pub timestamp: i64,
    /// Strict expiry boundary in Unix seconds.
    pub expiry_time: i64,
    /// PDA bump.
    pub bump: u8,
}

/// Fee paid to one referral recipient.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, InitSpace, PartialEq, Eq)]
pub struct ReferralFee {
    /// Recipient account.
    pub recipient: Pubkey,
    /// Fee at 1e18 precision.
    pub fee: u128,
}

/// Orchestrator-owned canonical intent snapshot.
#[account]
#[derive(InitSpace)]
pub struct Intent {
    /// Orchestrator configuration that owns this intent.
    pub orchestrator: Pubkey,
    /// Canonical identifier.
    pub intent_hash: [u8; 32],
    /// Monotonic nonce used to derive this intent and its PDA.
    pub nonce: u64,
    /// Taker that signaled the intent.
    pub owner: Pubkey,
    /// Final token recipient.
    pub recipient: Pubkey,
    /// Parent escrow deposit.
    pub deposit: Pubkey,
    /// Locked token amount.
    pub amount: u64,
    /// Signal time in Unix seconds.
    pub timestamp: i64,
    /// Payment method.
    pub payment_method: [u8; 32],
    /// Fiat currency.
    pub fiat_currency: [u8; 32],
    /// Snapshotted conversion rate at 1e18 precision.
    pub conversion_rate: u128,
    /// Snapshotted hashed payee identifier.
    pub payee_id: [u8; 32],
    /// Manager fee recipient at signal time.
    pub manager_fee_recipient: Option<Pubkey>,
    /// Manager fee at signal time.
    pub manager_fee: u128,
    /// Lifecycle policy at signal time.
    pub lifecycle_policy: LifecyclePolicy,
    /// Whether a nonzero risk window created stake-backed coverage.
    pub dispute_covered: bool,
    /// Referral fees at signal time.
    #[max_len(10)]
    pub referral_fees: Vec<ReferralFee>,
    /// PDA bump.
    pub bump: u8,
}

/// Live intent count for a taker.
#[account]
#[derive(InitSpace)]
pub struct TakerIntentState {
    /// Orchestrator component.
    pub orchestrator: Pubkey,
    /// Taker account.
    pub taker: Pubkey,
    /// Number of live intents.
    pub active_intents: u16,
    /// PDA bump.
    pub bump: u8,
}

/// Delegated rate-manager metadata and fee terms.
#[account]
#[derive(InitSpace)]
pub struct RateManager {
    /// Component configuration.
    pub config: Pubkey,
    /// Monotonic creation nonce used by the canonical PDA.
    pub nonce: u64,
    /// Deterministic manager identifier.
    pub id: [u8; 32],
    /// Current manager authority.
    pub manager: Pubkey,
    /// Optional fee recipient.
    pub fee_recipient: Option<Pubkey>,
    /// Immutable manager fee ceiling.
    pub max_fee: u128,
    /// Current manager fee.
    pub fee: u128,
    /// Minimum total deposit liquidity required at opt-in.
    pub min_liquidity: u64,
    /// Display name.
    #[max_len(64)]
    pub name: String,
    /// Metadata URI.
    #[max_len(200)]
    pub uri: String,
    /// PDA bump.
    pub bump: u8,
}

/// Arguments for creating one delegated rate manager.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct CreateRateManagerArgs {
    /// Current manager authority.
    pub manager: Pubkey,
    /// Optional fee recipient.
    pub fee_recipient: Option<Pubkey>,
    /// Immutable manager fee ceiling.
    pub max_fee: u128,
    /// Initial fee.
    pub fee: u128,
    /// Minimum deposit liquidity at opt-in.
    pub min_liquidity: u64,
    /// Display name.
    pub name: String,
    /// Metadata URI.
    pub uri: String,
}

/// One manager-defined payment/currency rate.
#[account]
#[derive(InitSpace)]
pub struct RateEntry {
    /// Parent manager.
    pub rate_manager: Pubkey,
    /// Payment method.
    pub payment_method: [u8; 32],
    /// Fiat currency.
    pub currency: [u8; 32],
    /// Rate at 1e18 precision; zero disables the tuple.
    pub rate: u128,
    /// PDA bump.
    pub bump: u8,
}

/// Rate-manager component configuration.
#[account]
#[derive(InitSpace)]
pub struct RateManagerConfig {
    /// Protocol root.
    pub protocol: Pubkey,
    /// Monotonic manager identifier source.
    pub next_manager_id: u64,
    /// PDA bump.
    pub bump: u8,
}

/// Aggregate stake-vault configuration and liabilities.
#[account]
#[derive(InitSpace)]
pub struct StakeVaultConfig {
    /// Protocol root.
    pub protocol: Pubkey,
    /// Canonical stake mint.
    pub stake_mint: Pubkey,
    /// Current lock-policy controller.
    pub controller: Pubkey,
    /// Proposed replacement controller.
    pub pending_controller: Option<Pubkey>,
    /// Earliest accepted handover time.
    pub pending_controller_valid_at: i64,
    /// Immutable handover delay.
    pub controller_change_delay: i64,
    /// Aggregate stake principal.
    pub total_staked: u64,
    /// Aggregate immediately claimable liability.
    pub total_claimable: u64,
    /// PDA bump.
    pub bump: u8,
    /// Authority bump for the token vault.
    pub vault_authority_bump: u8,
}

/// Aggregate principal for one stake owner.
#[account]
#[derive(InitSpace)]
pub struct StakePosition {
    /// Stake-vault configuration.
    pub vault: Pubkey,
    /// Principal owner.
    pub owner: Pubkey,
    /// Total principal including free and locked stake.
    pub balance: u64,
    /// Principal committed across active locks.
    pub locked: u64,
    /// PDA bump.
    pub bump: u8,
}

impl StakePosition {
    /// Returns principal available for withdrawal or a new lock.
    pub fn free(&self) -> Option<u64> {
        self.balance.checked_sub(self.locked)
    }

    /// Commits free principal to a new or increased lock.
    pub fn lock(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, crate::error::Zkp2pError::ZeroValue);
        let free = self
            .free()
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        require!(
            free >= amount,
            crate::error::Zkp2pError::InsufficientFreeStake
        );
        self.locked = self
            .locked
            .checked_add(amount)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        Ok(())
    }

    /// Reduces locked principal without changing total owner principal.
    pub fn unlock(&mut self, amount: u64) -> Result<()> {
        self.locked = self
            .locked
            .checked_sub(amount)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        Ok(())
    }

    /// Converts part of a removed lock into beneficiary claim liabilities.
    pub fn resolve(&mut self, lock_amount: u64, claims_amount: u64) -> Result<()> {
        require!(
            claims_amount <= lock_amount,
            crate::error::Zkp2pError::ClaimsExceedLock
        );
        self.unlock(lock_amount)?;
        self.balance = self
            .balance
            .checked_sub(claims_amount)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        Ok(())
    }
}

/// One stake owner's authorization of one taker.
#[account]
#[derive(InitSpace)]
pub struct TakerAuthorization {
    /// Stake owner granting authority.
    pub stake_owner: Pubkey,
    /// Authorized taker.
    pub taker: Pubkey,
    /// Live authorization value.
    pub authorized: bool,
    /// PDA bump.
    pub bump: u8,
}

/// One taker's selected third-party stake owner.
#[account]
#[derive(InitSpace)]
pub struct StakeSelection {
    /// Taker selecting collateral.
    pub taker: Pubkey,
    /// Selected owner; default means self fallback.
    pub selected_owner: Option<Pubkey>,
    /// PDA bump.
    pub bump: u8,
}

/// Active policy-owned collateral lock.
#[account]
#[derive(InitSpace)]
pub struct StakeLock {
    /// Stake-vault configuration.
    pub vault: Pubkey,
    /// Controller-defined lock identifier.
    pub id: [u8; 32],
    /// Principal owner.
    pub stake_owner: Pubkey,
    /// Locked amount.
    pub amount: u64,
    /// Timestamp after which increase and resize reject.
    pub matures_at: i64,
    /// PDA bump.
    pub bump: u8,
}

/// Immediately withdrawable beneficiary liability.
#[account]
#[derive(InitSpace)]
pub struct ClaimBalance {
    /// Stake-vault configuration.
    pub vault: Pubkey,
    /// Beneficiary.
    pub beneficiary: Pubkey,
    /// Complete claimable balance.
    pub amount: u64,
    /// PDA bump.
    pub bump: u8,
}

/// Payment verifier witness and method configuration.
#[account]
#[derive(InitSpace)]
pub struct VerifierConfig {
    /// Protocol root.
    pub protocol: Pubkey,
    /// Number of unique authorized signatures required.
    pub required_signatures: u8,
    /// Authorized Ethereum witness addresses.
    #[max_len(16)]
    pub witnesses: Vec<[u8; 20]>,
    /// Enabled payment methods.
    #[max_len(64)]
    pub payment_methods: Vec<[u8; 32]>,
    /// PDA bump.
    pub bump: u8,
}

/// Whitelist and address-group component configuration.
#[account]
#[derive(InitSpace)]
pub struct WhitelistConfig {
    /// Protocol root.
    pub protocol: Pubkey,
    /// Monotonic group identifier source.
    pub next_group_id: u64,
    /// PDA bump.
    pub bump: u8,
}

/// Stake-backed dispute component configuration.
#[account]
#[derive(InitSpace)]
pub struct DisputeConfig {
    /// Protocol root.
    pub protocol: Pubkey,
    /// Stake-vault component.
    pub stake_vault: Pubkey,
    /// Unified verifier component providing payment bindings.
    pub verifier_config: Pubkey,
    /// Whether new covered admissions are paused.
    pub admissions_paused: bool,
    /// PDA bump.
    pub bump: u8,
}

/// Immutable payment nullifier-to-intent binding.
#[account]
#[derive(InitSpace)]
pub struct PaymentBinding {
    /// Verifier configuration.
    pub verifier: Pubkey,
    /// Payment-method-scoped nullifier.
    pub nullifier: [u8; 32],
    /// Bound canonical intent.
    pub intent_hash: [u8; 32],
    /// PDA bump.
    pub bump: u8,
}

/// Immutable intent-to-payment reverse binding.
#[account]
#[derive(InitSpace)]
pub struct IntentPaymentBinding {
    /// Verifier configuration.
    pub verifier: Pubkey,
    /// Bound canonical intent.
    pub intent_hash: [u8; 32],
    /// Payment-method-scoped nullifier.
    pub nullifier: [u8; 32],
    /// PDA bump.
    pub bump: u8,
}

/// Whitelist group with two-step curator rotation.
#[account]
#[derive(InitSpace)]
pub struct AddressGroup {
    /// Whitelist component.
    pub whitelist_config: Pubkey,
    /// Monotonic creation nonce used by the canonical PDA.
    pub nonce: u64,
    /// Deterministic group identifier.
    pub id: [u8; 32],
    /// Current curator.
    pub curator: Pubkey,
    /// Proposed curator.
    pub pending_curator: Option<Pubkey>,
    /// Whether accounts may self-join.
    pub public: bool,
    /// Optional external resolver program/account.
    pub resolver: Option<Pubkey>,
    /// Display name.
    #[max_len(64)]
    pub name: String,
    /// PDA bump.
    pub bump: u8,
}

/// Explicit member record for one group and account.
#[account]
#[derive(InitSpace)]
pub struct GroupMember {
    /// Parent group.
    pub group: Pubkey,
    /// Member account.
    pub member: Pubkey,
    /// Live membership value.
    pub active: bool,
    /// PDA bump.
    pub bump: u8,
}

/// Per-deposit whitelist configuration.
#[account]
#[derive(InitSpace)]
pub struct DepositWhitelist {
    /// Parent deposit.
    pub deposit: Pubkey,
    /// Whether enforcement is active.
    pub enabled: bool,
    /// Permanent one-time bootstrap marker.
    pub bootstrapped: bool,
    /// Allowed group identifiers.
    #[max_len(10)]
    pub allowed_groups: Vec<[u8; 32]>,
    /// PDA bump.
    pub bump: u8,
}

/// Direct whitelist membership for one deposit and taker.
#[account]
#[derive(InitSpace)]
pub struct DepositWhitelistMember {
    /// Parent whitelist policy.
    pub deposit_whitelist: Pubkey,
    /// Taker account.
    pub taker: Pubkey,
    /// Live membership value.
    pub active: bool,
    /// PDA bump.
    pub bump: u8,
}

/// Dispute protection lifecycle status.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, InitSpace, PartialEq, Eq)]
pub enum DisputeStatus {
    /// Collateral is locked before settlement.
    Pending,
    /// Intent ended without settlement.
    Cancelled,
    /// Intent settled and remains disputable.
    Settled,
    /// Risk window elapsed and collateral was returned.
    Released,
    /// Valid dispute converted collateral into a depositor claim.
    Disputed,
}

/// Stake-backed dispute protection state for one intent.
#[account]
#[derive(InitSpace)]
pub struct DisputeIntent {
    /// Dispute component configuration.
    pub dispute_config: Pubkey,
    /// Canonical intent identifier.
    pub intent_hash: [u8; 32],
    /// Escrow deposit covered by this lock.
    pub deposit: Pubkey,
    /// Original taker.
    pub taker: Pubkey,
    /// Snapshotted collateral owner.
    pub stake_owner: Pubkey,
    /// Escrow depositor compensated by a valid dispute.
    pub depositor: Pubkey,
    /// Payment method.
    pub payment_method: [u8; 32],
    /// Principal initially locked for coverage.
    pub locked_amount: u64,
    /// Current lifecycle status.
    pub status: DisputeStatus,
    /// Snapshotted risk window in seconds.
    pub risk_window: i64,
    /// Earliest permissionless collateral release.
    pub release_eligible_at: i64,
    /// Settled release amount.
    pub release_amount: u64,
    /// PDA bump.
    pub bump: u8,
}

impl DisputeIntent {
    /// Cancels pending coverage; the paired stake lock remains authoritative for amount.
    pub fn cancel(&mut self) -> Result<()> {
        require!(
            self.status == DisputeStatus::Pending,
            crate::error::Zkp2pError::DisputeIntentNotPending
        );
        self.status = DisputeStatus::Cancelled;
        Ok(())
    }

    /// Snapshots settlement exposure and its release-eligibility timestamp.
    pub fn settle(&mut self, release_amount: u64, now: i64) -> Result<i64> {
        require!(
            self.status == DisputeStatus::Pending,
            crate::error::Zkp2pError::DisputeIntentNotPending
        );
        require!(release_amount > 0, crate::error::Zkp2pError::ZeroValue);
        let release_eligible_at = now
            .checked_add(self.risk_window)
            .ok_or(crate::error::Zkp2pError::ArithmeticOverflow)?;
        self.release_amount = release_amount;
        self.release_eligible_at = release_eligible_at;
        self.status = DisputeStatus::Settled;
        Ok(release_eligible_at)
    }

    /// Releases settled collateral after the snapshotted risk window.
    pub fn release(&mut self, now: i64) -> Result<u64> {
        require!(
            self.status == DisputeStatus::Settled,
            crate::error::Zkp2pError::DisputeIntentNotSettled
        );
        require!(
            now >= self.release_eligible_at,
            crate::error::Zkp2pError::NotReleaseEligible
        );
        self.status = DisputeStatus::Released;
        Ok(self.release_amount)
    }

    /// Marks settled collateral as converted to a depositor claim.
    pub fn dispute(&mut self) -> Result<u64> {
        require!(
            self.status == DisputeStatus::Settled,
            crate::error::Zkp2pError::DisputeIntentNotSettled
        );
        self.status = DisputeStatus::Disputed;
        Ok(self.release_amount)
    }
}

/// Payment-method risk configuration.
#[account]
#[derive(InitSpace)]
pub struct RiskWindow {
    /// Dispute component configuration.
    pub dispute_config: Pubkey,
    /// Payment method.
    pub payment_method: [u8; 32],
    /// Risk window in seconds; zero means pass-through.
    pub seconds: i64,
    /// PDA bump.
    pub bump: u8,
}

/// Deposit opt-out from default-on dispute protection.
#[account]
#[derive(InitSpace)]
pub struct DepositDisputeSetting {
    /// Parent deposit.
    pub deposit: Pubkey,
    /// Whether coverage is enabled.
    pub enabled: bool,
    /// PDA bump.
    pub bump: u8,
}

/// Consumed dispute replay key.
#[account]
#[derive(InitSpace)]
pub struct DisputeNullifier {
    /// Dispute component configuration.
    pub dispute_config: Pubkey,
    /// Payment-method-scoped replay key.
    pub nullifier: [u8; 32],
    /// Intent whose dispute consumed it.
    pub intent_hash: [u8; 32],
    /// PDA bump.
    pub bump: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn deposit() -> Deposit {
        Deposit {
            escrow_config: Pubkey::new_unique(),
            id: 7,
            depositor: Pubkey::new_unique(),
            delegate: None,
            token_mint: Pubkey::new_unique(),
            intent_amount_range: AmountRange { min: 10, max: 100 },
            accepting_intents: true,
            remaining_deposits: 200,
            outstanding_intent_amount: 0,
            active_intents: 0,
            intent_guardian: None,
            retain_on_empty: false,
            rate_manager: None,
            bump: 1,
            vault_authority_bump: 2,
        }
    }

    fn bounded_nonzero(seed: u64, inclusive_maximum: u64) -> u64 {
        seed.checked_rem(inclusive_maximum)
            .unwrap_or(0)
            .saturating_add(1)
    }

    #[test]
    fn escrow_lock_cancel_conserves_liquidity() {
        let mut state = deposit();
        let before = state.total_liquidity().expect("total");
        state.lock(60, 4).expect("lock");
        assert_eq!(state.remaining_deposits, 140);
        assert_eq!(state.outstanding_intent_amount, 60);
        assert_eq!(state.total_liquidity(), Some(before));
        state.unlock(60).expect("unlock");
        assert_eq!(state.total_liquidity(), Some(before));
        assert_eq!(state.active_intents, 0);
    }

    #[test]
    fn partial_settlement_returns_unused_principal() {
        let mut state = deposit();
        state.lock(60, 4).expect("lock");
        state.settle(60, 25).expect("settle");
        assert_eq!(state.remaining_deposits, 175);
        assert_eq!(state.outstanding_intent_amount, 0);
        assert_eq!(state.total_liquidity(), Some(175));
    }

    #[test]
    fn lock_rejects_range_liquidity_and_cap_boundaries() {
        let mut state = deposit();
        state.accepting_intents = false;
        assert!(state.lock(10, 4).is_err());
        state.accepting_intents = true;
        assert!(state.lock(9, 4).is_err());
        assert!(state.lock(101, 4).is_err());
        state.intent_amount_range.max = 1_000;
        assert!(state.lock(201, 4).is_err());
        state.active_intents = 4;
        assert!(state.lock(10, 4).is_err());
    }

    #[test]
    fn stake_lock_resolution_conserves_owner_and_claim_liabilities() {
        let mut position = StakePosition {
            vault: Pubkey::new_unique(),
            owner: Pubkey::new_unique(),
            balance: 100,
            locked: 0,
            bump: 1,
        };
        position.lock(80).expect("lock");
        assert_eq!(position.free(), Some(20));
        position.resolve(80, 35).expect("resolve");
        assert_eq!(position.balance, 65);
        assert_eq!(position.locked, 0);
        assert_eq!(position.free(), Some(65));
        assert_eq!(position.balance.checked_add(35), Some(100));
        assert!(position.lock(66).is_err());
        assert!(position.resolve(1, 2).is_err());
    }

    fn pending_dispute() -> DisputeIntent {
        DisputeIntent {
            dispute_config: Pubkey::new_unique(),
            intent_hash: [7; 32],
            deposit: Pubkey::new_unique(),
            taker: Pubkey::new_unique(),
            stake_owner: Pubkey::new_unique(),
            depositor: Pubkey::new_unique(),
            payment_method: [8; 32],
            locked_amount: 100,
            status: DisputeStatus::Pending,
            risk_window: 600,
            release_eligible_at: 0,
            release_amount: 0,
            bump: 1,
        }
    }

    #[test]
    fn dispute_settlement_has_two_exclusive_terminals() {
        let mut released = pending_dispute();
        assert_eq!(released.settle(40, 1_000), Ok(1_600));
        assert!(released.release(1_599).is_err());
        assert_eq!(released.release(1_600), Ok(40));
        assert!(released.dispute().is_err());

        let mut disputed = pending_dispute();
        disputed.settle(25, 1_000).expect("settle");
        assert_eq!(disputed.dispute(), Ok(25));
        assert!(disputed.release(2_000).is_err());
        assert!(disputed.settle(1, 2_000).is_err());
    }

    #[test]
    fn dispute_cancel_is_pending_only() {
        let mut state = pending_dispute();
        assert!(state.cancel().is_ok());
        assert_eq!(state.status, DisputeStatus::Cancelled);
        assert!(state.cancel().is_err());

        let mut settled = pending_dispute();
        settled.settle(50, 10).expect("settle");
        assert!(settled.cancel().is_err());
    }

    #[test]
    fn oracle_floor_halts_on_every_invalid_boundary() {
        let quote_key = Pubkey::new_unique();
        let currency = DepositCurrency {
            deposit: Pubkey::new_unique(),
            payment_method: [1; 32],
            currency: [2; 32],
            fixed_min_rate: 100,
            oracle_quote: Some(quote_key),
            spread_bps: 100,
            max_staleness: 60,
            listed: true,
            bump: 1,
        };
        let mut quote = OracleQuote {
            authority: Pubkey::new_unique(),
            market_rate: 200,
            updated_at: 1_000,
            valid: true,
            bump: 1,
        };
        assert_eq!(currency.escrow_floor(Some(&quote), 1_060), Ok(202));
        assert_eq!(currency.escrow_floor(Some(&quote), 1_061), Ok(0));
        quote.valid = false;
        assert_eq!(currency.escrow_floor(Some(&quote), 1_000), Ok(0));
        quote.valid = true;
        quote.updated_at = 1_001;
        assert_eq!(currency.escrow_floor(Some(&quote), 1_000), Ok(0));
        assert!(currency.escrow_floor(None, 1_000).is_err());

        let mut fixed = currency;
        fixed.oracle_quote = None;
        assert_eq!(fixed.escrow_floor(None, 1_000), Ok(100));
        fixed.listed = false;
        assert_eq!(fixed.escrow_floor(None, 1_000), Ok(0));
    }

    #[test]
    fn settlement_rejects_release_above_lock() {
        let mut state = deposit();
        state.lock(50, 1).expect("lock");
        assert!(state.settle(50, 51).is_err());
    }

    proptest! {
        #[test]
        fn escrow_lock_cancel_invariant(
            total in 1_u64..1_000_000_000,
            amount_seed in any::<u64>(),
        ) {
            let amount = bounded_nonzero(amount_seed, total);
            let mut state = deposit();
            state.intent_amount_range = AmountRange { min: 1, max: total };
            state.remaining_deposits = total;
            state.lock(amount, 1).expect("valid lock");
            prop_assert_eq!(state.total_liquidity(), Some(total));
            prop_assert_eq!(state.outstanding_intent_amount, amount);
            state.unlock(amount).expect("valid unlock");
            prop_assert_eq!(state.remaining_deposits, total);
            prop_assert_eq!(state.outstanding_intent_amount, 0);
            prop_assert_eq!(state.active_intents, 0);
        }

        #[test]
        fn escrow_settlement_conservation_invariant(
            total in 1_u64..1_000_000_000,
            amount_seed in any::<u64>(),
            release_seed in any::<u64>(),
        ) {
            let amount = bounded_nonzero(amount_seed, total);
            let release = bounded_nonzero(release_seed, amount);
            let mut state = deposit();
            state.intent_amount_range = AmountRange { min: 1, max: total };
            state.remaining_deposits = total;
            state.lock(amount, 1).expect("valid lock");
            state.settle(amount, release).expect("valid settle");
            prop_assert_eq!(state.total_liquidity(), total.checked_sub(release));
            prop_assert_eq!(state.outstanding_intent_amount, 0);
            prop_assert_eq!(state.active_intents, 0);
        }

        #[test]
        fn stake_resolution_liability_invariant(
            balance in 1_u64..1_000_000_000,
            lock_seed in any::<u64>(),
            claim_seed in any::<u64>(),
        ) {
            let lock = bounded_nonzero(lock_seed, balance);
            let claim = claim_seed.checked_rem(lock.saturating_add(1)).unwrap_or(0);
            let mut position = StakePosition {
                vault: Pubkey::new_unique(),
                owner: Pubkey::new_unique(),
                balance,
                locked: 0,
                bump: 1,
            };
            position.lock(lock).expect("valid lock");
            position.resolve(lock, claim).expect("valid resolution");
            prop_assert_eq!(position.locked, 0);
            prop_assert_eq!(position.balance.checked_add(claim), Some(balance));
        }

        #[test]
        fn dispute_window_is_snapshotted_and_exact(
            release in 1_u64..1_000_000_000,
            now in 0_i64..1_000_000_000,
            window in 0_i64..31_536_000,
        ) {
            let mut state = pending_dispute();
            state.locked_amount = release;
            state.risk_window = window;
            let boundary = state.settle(release, now).expect("valid settle");
            prop_assert_eq!(boundary, now.saturating_add(window));
            if window > 0 {
                prop_assert!(state.release(boundary.saturating_sub(1)).is_err());
            }
            prop_assert_eq!(state.release(boundary), Ok(release));
        }
    }
}
