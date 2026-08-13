use rust_decimal::Decimal;

use crate::{unexpected_type, FromSql, KlickhouseError, Result, ToSql, Type, Value};

impl FromSql for Decimal {
    fn from_sql(type_: &Type, value: Value) -> Result<Self> {
        match value {
            Value::Int8(i) => Ok(Decimal::new(i as i64, 0)),
            Value::Int16(i) => Ok(Decimal::new(i as i64, 0)),
            Value::Int32(i) => Ok(Decimal::new(i as i64, 0)),
            Value::Int64(i) => Ok(Decimal::new(i, 0)),
            Value::Int128(i) => {
                Decimal::try_from_i128_with_scale(i, 0).map_err(|_| out_of_range_error("i128"))
            }
            Value::UInt8(i) => Ok(Decimal::new(i as i64, 0)),
            Value::UInt16(i) => Ok(Decimal::new(i as i64, 0)),
            Value::UInt32(i) => Ok(Decimal::new(i as i64, 0)),
            Value::UInt64(i) => Decimal::try_from_i128_with_scale(i.into(), 0)
                .map_err(|_| out_of_range_error("u128")),
            Value::UInt128(i) => Decimal::try_from_i128_with_scale(
                i.try_into().map_err(|_| out_of_range_error("u128"))?,
                0,
            )
            .map_err(|_| out_of_range_error("u128")),
            // ClickHouse fixed-point decimals: wire mantissa is at the column scale.
            Value::Decimal32(column_scale, value) => {
                from_scaled(value, column_scale as u32, "Decimal32")
            }
            Value::Decimal64(column_scale, value) => {
                from_scaled(value, column_scale as u32, "Decimal64")
            }
            Value::Decimal128(column_scale, value) => {
                from_scaled(value, column_scale as u32, "Decimal128")
            }
            _ => Err(unexpected_type(type_)),
        }
    }
}

impl ToSql for Decimal {
    fn to_sql(self, type_hint: Option<&Type>) -> Result<Value> {
        fn out_of_range(name: &str) -> KlickhouseError {
            KlickhouseError::SerializeError(format!("{name} out of bounds for rust_decimal"))
        }

        // ClickHouse stores decimals as a fixed-scale integer: the wire mantissa is always at
        // the column's scale. If our rust_decimal has a lower scale, multiply up on write.
        // (See `from_scaled` for the inverse on read.)
        fn mantissa_to_column_scale(mantissa: i128, scale: u32, column_scale: u32) -> Result<i128> {
            assert!(column_scale >= scale);
            if column_scale == scale {
                Ok(mantissa)
            } else {
                mantissa
                    .checked_mul(10i128.pow(column_scale - scale))
                    .ok_or_else(|| out_of_range("mantissa"))
            }
        }

        let scale = self.scale();
        let mantissa = self.mantissa();
        match type_hint {
            None => Ok(Value::Decimal128(scale as usize, mantissa)),
            Some(Type::Decimal32(column_scale)) if *column_scale as u32 >= scale => {
                Ok(Value::Decimal32(
                    *column_scale,
                    mantissa_to_column_scale(mantissa, scale, *column_scale as u32)?
                        .try_into()
                        .map_err(|_| out_of_range("Decimal32"))?,
                ))
            }
            Some(Type::Decimal64(column_scale)) if *column_scale as u32 >= scale => {
                Ok(Value::Decimal64(
                    *column_scale,
                    mantissa_to_column_scale(mantissa, scale, *column_scale as u32)?
                        .try_into()
                        .map_err(|_| out_of_range("Decimal64"))?,
                ))
            }
            Some(Type::Decimal128(column_scale)) if *column_scale as u32 >= scale => {
                Ok(Value::Decimal128(
                    *column_scale,
                    mantissa_to_column_scale(mantissa, scale, *column_scale as u32)?,
                ))
            }
            Some(x) => Err(KlickhouseError::SerializeError(format!(
                "unexpected type: {x}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FromSql, ToSql, Type};

    /// `Decimal(38, 18)` is represented as `Type::Decimal128(18)`.
    const DECIMAL_38_18: Type = Type::Decimal128(18);
    const COLUMN_SCALE: u32 = 18;
    const MAX_I128_REPR: i128 = 0x0000_0000_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF;

    fn decimal(mantissa: i128, scale: u32) -> Decimal {
        Decimal::try_from_i128_with_scale(mantissa, scale).expect("valid test decimal")
    }

    fn max_mantissa_at_scale(scale: u32) -> i128 {
        MAX_I128_REPR / 10i128.pow(COLUMN_SCALE - scale)
    }

    fn roundtrip(value: Decimal) -> Decimal {
        let stored = value
            .to_sql(Some(&DECIMAL_38_18))
            .expect("serializing should succeed");
        Decimal::from_sql(&DECIMAL_38_18, stored).expect("deserializing should succeed")
    }

    fn assert_roundtrips(value: Decimal) {
        assert_eq!(value, roundtrip(value), "value = {value}");
    }

    #[test]
    fn decimal128_scale_18_roundtrip() {
        assert_roundtrips(Decimal::ZERO);
        assert_roundtrips(decimal(0, COLUMN_SCALE));
        assert_roundtrips(Decimal::new(12345, 2));

        assert_roundtrips(Decimal::from(1));
        assert_roundtrips(Decimal::from(10_000_000_000i64));
        assert_roundtrips(Decimal::from(79_228_162_515i64));
        assert_roundtrips(Decimal::from(100_000_000_000i64));

        assert_roundtrips(Decimal::from(-10_000_000_000i64));
        assert_roundtrips(Decimal::from(-100_000_000_000i64));
        assert_roundtrips(decimal(-7_922_816_251_426, 2));

        for scale in [0, 1, 2, 9, 17] {
            let max_mantissa = max_mantissa_at_scale(scale);
            assert_roundtrips(decimal(max_mantissa, scale));
            assert_roundtrips(decimal(max_mantissa + 1, scale));
            assert_roundtrips(decimal(-max_mantissa, scale));
            assert_roundtrips(decimal(-max_mantissa - 1, scale));
        }

        assert_roundtrips(decimal(12_345, COLUMN_SCALE));
        assert_roundtrips(decimal(9_999_999_999, COLUMN_SCALE));
        assert_roundtrips(decimal(MAX_I128_REPR, COLUMN_SCALE));
        assert_roundtrips(decimal(-MAX_I128_REPR, COLUMN_SCALE));
    }

    #[test]
    fn decimal128_scale_18_serialization_errors() {
        let err = decimal(1, COLUMN_SCALE + 1)
            .to_sql(Some(&DECIMAL_38_18))
            .expect_err("scale 19 should not serialize to Decimal128(18)");
        assert!(matches!(
            err,
            KlickhouseError::SerializeError(message) if message.contains("unexpected type")
        ));

        let err = Decimal::MAX
            .to_sql(Some(&DECIMAL_38_18))
            .expect_err("Decimal::MAX should not serialize to Decimal128(18)");
        assert!(matches!(
            err,
            KlickhouseError::SerializeError(message) if message.contains("mantissa")
        ));

        let err = decimal(10i128.pow(21), 0)
            .to_sql(Some(&DECIMAL_38_18))
            .expect_err("mantissa scaling should overflow i128");
        assert!(matches!(
            err,
            KlickhouseError::SerializeError(message) if message.contains("mantissa")
        ));
    }
}

fn out_of_range_error(name: &str) -> KlickhouseError {
    KlickhouseError::DeserializeError(format!("{name} out of bounds for rust_decimal"))
}

fn trailing_pow10(n: i128) -> u32 {
    if n == 0 {
        return 0;
    }
    let n = n.unsigned_abs();
    // A trailing decimal zero needs one factor of 2 and one factor of 5 (10 = 2 × 5).
    n.trailing_zeros().min(trailing_fives(n))
}

fn trailing_fives(mut n: u128) -> u32 {
    // No CPU instruction for factors of 5; bounded to ~55 iterations for u128.
    let mut count = 0;
    while n % 5 == 0 {
        n /= 5;
        count += 1;
    }
    count
}

/// Decode a ClickHouse decimal wire value into a `Decimal`.
///
/// The wire carries `(mantissa, column_scale)` where the numeric value is
/// `mantissa / 10^column_scale`. On write we scale the rust_decimal mantissa *up*
/// to `column_scale`; on read we do not multiply — we strip trailing factors of 10
/// from the wire mantissa and lower the scale by the same amount before constructing
/// the `Decimal`. This keeps the value exact and avoids rejecting wire mantissas
/// that exceed rust_decimal's 96-bit limit at the column scale but normalize to a
/// smaller mantissa (e.g. `79_228_162_515 × 10^18` → `79_228_162_515` at scale 0).
fn from_scaled<I>(mantissa: I, column_scale: u32, name: &str) -> Result<Decimal>
where
    I: Into<i128>,
{
    let mantissa = mantissa.into();
    if let Ok(value) = Decimal::try_from_i128_with_scale(mantissa, column_scale) {
        return Ok(value);
    }

    // Backpack fix for EXP-4436:
    // https://linear.app/backpack-company/issue/EXP-4436/error-getting-latest-risk-metrics
    // This should likely be merged upstream to the main klickhouse repo.
    //
    // Fallback when the wire mantissa exceeds rust_decimal's 96-bit limit at
    // column_scale: strip trailing ×10 and lower the scale before constructing
    // the Decimal (see doc comment above).
    // Cap at column_scale so we never produce a negative scale.
    let reduce = trailing_pow10(mantissa).min(column_scale);
    let mantissa = mantissa / 10i128.pow(reduce);
    let scale = column_scale - reduce;

    Decimal::try_from_i128_with_scale(mantissa, scale).map_err(|_| out_of_range_error(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FromSql, ToSql, Type, Value};

    const MAX_I128_REPR: i128 = 0x0000_0000_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF;

    fn decimal(mantissa: i128, scale: u32) -> Decimal {
        Decimal::try_from_i128_with_scale(mantissa, scale).expect("valid test decimal")
    }

    fn max_mantissa_at_scale(column_scale: u32, scale: u32) -> i128 {
        MAX_I128_REPR / 10i128.pow(column_scale - scale)
    }

    fn roundtrip(value: Decimal, column_type: &Type) -> Decimal {
        let stored = value
            .to_sql(Some(column_type))
            .expect("serializing should succeed");
        Decimal::from_sql(column_type, stored).expect("deserializing should succeed")
    }

    fn assert_roundtrips(value: Decimal, column_type: &Type) {
        assert_eq!(value, roundtrip(value, column_type), "value = {value}");
    }

    #[test]
    fn decimal32_scale_5_roundtrip() {
        let column_type = Type::Decimal32(5);

        assert_roundtrips(Decimal::ZERO, &column_type);
        assert_roundtrips(Decimal::new(12345, 2), &column_type);
        assert_roundtrips(Decimal::from(21_474), &column_type);
        assert_roundtrips(Decimal::from(-21_474), &column_type);
    }

    #[test]
    fn decimal32_from_scaled_wire() {
        let column_type = Type::Decimal32(5);
        let parsed = Decimal::from_sql(&column_type, Value::Decimal32(5, 12_345_000)).unwrap();
        assert_eq!(Decimal::new(12345, 2), parsed);
    }

    #[test]
    fn decimal64_scale_5_roundtrip() {
        let column_type = Type::Decimal64(5);
        let column_scale = 5;
        let max_mantissa_scale_0 = i64::MAX as i128 / 10i128.pow(column_scale);

        assert_roundtrips(Decimal::ZERO, &column_type);
        assert_roundtrips(Decimal::new(12345, 2), &column_type);
        assert_roundtrips(Decimal::from(10_000_000_000i64), &column_type);
        assert_roundtrips(decimal(max_mantissa_scale_0, 0), &column_type);
        assert_roundtrips(decimal(-max_mantissa_scale_0, 0), &column_type);
    }

    #[test]
    fn decimal64_from_scaled_wire() {
        let column_type = Type::Decimal64(5);
        let parsed = Decimal::from_sql(&column_type, Value::Decimal64(5, 12_345_000)).unwrap();
        assert_eq!(Decimal::new(12345, 2), parsed);
    }

    #[test]
    fn decimal128_scale_18_roundtrip() {
        let column_type = Type::Decimal128(18);
        let column_scale = 18;

        assert_roundtrips(Decimal::ZERO, &column_type);
        assert_roundtrips(decimal(0, column_scale), &column_type);
        assert_roundtrips(Decimal::new(12345, 2), &column_type);

        assert_roundtrips(Decimal::from(1), &column_type);
        assert_roundtrips(Decimal::from(10_000_000_000i64), &column_type);
        assert_roundtrips(Decimal::from(79_228_162_515i64), &column_type);
        assert_roundtrips(Decimal::from(100_000_000_000i64), &column_type);

        assert_roundtrips(Decimal::from(-10_000_000_000i64), &column_type);
        assert_roundtrips(Decimal::from(-100_000_000_000i64), &column_type);
        assert_roundtrips(decimal(-7_922_816_251_426, 2), &column_type);

        for scale in [0, 1, 2, 9, 17] {
            let max_mantissa = max_mantissa_at_scale(column_scale, scale);
            assert_roundtrips(decimal(max_mantissa, scale), &column_type);
            assert_roundtrips(decimal(max_mantissa + 1, scale), &column_type);
            assert_roundtrips(decimal(-max_mantissa, scale), &column_type);
            assert_roundtrips(decimal(-max_mantissa - 1, scale), &column_type);
        }

        assert_roundtrips(decimal(12_345, column_scale), &column_type);
        assert_roundtrips(decimal(9_999_999_999, column_scale), &column_type);
        assert_roundtrips(decimal(MAX_I128_REPR, column_scale), &column_type);
        assert_roundtrips(decimal(-MAX_I128_REPR, column_scale), &column_type);
    }

    #[test]
    fn decimal128_scale_18_serialization_errors() {
        let column_type = Type::Decimal128(18);
        let column_scale = 18;

        let err = decimal(1, column_scale + 1)
            .to_sql(Some(&column_type))
            .expect_err("scale 19 should not serialize to Decimal128(18)");
        assert!(matches!(
            err,
            KlickhouseError::SerializeError(message) if message.contains("unexpected type")
        ));

        let err = Decimal::MAX
            .to_sql(Some(&column_type))
            .expect_err("Decimal::MAX should not serialize to Decimal128(18)");
        assert!(matches!(
            err,
            KlickhouseError::SerializeError(message) if message.contains("mantissa")
        ));

        let err = decimal(10i128.pow(21), 0)
            .to_sql(Some(&column_type))
            .expect_err("mantissa scaling should overflow i128");
        assert!(matches!(
            err,
            KlickhouseError::SerializeError(message) if message.contains("mantissa")
        ));
    }
}
