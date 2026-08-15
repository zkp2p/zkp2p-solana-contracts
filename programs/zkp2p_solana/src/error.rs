//! Canonical protocol errors.

use anchor_lang::prelude::*;

/// Errors intentionally name the exact rejected condition.
#[error_code]
pub enum Zkp2pError {
    /// A required public key was the all-zero key or system default.
    #[msg("required public key is zero")]
    ZeroAddress,
    /// A required numeric value was zero.
    #[msg("required value is zero")]
    ZeroValue,
    /// Caller does not match the required authority.
    #[msg("caller is unauthorized")]
    Unauthorized,
    /// Component is paused at this admission boundary.
    #[msg("component is paused")]
    Paused,
    /// Component is already in the requested state.
    #[msg("component is already in requested state")]
    AlreadyInState,
    /// Checked arithmetic overflowed or underflowed.
    #[msg("arithmetic overflow or underflow")]
    ArithmeticOverflow,
    /// Input array lengths do not match.
    #[msg("array lengths do not match")]
    ArrayLengthMismatch,
    /// The configured range has invalid bounds.
    #[msg("range minimum exceeds maximum")]
    InvalidRange,
    /// Amount is below the configured minimum.
    #[msg("amount is below minimum")]
    AmountBelowMinimum,
    /// Amount is above the configured maximum.
    #[msg("amount is above maximum")]
    AmountAboveMaximum,
    /// Available balance cannot satisfy the requested amount.
    #[msg("insufficient available balance")]
    InsufficientBalance,
    /// Fee exceeds its immutable maximum.
    #[msg("fee exceeds maximum")]
    FeeExceedsMaximum,
    /// Deposit does not exist or supplied account does not describe it.
    #[msg("deposit not found")]
    DepositNotFound,
    /// Deposit is closed to new intents.
    #[msg("deposit is not accepting intents")]
    DepositNotAcceptingIntents,
    /// Intent does not exist.
    #[msg("intent not found")]
    IntentNotFound,
    /// Intent or lock already exists.
    #[msg("intent or lock already exists")]
    IntentAlreadyExists,
    /// Payment method is disabled or absent.
    #[msg("payment method is not supported")]
    PaymentMethodNotSupported,
    /// Currency is disabled or absent.
    #[msg("currency is not supported")]
    CurrencyNotSupported,
    /// Proposed rate is below the effective minimum.
    #[msg("conversion rate is below minimum")]
    RateBelowMinimum,
    /// Oracle quote is invalid, future-dated, or stale.
    #[msg("oracle quote is unusable")]
    InvalidOracleQuote,
    /// Configured spread produces a nonpositive multiplier.
    #[msg("rate spread is invalid")]
    InvalidSpread,
    /// Account already has an active intent and multiple intents are disabled.
    #[msg("account already has an active intent")]
    AccountHasActiveIntent,
    /// Configured active-intent cap would be exceeded.
    #[msg("maximum active intents exceeded")]
    MaximumIntentsExceeded,
    /// Signature has expired.
    #[msg("signature has expired")]
    SignatureExpired,
    /// Signature or attestation did not validate.
    #[msg("signature or attestation is invalid")]
    InvalidSignature,
    /// Signed data hash does not match the supplied payload.
    #[msg("signed data hash mismatch")]
    DataHashMismatch,
    /// Signed intent snapshot differs from canonical intent state.
    #[msg("intent snapshot mismatch")]
    IntentSnapshotMismatch,
    /// Payment proof was already consumed.
    #[msg("payment nullifier already exists")]
    NullifierAlreadyExists,
    /// Intent already has a different payment binding.
    #[msg("intent already has a payment binding")]
    IntentAlreadyBound,
    /// Original payment binding does not match the disputed intent.
    #[msg("original payment is not bound to intent")]
    InvalidPaymentBinding,
    /// Stake owner lacks free collateral.
    #[msg("insufficient free stake")]
    InsufficientFreeStake,
    /// Active stake lock was not found.
    #[msg("stake lock not found")]
    LockNotFound,
    /// Stake lock ID is already active.
    #[msg("stake lock already exists")]
    LockAlreadyExists,
    /// Proposed lock maturity is not strictly in the future.
    #[msg("lock maturity is invalid")]
    InvalidMaturity,
    /// Mature lock may no longer be increased or resized.
    #[msg("stake lock has matured")]
    LockAlreadyMatured,
    /// Claims allocate more principal than the resolved lock.
    #[msg("claims exceed lock amount")]
    ClaimsExceedLock,
    /// Claim has a zero beneficiary or amount.
    #[msg("claim allocation is invalid")]
    InvalidClaim,
    /// Taker did not authorize the selected stake owner.
    #[msg("stake owner is not authorized for taker")]
    UnauthorizedStakeOwner,
    /// Controller handover delay has not elapsed.
    #[msg("controller proposal is not ready")]
    ControllerNotReady,
    /// Dispute protection does not admit this deposit.
    #[msg("dispute protection is disabled")]
    DisputeProtectionDisabled,
    /// New dispute-protection admission is paused.
    #[msg("dispute protection admissions are paused")]
    AdmissionsPaused,
    /// Dispute state is not pending.
    #[msg("dispute protection intent is not pending")]
    DisputeIntentNotPending,
    /// Dispute state is not settled.
    #[msg("dispute protection intent is not settled")]
    DisputeIntentNotSettled,
    /// Risk window has not elapsed.
    #[msg("dispute collateral is not release eligible")]
    NotReleaseEligible,
    /// Escrow token does not match the configured stake token.
    #[msg("intent token does not match stake token")]
    IntentTokenMismatch,
    /// Enabled whitelist does not admit the taker.
    #[msg("taker is not whitelisted")]
    TakerNotWhitelisted,
    /// Whitelist group does not exist.
    #[msg("address group does not exist")]
    GroupNotFound,
    /// Deposit already consumed one-time bootstrap.
    #[msg("deposit is already bootstrapped")]
    DepositAlreadyBootstrapped,
    /// Per-deposit group cap would be exceeded.
    #[msg("too many groups configured for deposit")]
    TooManyGroups,
    /// Exact SPL token balance delta was not observed.
    #[msg("token balance delta is not exact")]
    InvalidTokenBalanceDelta,
    /// Canonical SlotHashes state cannot yield a safe deployment-domain seed.
    #[msg("deployment domain is not authenticated by recent cluster state")]
    InvalidDeploymentDomain,
}
