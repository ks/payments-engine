use rust_decimal::Decimal;
use serde::Deserialize;

use std::{fmt, ops, str::FromStr};

/// A money value with at most 4 decimal places.
///
/// Values parsed from input are validated: positive, at most 4 decimal places,
/// at most [`MAX_AMOUNT`]. Results of arithmetic (account balances) are also
/// `Amount`s and may be negative or above `MAX_AMOUNT`.
#[derive(Debug, Default, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct Amount(Decimal);

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(d)?; // csv never guesses a type
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Largest amount a single transaction may carry (10^15).
///
/// Transaction ids are `u32` and a repeated id is ignored, so at most 2^32
/// transactions ever change a balance, and no balance or held sum can exceed
/// 2^32 × 10^15 ≈ 4.3e24 in magnitude. That is far below `Decimal::MAX`
/// (≈ 7.9e28), so account arithmetic cannot overflow, and every balance printed
/// with 4 decimal places fits rust_decimal's 32-character formatting buffer
/// (25 digits + sign + point + 4 decimals = 31).
///
/// The bound relies on ids never being reused. Should that ever break (for
/// example if old transactions were pruned), `Decimal` arithmetic panics on
/// overflow rather than wrapping, so a balance is never silently wrong.
pub const MAX_AMOUNT: u64 = 1_000_000_000_000_000;

impl FromStr for Amount {
    type Err = InvalidAmount;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            return Err(InvalidAmount::Format);
        }
        let mut decimal = Decimal::from_str(s).map_err(|_| InvalidAmount::Format)?;
        decimal.normalize_assign();
        if decimal.scale() > 4 {
            return Err(InvalidAmount::Precision);
        }
        if decimal.is_zero() {
            return Err(InvalidAmount::Zero);
        }
        if decimal > Decimal::from(MAX_AMOUNT) {
            return Err(InvalidAmount::TooLarge);
        }
        Ok(Self(decimal))
    }
}

// Plain (unchecked) arithmetic: overflow is unreachable, see `MAX_AMOUNT`.

impl ops::Add for Amount {
    type Output = Amount;

    fn add(self, rhs: Amount) -> Amount {
        Amount(self.0 + rhs.0)
    }
}

impl ops::Sub for Amount {
    type Output = Amount;

    fn sub(self, rhs: Amount) -> Amount {
        Amount(self.0 - rhs.0)
    }
}

impl ops::AddAssign for Amount {
    fn add_assign(&mut self, rhs: Amount) {
        self.0 += rhs.0;
    }
}

impl ops::SubAssign for Amount {
    fn sub_assign(&mut self, rhs: Amount) {
        self.0 -= rhs.0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidAmount {
    /// Not a plain unsigned decimal (sign, exponent, separators, garbage).
    Format,
    /// More than 4 decimal places.
    Precision,
    /// Exactly zero: moves no money, so it is not a valid transaction.
    Zero,
    /// Above [`MAX_AMOUNT`].
    TooLarge,
}

impl fmt::Display for InvalidAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format => write!(f, "amount must be a plain positive decimal number"),
            Self::Precision => write!(f, "amount cannot have more than 4 decimal places"),
            Self::Zero => write!(f, "amount must be greater than zero"),
            Self::TooLarge => write!(f, "amount cannot exceed {MAX_AMOUNT}"),
        }
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.4}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Amount, InvalidAmount> {
        s.parse()
    }

    fn amount(s: &str) -> Amount {
        Amount(Decimal::from_str(s).unwrap())
    }

    #[test]
    fn accepts_plain_decimals_up_to_four_places() {
        assert_eq!(parse("1"), Ok(amount("1")));
        assert_eq!(parse("1.5"), Ok(amount("1.5")));
        assert_eq!(parse("0.0001"), Ok(amount("0.0001")));
        assert_eq!(parse(".5"), Ok(amount("0.5")));
        assert_eq!(parse("5."), Ok(amount("5")));
    }

    #[test]
    fn trailing_zeros_do_not_count_as_precision() {
        assert_eq!(parse("1.500000"), Ok(amount("1.5")));
    }

    #[test]
    fn rejects_more_than_four_places() {
        assert_eq!(parse("0.00001"), Err(InvalidAmount::Precision));
        assert_eq!(parse("1.23456"), Err(InvalidAmount::Precision));
    }

    #[test]
    fn rejects_zero() {
        assert_eq!(parse("0"), Err(InvalidAmount::Zero));
        assert_eq!(parse("0.0000"), Err(InvalidAmount::Zero));
    }

    #[test]
    fn rejects_anything_but_digits_and_a_point() {
        for s in [
            "-1", "+1", "1e3", "1E-2", "1_000", "1,5", "0x10", "ten", " 1", "",
        ] {
            assert_eq!(parse(s), Err(InvalidAmount::Format), "{s:?}");
        }
        assert_eq!(parse("1.2.3"), Err(InvalidAmount::Format));
    }

    #[test]
    fn maximum_is_inclusive() {
        assert_eq!(parse("1000000000000000"), Ok(amount("1000000000000000")));
        assert_eq!(parse("1000000000000000.0001"), Err(InvalidAmount::TooLarge));
    }

    #[test]
    fn displays_four_decimal_places() {
        assert_eq!(amount("1.5").to_string(), "1.5000");
        assert_eq!(amount("0").to_string(), "0.0000");
        assert_eq!(amount("-8").to_string(), "-8.0000");
    }
}
