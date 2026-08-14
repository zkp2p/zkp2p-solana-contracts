//! Checked fixed-point helpers shared by escrow and fee accounting.

use anchor_lang::prelude::*;

use crate::{constants::BPS, error::Zkp2pError};

/// Calculates `amount * rate / 1e18`, rounding down exactly like Solidity integer division.
pub fn precise_mul_floor(amount: u64, rate: u128) -> Result<u64> {
    let product = u128::from(amount)
        .checked_mul(rate)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    let quotient = product
        .checked_div(crate::constants::PRECISE_UNIT)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    u64::try_from(quotient).map_err(|_| error!(Zkp2pError::ArithmeticOverflow))
}

/// Applies a signed basis-point spread and rounds up, matching `Math.mulDiv(..., Rounding.Up)`.
pub fn spread_rate_ceil(market_rate: u128, spread_bps: i16) -> Result<u128> {
    let multiplier = BPS
        .checked_add(i128::from(spread_bps))
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    require!(multiplier > 0, Zkp2pError::InvalidSpread);

    let positive_multiplier =
        u128::try_from(multiplier).map_err(|_| error!(Zkp2pError::InvalidSpread))?;
    let product = market_rate
        .checked_mul(positive_multiplier)
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    let denominator = u128::try_from(BPS).map_err(|_| error!(Zkp2pError::ArithmeticOverflow))?;
    let rounded = product
        .checked_add(
            denominator
                .checked_sub(1)
                .ok_or(Zkp2pError::ArithmeticOverflow)?,
        )
        .ok_or(Zkp2pError::ArithmeticOverflow)?;
    rounded
        .checked_div(denominator)
        .ok_or_else(|| error!(Zkp2pError::ArithmeticOverflow))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn precise_fee_rounds_down() {
        assert_eq!(
            precise_mul_floor(101, 10_000_000_000_000_000).expect("fee"),
            1
        );
    }

    #[test]
    fn spread_rounds_up() {
        assert_eq!(spread_rate_ceil(101, 1).expect("spread"), 102);
        assert_eq!(spread_rate_ceil(10_000, -5_000).expect("spread"), 5_000);
    }

    #[test]
    fn nonpositive_spread_multiplier_rejects() {
        assert!(spread_rate_ceil(1, -10_000).is_err());
    }

    #[test]
    fn arithmetic_overflow_paths_reject() {
        assert!(precise_mul_floor(u64::MAX, u128::MAX).is_err());
        assert!(
            precise_mul_floor(u64::MAX, crate::constants::PRECISE_UNIT.saturating_mul(2)).is_err()
        );
        assert!(spread_rate_ceil(u128::MAX, 1).is_err());
    }

    proptest! {
        #[test]
        fn fee_math_matches_integer_floor(
            amount in any::<u64>(),
            rate in 0_u128..=crate::constants::PRECISE_UNIT,
        ) {
            let expected = u128::from(amount)
                .saturating_mul(rate)
                .checked_div(crate::constants::PRECISE_UNIT)
                .and_then(|value| u64::try_from(value).ok());
            prop_assert_eq!(precise_mul_floor(amount, rate).ok(), expected);
        }

        #[test]
        fn spread_rounding_is_minimal_ceiling(
            market_rate in 1_u128..1_000_000_000_000_000_000,
            spread in -9_999_i16..=10_000_i16,
        ) {
            let result = spread_rate_ceil(market_rate, spread).expect("valid spread");
            let multiplier = u128::try_from(10_000_i128.saturating_add(i128::from(spread)))
                .expect("positive");
            let product = market_rate.saturating_mul(multiplier);
            prop_assert!(result.saturating_mul(10_000) >= product);
            if result > 0 {
                prop_assert!(result.saturating_sub(1).saturating_mul(10_000) < product);
            }
        }
    }
}
