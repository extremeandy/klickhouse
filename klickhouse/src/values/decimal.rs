use rust_decimal::Decimal;

use crate::{unexpected_type, FromSql, KlickhouseError, Result, ToSql, Type, Value};

impl FromSql for Decimal {
    fn from_sql(type_: &Type, value: Value) -> Result<Self> {
        fn out_of_range(name: &str) -> KlickhouseError {
            KlickhouseError::DeserializeError(format!("{name} out of bounds for rust_decimal"))
        }

        match value {
            Value::Int8(i) => Ok(Decimal::new(i as i64, 0)),
            Value::Int16(i) => Ok(Decimal::new(i as i64, 0)),
            Value::Int32(i) => Ok(Decimal::new(i as i64, 0)),
            Value::Int64(i) => Ok(Decimal::new(i, 0)),
            Value::Int128(i) => {
                Decimal::try_from_i128_with_scale(i, 0).map_err(|_| out_of_range("i128"))
            }
            Value::UInt8(i) => Ok(Decimal::new(i as i64, 0)),
            Value::UInt16(i) => Ok(Decimal::new(i as i64, 0)),
            Value::UInt32(i) => Ok(Decimal::new(i as i64, 0)),
            Value::UInt64(i) => {
                Decimal::try_from_i128_with_scale(i.into(), 0).map_err(|_| out_of_range("u128"))
            }
            Value::UInt128(i) => Decimal::try_from_i128_with_scale(
                i.try_into().map_err(|_| out_of_range("u128"))?,
                0,
            )
            .map_err(|_| out_of_range("u128")),
            Value::Decimal32(precision, value) => Decimal::try_new(value as i64, precision as u32)
                .map_err(|_| out_of_range("Decimal32")),
            Value::Decimal64(precision, value) => {
                Decimal::try_new(value, precision as u32).map_err(|_| out_of_range("Decimal64"))
            }
            Value::Decimal128(precision, value) => {
                Decimal::try_from_i128_with_scale(value, precision as u32)
                    .map_err(|_| out_of_range("Decimal128"))
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

        fn mantissa_to_scale(mantissa: i128, scale: u32, precision: u32) -> Result<i128> {
            assert!(precision >= scale);
            if precision == scale {
                Ok(mantissa)
            } else {
                mantissa
                    .checked_mul(10i128.pow(precision - scale))
                    .ok_or_else(|| out_of_range("mantissa"))
            }
        }

        let scale = self.scale();
        let mantissa = self.mantissa();
        match type_hint {
            None => Ok(Value::Decimal128(scale as usize, mantissa)),
            Some(Type::Decimal32(precision)) if *precision as u32 >= scale => Ok(Value::Decimal32(
                *precision,
                mantissa_to_scale(mantissa, scale, *precision as u32)?
                    .try_into()
                    .map_err(|_| out_of_range("Decimal32"))?,
            )),
            Some(Type::Decimal64(precision)) if *precision as u32 >= scale => Ok(Value::Decimal64(
                *precision,
                mantissa_to_scale(mantissa, scale, *precision as u32)?
                    .try_into()
                    .map_err(|_| out_of_range("Decimal64"))?,
            )),
            Some(Type::Decimal128(precision)) if *precision as u32 >= scale => {
                Ok(Value::Decimal128(
                    *precision,
                    mantissa_to_scale(mantissa, scale, *precision as u32)?,
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
