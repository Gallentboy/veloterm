use rust_decimal::Decimal;
use std::str::FromStr;
use crate::error::{Error, Result};

pub fn is_zero(b: &[u8]) -> bool {
    b.len() == 1 && b[0] == 128
}

pub fn is_positive(b: &[u8]) -> bool {
    !b.is_empty() && (b[0] & 128) != 0
}

/// Convert Oracle internal NUMBER bytes into a standard ASCII numeric string.
pub fn decode_number(b: &[u8]) -> Result<String> {
    if b.is_empty() {
        return Err(Error::Decode("empty Oracle number bytes".into()));
    }
    if is_zero(b) {
        return Ok("0".into());
    }
    if b == [255, 101] {
        return Ok("Infinity".into());
    }
    if b == [0] {
        return Ok("-Infinity".into());
    }

    let positive = is_positive(b);
    let mut data = Vec::with_capacity(b.len());

    if positive {
        data.push((b[0] & 0x7F).wrapping_sub(65) as i8);
        for &byte in &b[1..] {
            data.push(byte.wrapping_sub(1) as i8);
        }
    } else {
        let actual_len = if b.len() == 21 && *b.last().unwrap() != 102 {
            b.len()
        } else {
            b.len().saturating_sub(1)
        };
        data.push((((b[0] ^ 0xFF) & 0x7F).wrapping_sub(65)) as i8);
        for &byte in &b[1..actual_len] {
            data.push((101u8.wrapping_sub(byte)) as i8);
        }
    }

    let mut exponent = data[0] as i32;
    let real_data_len = data.len() - 1;
    let k = exponent - (real_data_len as i32 - 1);

    let mut result = String::with_capacity(64);
    if !positive {
        result.push('-');
    }

    if k >= 0 {
        result.push_str(&format!("{}", data[1]));
        let mut b1 = 2;
        while b1 <= real_data_len {
            result.push_str(&format!("{:02}", data[b1]));
            b1 += 1;
            exponent -= 1;
        }
        while exponent > 0 {
            result.push_str("00");
            exponent -= 1;
        }
    } else {
        let n = real_data_len as i32 + k;
        if n > 0 {
            result.push_str(&format!("{}", data[1]));
            if n == 1 {
                result.push('.');
            }
            let mut b1 = 2;
            while b1 < real_data_len {
                result.push_str(&format!("{:02}", data[b1]));
                if n == b1 as i32 {
                    result.push('.');
                }
                b1 += 1;
            }
            if b1 <= real_data_len {
                if data[b1] % 10 == 0 {
                    result.push_str(&format!("{}", data[b1] / 10));
                } else {
                    result.push_str(&format!("{:02}", data[b1]));
                }
            }
        } else {
            result.push_str("0.");
            let mut padding = n;
            while padding < 0 {
                result.push_str("00");
                padding += 1;
            }
            for val in data.iter().take(real_data_len).skip(1) {
                result.push_str(&format!("{:02}", val));
            }
            if real_data_len >= 1 {
                let last = data[real_data_len];
                if last % 10 == 0 {
                    result.push_str(&format!("{}", last / 10));
                } else {
                    result.push_str(&format!("{:02}", last));
                }
            }
        }
    }

    Ok(result)
}

pub fn decode_number_to_i64(b: &[u8]) -> Result<i64> {
    let s = decode_number(b)?;
    s.parse::<i64>().map_err(|e| Error::Decode(format!("failed to parse i64 from '{s}': {e}")))
}

pub fn decode_number_to_f64(b: &[u8]) -> Result<f64> {
    let s = decode_number(b)?;
    s.parse::<f64>().map_err(|e| Error::Decode(format!("failed to parse f64 from '{s}': {e}")))
}

pub fn decode_number_to_decimal(b: &[u8]) -> Result<Decimal> {
    let s = decode_number(b)?;
    Decimal::from_str(&s).map_err(|e| Error::Decode(format!("failed to parse Decimal from '{s}': {e}")))
}

/// Encode i64 into Oracle internal NUMBER byte representation.
pub fn encode_i64_to_number(val: i64) -> Vec<u8> {
    if val == 0 {
        return vec![128];
    }
    let is_neg = val < 0;
    let mut abs_val = val.unsigned_abs();

    // Break into base-100 digits
    let mut digits = Vec::new();
    while abs_val > 0 {
        digits.push((abs_val % 100) as u8);
        abs_val /= 100;
    }
    digits.reverse();

    let exponent = (digits.len() as i32) - 1;
    let mut out = Vec::with_capacity(digits.len() + 2);

    if !is_neg {
        let exp_byte = (exponent + 193) as u8;
        out.push(exp_byte);
        for d in digits {
            out.push(d + 1);
        }
    } else {
        let exp_byte = (62 - exponent) as u8;
        out.push(exp_byte);
        for d in digits {
            out.push(101 - d);
        }
        if out.len() < 21 {
            out.push(102);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero() {
        assert_eq!(decode_number(&[128]).unwrap(), "0");
        assert_eq!(encode_i64_to_number(0), vec![128]);
    }

    #[test]
    fn test_positive_integers() {
        let b = encode_i64_to_number(1);
        assert_eq!(decode_number(&b).unwrap(), "1");

        let b = encode_i64_to_number(100);
        assert_eq!(decode_number(&b).unwrap(), "100");

        let b = encode_i64_to_number(123456);
        assert_eq!(decode_number(&b).unwrap(), "123456");
    }

    #[test]
    fn test_negative_integers() {
        let b = encode_i64_to_number(-1);
        assert_eq!(decode_number(&b).unwrap(), "-1");

        let b = encode_i64_to_number(-100);
        assert_eq!(decode_number(&b).unwrap(), "-100");

        let b = encode_i64_to_number(-123456);
        assert_eq!(decode_number(&b).unwrap(), "-123456");
    }
}
