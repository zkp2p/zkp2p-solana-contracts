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
/// Maximum read-only accounts exposed to one external whitelist resolver.
pub const MAX_RESOLVER_ACCOUNTS: usize = 16;
/// Maximum ECDSA witnesses that fit the canonical transaction budget with account compression.
pub const MAX_WITNESSES: usize = 2;
/// Maximum signed payment timestamp buffer: 48 hours in milliseconds.
pub const MAX_TIMESTAMP_BUFFER_MS: u64 = 48 * 60 * 60 * 1_000;
/// Sentinel maturity for pending dispute collateral.
pub const NEVER_MATURES: i64 = i64::MAX;

/// Keccak-256 of the canonical EIP-712 domain schema.
pub const EIP712_DOMAIN_TYPEHASH: [u8; 32] = [
    0x8b, 0x73, 0xc3, 0xc6, 0x9b, 0xb8, 0xfe, 0x3d, 0x51, 0x2e, 0xcc, 0x4c, 0xf7, 0x59, 0xcc, 0x79,
    0x23, 0x9f, 0x7b, 0x17, 0x9b, 0x0f, 0xfa, 0xca, 0xa9, 0xa7, 0x5d, 0x52, 0x2b, 0x39, 0x40, 0x0f,
];
/// Keccak-256 of the EIP-712 version string `1`.
pub const EIP712_VERSION_ONE_HASH: [u8; 32] = [
    0xc8, 0x9e, 0xfd, 0xaa, 0x54, 0xc0, 0xf2, 0x0c, 0x7a, 0xdf, 0x61, 0x28, 0x82, 0xdf, 0x09, 0x50,
    0xf5, 0xa9, 0x51, 0x63, 0x7e, 0x03, 0x07, 0xcd, 0xcb, 0x4c, 0x67, 0x2f, 0x29, 0x8b, 0x8b, 0xc6,
];
/// Keccak-256 of the unified verifier EIP-712 name.
pub const PAYMENT_VERIFIER_NAME_HASH: [u8; 32] = [
    0x48, 0x80, 0x1c, 0xbb, 0xd5, 0x3d, 0x2e, 0x05, 0x48, 0x50, 0xb7, 0xbc, 0x93, 0x5e, 0xcc, 0xe3,
    0x62, 0x86, 0xf2, 0x1c, 0x08, 0x53, 0x67, 0xc2, 0xfb, 0x80, 0x3f, 0x75, 0x81, 0x41, 0xdd, 0x3a,
];
/// Keccak-256 of the payment-attestation EIP-712 schema.
pub const PAYMENT_ATTESTATION_TYPEHASH: [u8; 32] = [
    0x3f, 0xbf, 0x9d, 0xf1, 0x8b, 0x1c, 0x2c, 0xa7, 0xc4, 0x8d, 0x80, 0x9a, 0x8d, 0x8c, 0x6b, 0xbf,
    0x8d, 0x7a, 0xc3, 0x3f, 0x37, 0x41, 0xab, 0x78, 0xda, 0xc0, 0x72, 0xab, 0xa0, 0xf7, 0x71, 0x03,
];
/// Keccak-256 of the dispute verifier EIP-712 name.
pub const DISPUTE_VERIFIER_NAME_HASH: [u8; 32] = [
    0x6d, 0x80, 0xce, 0xfa, 0x38, 0x78, 0x70, 0x94, 0x4c, 0x7c, 0x59, 0xee, 0xce, 0x6e, 0x3f, 0x92,
    0x09, 0x0d, 0x5e, 0x4d, 0xd1, 0xf8, 0xbd, 0xd2, 0x40, 0xff, 0xff, 0xe8, 0x4a, 0xf3, 0x8a, 0xb3,
];
/// Keccak-256 of the dispute-attestation EIP-712 schema.
pub const DISPUTE_ATTESTATION_TYPEHASH: [u8; 32] = [
    0xa8, 0x00, 0x01, 0xb1, 0x3b, 0xc4, 0x51, 0x24, 0x0e, 0xb6, 0x0a, 0x9c, 0x81, 0xc7, 0x1d, 0x67,
    0xe7, 0xcf, 0x79, 0x94, 0xaa, 0x0a, 0xb6, 0x48, 0x92, 0xd9, 0x51, 0x2a, 0xef, 0xb1, 0x89, 0xfa,
];

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
