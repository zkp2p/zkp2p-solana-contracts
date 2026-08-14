//! Protocol constants shared across instruction modules.

/// Fixed-point precision used by fees and conversion rates.
pub const PRECISE_UNIT: u128 = 1_000_000_000_000_000_000;
/// Basis-point denominator.
pub const BPS: i128 = 10_000;
/// Maximum protocol or delegated manager fee: 5%.
pub const MAX_FEE: u128 = 50_000_000_000_000_000;
/// Maximum aggregate referral fee: 50%.
pub const MAX_REFERRAL_FEE: u128 = 500_000_000_000_000_000;
/// Maximum lifetime of an escrow intent lock: five days.
pub const MAX_INTENT_LIFETIME_SECONDS: i64 = 5 * 24 * 60 * 60;
/// Minimum controller handover delay: one day.
pub const MIN_CONTROLLER_CHANGE_DELAY_SECONDS: i64 = 24 * 60 * 60;
/// Maximum dispute risk window: 365 days.
pub const MAX_RISK_WINDOW_SECONDS: i64 = 365 * 24 * 60 * 60;
/// Maximum number of address groups configured for one deposit.
pub const MAX_GROUPS_PER_DEPOSIT: usize = 10;
/// Maximum signed payment timestamp buffer: 48 hours in milliseconds.
pub const MAX_TIMESTAMP_BUFFER_MS: u64 = 48 * 60 * 60 * 1_000;
/// Sentinel maturity for pending dispute collateral.
pub const NEVER_MATURES: i64 = i64::MAX;

/// Root protocol configuration PDA seed.
pub const PROTOCOL_SEED: &[u8] = b"protocol";
/// Escrow component configuration PDA seed.
pub const ESCROW_CONFIG_SEED: &[u8] = b"escrow-config";
/// Orchestrator component configuration PDA seed.
pub const ORCHESTRATOR_CONFIG_SEED: &[u8] = b"orchestrator-config";
/// Stake-vault component configuration PDA seed.
pub const STAKE_VAULT_CONFIG_SEED: &[u8] = b"stake-vault-config";
/// Unified-verifier component configuration PDA seed.
pub const VERIFIER_CONFIG_SEED: &[u8] = b"verifier-config";
/// Whitelist component configuration PDA seed.
pub const WHITELIST_CONFIG_SEED: &[u8] = b"whitelist-config";
/// Dispute component configuration PDA seed.
pub const DISPUTE_CONFIG_SEED: &[u8] = b"dispute-config";
/// Rate-manager component configuration PDA seed.
pub const RATE_MANAGER_CONFIG_SEED: &[u8] = b"rate-manager-config";
/// Per-owner aggregate stake position seed.
pub const STAKE_POSITION_SEED: &[u8] = b"stake-position";
/// Stake-vault SPL token account seed.
pub const STAKE_TOKEN_VAULT_SEED: &[u8] = b"stake-token-vault";
/// Owner-to-taker authorization seed.
pub const TAKER_AUTHORIZATION_SEED: &[u8] = b"taker-authorization";
/// Taker-selected owner seed.
pub const STAKE_SELECTION_SEED: &[u8] = b"stake-selection";
/// Beneficiary claim balance seed.
pub const CLAIM_BALANCE_SEED: &[u8] = b"claim-balance";
/// Delegated rate-manager account seed.
pub const RATE_MANAGER_SEED: &[u8] = b"rate-manager";
/// Per-manager payment/currency rate seed.
pub const RATE_ENTRY_SEED: &[u8] = b"rate-entry";
/// Escrow deposit account seed.
pub const DEPOSIT_SEED: &[u8] = b"deposit";
/// Per-deposit token custody account seed.
pub const DEPOSIT_VAULT_SEED: &[u8] = b"deposit-vault";
/// Deposit payment-method account seed.
pub const PAYMENT_METHOD_SEED: &[u8] = b"payment-method";
/// Deposit payment/currency account seed.
pub const DEPOSIT_CURRENCY_SEED: &[u8] = b"deposit-currency";
/// Trusted oracle quote account seed.
pub const ORACLE_QUOTE_SEED: &[u8] = b"oracle-quote";
/// Escrow lock account seed.
pub const ESCROW_INTENT_LOCK_SEED: &[u8] = b"escrow-intent-lock";
/// Canonical orchestrator intent account seed.
pub const INTENT_SEED: &[u8] = b"intent";
/// Per-taker active intent counter seed.
pub const TAKER_INTENT_STATE_SEED: &[u8] = b"taker-intent-state";
/// Payment nullifier binding seed.
pub const PAYMENT_BINDING_SEED: &[u8] = b"payment-binding";
/// Reverse intent-to-payment binding seed.
pub const INTENT_PAYMENT_BINDING_SEED: &[u8] = b"intent-payment-binding";
/// Address group seed.
pub const ADDRESS_GROUP_SEED: &[u8] = b"address-group";
/// Address group member seed.
pub const GROUP_MEMBER_SEED: &[u8] = b"group-member";
/// Per-deposit whitelist policy seed.
pub const DEPOSIT_WHITELIST_SEED: &[u8] = b"deposit-whitelist";
/// Direct per-deposit whitelist member seed.
pub const DEPOSIT_WHITELIST_MEMBER_SEED: &[u8] = b"deposit-whitelist-member";
/// Stake controller lock seed.
pub const STAKE_LOCK_SEED: &[u8] = b"stake-lock";
/// Per-method dispute risk window seed.
pub const RISK_WINDOW_SEED: &[u8] = b"risk-window";
/// Per-deposit dispute setting seed.
pub const DEPOSIT_DISPUTE_SETTING_SEED: &[u8] = b"deposit-dispute-setting";
/// Per-intent dispute state seed.
pub const DISPUTE_INTENT_SEED: &[u8] = b"dispute-intent";
/// Dispute replay nullifier seed.
pub const DISPUTE_NULLIFIER_SEED: &[u8] = b"dispute-nullifier";
