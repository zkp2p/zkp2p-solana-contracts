//! UnifiedPaymentVerifierV3, MultiAttestationVerifier, and NullifierRegistryV2 logic.

use anchor_lang::prelude::*;
use solana_keccak_hasher as keccak;
use solana_secp256k1_recover::secp256k1_recover;

use crate::{
    constants::{MAX_TIMESTAMP_BUFFER_MS, PROTOCOL_SEED, VERIFIER_CONFIG_SEED},
    error::Zkp2pError,
    state::{Intent, ProtocolConfig, VerifierConfig},
};

/// Standardized privacy-preserving off-chain payment details.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaymentDetails {
    /// Payment method.
    pub method: [u8; 32],
    /// Hashed payment recipient.
    pub payee_id: [u8; 32],
    /// Off-chain amount in the method's smallest unit.
    pub amount: u128,
    /// Off-chain currency.
    pub currency: [u8; 32],
    /// Payment time in Unix milliseconds.
    pub timestamp_ms: u64,
    /// Hashed provider transaction identifier.
    pub payment_id: [u8; 32],
}

/// Canonical intent values signed into an attestation payload.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntentSnapshot {
    /// Canonical intent hash.
    pub intent_hash: [u8; 32],
    /// Complete locked intent amount.
    pub amount: u64,
    /// Payment method.
    pub payment_method: [u8; 32],
    /// Fiat currency requested by the intent.
    pub fiat_currency: [u8; 32],
    /// Snapshotted payee hash.
    pub payee_details: [u8; 32],
    /// Snapshotted conversion rate.
    pub conversion_rate: u128,
    /// Signal timestamp in Unix seconds.
    pub signal_timestamp: i64,
    /// Allowed timestamp variance in milliseconds.
    pub timestamp_buffer_ms: u64,
}

/// Threshold-signed payment attestation.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct PaymentAttestation {
    /// Canonical intent hash.
    pub intent_hash: [u8; 32],
    /// Requested on-chain release amount, capped at the intent amount.
    pub release_amount: u64,
    /// Keccak commitment to the canonical Borsh payload.
    pub data_hash: [u8; 32],
    /// Ethereum-format compact recoverable signatures: r || s || v.
    pub signatures: Vec<[u8; 65]>,
    /// Standardized payment details.
    pub payment: PaymentDetails,
    /// Canonical intent snapshot.
    pub snapshot: IntentSnapshot,
}

/// Accounts for protocol-governed verifier configuration.
#[derive(Accounts)]
pub struct ConfigureVerifier<'info> {
    /// Protocol governance authority.
    #[account(address = protocol.authority)]
    pub authority: Signer<'info>,
    /// Protocol root.
    #[account(seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Account<'info, ProtocolConfig>,
    /// Unified verifier state.
    #[account(
        mut,
        seeds = [VERIFIER_CONFIG_SEED],
        bump = verifier.bump,
        constraint = verifier.protocol == protocol.key() @ Zkp2pError::Unauthorized
    )]
    pub verifier: Account<'info, VerifierConfig>,
}

/// Adds or removes one enabled payment method.
pub fn handle_set_verifier_payment_method(
    ctx: Context<ConfigureVerifier>,
    payment_method: [u8; 32],
    enabled: bool,
) -> Result<()> {
    require!(payment_method != [0; 32], Zkp2pError::ZeroValue);
    let position = ctx
        .accounts
        .verifier
        .payment_methods
        .iter()
        .position(|candidate| candidate == &payment_method);
    match (enabled, position) {
        (true, None) => {
            require!(
                ctx.accounts.verifier.payment_methods.len() < 64,
                Zkp2pError::AmountAboveMaximum
            );
            ctx.accounts.verifier.payment_methods.push(payment_method);
        }
        (false, Some(index)) => {
            ctx.accounts.verifier.payment_methods.remove(index);
        }
        _ => return err!(Zkp2pError::AlreadyInState),
    }
    Ok(())
}

/// Adds or removes one Ethereum witness while preserving threshold satisfiability.
pub fn handle_set_verifier_witness(
    ctx: Context<ConfigureVerifier>,
    witness: [u8; 20],
    enabled: bool,
) -> Result<()> {
    require!(witness != [0; 20], Zkp2pError::ZeroAddress);
    let position = ctx
        .accounts
        .verifier
        .witnesses
        .iter()
        .position(|candidate| candidate == &witness);
    match (enabled, position) {
        (true, None) => {
            require!(
                ctx.accounts.verifier.witnesses.len() < 16,
                Zkp2pError::AmountAboveMaximum
            );
            ctx.accounts.verifier.witnesses.push(witness);
        }
        (false, Some(index)) => {
            let remaining = ctx
                .accounts
                .verifier
                .witnesses
                .len()
                .checked_sub(1)
                .ok_or(Zkp2pError::ArithmeticOverflow)?;
            require!(
                remaining >= usize::from(ctx.accounts.verifier.required_signatures),
                Zkp2pError::InvalidSignature
            );
            ctx.accounts.verifier.witnesses.remove(index);
        }
        _ => return err!(Zkp2pError::AlreadyInState),
    }
    Ok(())
}

/// Updates the live witness threshold within the current set size.
pub fn handle_set_required_signatures(ctx: Context<ConfigureVerifier>, required: u8) -> Result<()> {
    require!(required > 0, Zkp2pError::ZeroValue);
    require!(
        usize::from(required) <= ctx.accounts.verifier.witnesses.len(),
        Zkp2pError::InvalidSignature
    );
    ctx.accounts.verifier.required_signatures = required;
    Ok(())
}

/// Verifies all payment, snapshot, data-integrity, and witness-threshold conditions.
pub fn verify_payment_attestation(
    verifier_key: Pubkey,
    verifier: &VerifierConfig,
    intent: &Intent,
    attestation: &PaymentAttestation,
) -> Result<[u8; 32]> {
    require!(
        attestation.intent_hash == intent.intent_hash,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(attestation.release_amount > 0, Zkp2pError::ZeroValue);
    require!(
        verifier
            .payment_methods
            .contains(&attestation.payment.method),
        Zkp2pError::PaymentMethodNotSupported
    );
    require!(
        attestation.payment.method == attestation.snapshot.payment_method,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(
        attestation.payment.payment_id != [0; 32],
        Zkp2pError::ZeroValue
    );
    require!(attestation.payment.amount > 0, Zkp2pError::ZeroValue);
    require!(
        attestation.payment.currency != [0; 32],
        Zkp2pError::ZeroValue
    );
    require!(
        attestation.snapshot.timestamp_buffer_ms <= MAX_TIMESTAMP_BUFFER_MS,
        Zkp2pError::AmountAboveMaximum
    );
    validate_snapshot(intent, &attestation.snapshot)?;

    let mut payload = Vec::new();
    attestation
        .payment
        .serialize(&mut payload)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    attestation
        .snapshot
        .serialize(&mut payload)
        .map_err(|_| error!(Zkp2pError::DataHashMismatch))?;
    require!(
        keccak::hash(&payload).to_bytes() == attestation.data_hash,
        Zkp2pError::DataHashMismatch
    );
    let digest = payment_attestation_digest(
        verifier_key,
        attestation.intent_hash,
        attestation.release_amount,
        attestation.data_hash,
    );
    verify_witness_threshold(verifier, digest, &attestation.signatures)?;
    Ok(keccak::hashv(&[&attestation.payment.method, &attestation.payment.payment_id]).to_bytes())
}

fn validate_snapshot(intent: &Intent, snapshot: &IntentSnapshot) -> Result<()> {
    require!(
        snapshot.intent_hash == intent.intent_hash,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(
        snapshot.amount == intent.amount,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(
        snapshot.payment_method == intent.payment_method,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(
        snapshot.fiat_currency == intent.fiat_currency,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(
        snapshot.payee_details == intent.payee_id,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(
        snapshot.conversion_rate == intent.conversion_rate,
        Zkp2pError::IntentSnapshotMismatch
    );
    require!(
        snapshot.signal_timestamp == intent.timestamp,
        Zkp2pError::IntentSnapshotMismatch
    );
    Ok(())
}

pub(crate) fn verify_witness_threshold(
    verifier: &VerifierConfig,
    digest: [u8; 32],
    signatures: &[[u8; 65]],
) -> Result<()> {
    require!(
        signatures.len() >= usize::from(verifier.required_signatures),
        Zkp2pError::InvalidSignature
    );
    require!(
        signatures.len() <= verifier.witnesses.len(),
        Zkp2pError::InvalidSignature
    );
    let mut recovered = Vec::<[u8; 20]>::with_capacity(signatures.len());
    for signature in signatures {
        let signature_bytes: [u8; 64] = signature
            .get(..64)
            .ok_or(Zkp2pError::InvalidSignature)?
            .try_into()
            .map_err(|_| error!(Zkp2pError::InvalidSignature))?;
        let recovery_byte = *signature.get(64).ok_or(Zkp2pError::InvalidSignature)?;
        let recovery_id = match recovery_byte {
            0 | 1 => recovery_byte,
            27 | 28 => recovery_byte
                .checked_sub(27)
                .ok_or(Zkp2pError::InvalidSignature)?,
            _ => return err!(Zkp2pError::InvalidSignature),
        };
        require!(is_low_s(&signature_bytes), Zkp2pError::InvalidSignature);
        let public_key = secp256k1_recover(&digest, recovery_id, &signature_bytes)
            .map_err(|_| error!(Zkp2pError::InvalidSignature))?
            .to_bytes();
        let public_key_hash = keccak::hash(&public_key).to_bytes();
        let address: [u8; 20] = public_key_hash
            .get(12..)
            .ok_or(Zkp2pError::InvalidSignature)?
            .try_into()
            .map_err(|_| error!(Zkp2pError::InvalidSignature))?;
        require!(
            verifier.witnesses.contains(&address),
            Zkp2pError::InvalidSignature
        );
        require!(!recovered.contains(&address), Zkp2pError::InvalidSignature);
        recovered.push(address);
    }
    require!(
        recovered.len() >= usize::from(verifier.required_signatures),
        Zkp2pError::InvalidSignature
    );
    Ok(())
}

fn is_low_s(signature: &[u8; 64]) -> bool {
    const HALF_ORDER: [u8; 32] = [
        0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b,
        0x20, 0xa0,
    ];
    signature
        .get(32..)
        .is_some_and(|s| s <= HALF_ORDER.as_slice())
}

/// Returns the EIP-712 digest for the Solana verifier domain.
pub fn payment_attestation_digest(
    verifier_key: Pubkey,
    intent_hash: [u8; 32],
    release_amount: u64,
    data_hash: [u8; 32],
) -> [u8; 32] {
    let domain_typehash = keccak::hash(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    )
    .to_bytes();
    let name_hash = keccak::hash(b"UnifiedPaymentVerifier").to_bytes();
    let version_hash = keccak::hash(b"1").to_bytes();
    let verifier_hash = keccak::hash(verifier_key.as_ref()).to_bytes();
    let mut address_word = [0_u8; 32];
    if let (Some(destination), Some(source)) = (address_word.get_mut(12..), verifier_hash.get(12..))
    {
        destination.copy_from_slice(source);
    }
    let chain_id = [0_u8; 32];
    let domain_separator = keccak::hashv(&[
        &domain_typehash,
        &name_hash,
        &version_hash,
        &chain_id,
        &address_word,
    ])
    .to_bytes();

    let typehash = keccak::hash(
        b"PaymentAttestation(bytes32 intentHash,uint256 releaseAmount,bytes32 dataHash)",
    )
    .to_bytes();
    let mut release_word = [0_u8; 32];
    if let Some(destination) = release_word.get_mut(24..) {
        destination.copy_from_slice(&release_amount.to_be_bytes());
    }
    let struct_hash =
        keccak::hashv(&[&typehash, &intent_hash, &release_word, &data_hash]).to_bytes();
    keccak::hashv(&[&[0x19, 0x01], &domain_separator, &struct_hash]).to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_high_s_boundary() {
        let mut signature = [0_u8; 64];
        if let Some(s) = signature.get_mut(32..) {
            s.copy_from_slice(&[0xff; 32]);
        }
        assert!(!is_low_s(&signature));
    }

    #[test]
    fn digest_is_domain_bound() {
        let first = payment_attestation_digest(Pubkey::new_unique(), [1; 32], 7, [2; 32]);
        let second = payment_attestation_digest(Pubkey::new_unique(), [1; 32], 7, [2; 32]);
        assert_ne!(first, second);
    }
}
