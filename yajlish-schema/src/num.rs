//! Exact comparison of integer and floating point numbers.

use crate::ir::Num;
use std::cmp::Ordering;

/// Compare by mathematical value. `None` only when a NaN is involved.
pub(crate) fn cmp(a: Num, b: Num) -> Option<Ordering> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Some(x.cmp(&y)),
        (Num::Float(x), Num::Float(y)) => x.partial_cmp(&y),
        (Num::Int(i), Num::Float(f)) => cmp_int_float(i, f),
        (Num::Float(f), Num::Int(i)) => cmp_int_float(i, f).map(Ordering::reverse),
    }
}

/// `i` compared to `f`, exactly. Rounding an integer to f64 is monotonic, so
/// a difference after rounding is the right answer; only a tie needs a
/// closer look, and a tie means `f` is integral and within i64's range
/// (or exactly 2^63, which `as i128` keeps).
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn cmp_int_float(i: i64, f: f64) -> Option<Ordering> {
    match (i as f64).partial_cmp(&f)? {
        Ordering::Equal => Some(i128::from(i).cmp(&(f as i128))),
        other => Some(other),
    }
}

pub(crate) fn show(n: Num) -> String {
    match n {
        Num::Int(i) => i.to_string(),
        Num::Float(f) => f.to_string(),
    }
}

/// `multipleOf`. Exact for integers. For floats the quotient must be within
/// a relative 1e-9 of an integer, so that `0.0075` is a multiple of `0.0001`
/// although neither is exact in binary; an infinite quotient is not a
/// multiple.
#[allow(clippy::cast_precision_loss)]
pub(crate) fn is_multiple_of(n: Num, m: Num) -> bool {
    match (n, m) {
        (Num::Int(_), Num::Int(0)) => false,
        (Num::Int(x), Num::Int(y)) => i128::from(x) % i128::from(y) == 0,
        _ => {
            let x = match n {
                Num::Int(i) => i as f64,
                Num::Float(f) => f,
            };
            let y = match m {
                Num::Int(i) => i as f64,
                Num::Float(f) => f,
            };
            let q = x / y;
            if !q.is_finite() {
                return false;
            }
            (q - q.round()).abs() <= 1e-9 * q.abs().max(1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmp() {
        assert_eq!(cmp(Num::Int(1), Num::Float(1.0)), Some(Ordering::Equal));
        assert_eq!(cmp(Num::Int(1), Num::Float(1.5)), Some(Ordering::Less));
        assert_eq!(cmp(Num::Float(2.5), Num::Int(2)), Some(Ordering::Greater));
        // 2^53 + 1 rounds to 2^53 as f64, but is still greater.
        let big = (1i64 << 53) + 1;
        assert_eq!(
            cmp(Num::Int(big), Num::Float(9_007_199_254_740_992.0)),
            Some(Ordering::Greater)
        );
        assert_eq!(
            cmp(Num::Int(i64::MAX), Num::Float(9_223_372_036_854_775_808.0)),
            Some(Ordering::Less)
        );
        assert_eq!(cmp(Num::Float(f64::NAN), Num::Int(0)), None);
    }

    #[test]
    fn test_multiple_of() {
        assert!(is_multiple_of(Num::Int(10), Num::Int(5)));
        assert!(!is_multiple_of(Num::Int(7), Num::Int(2)));
        assert!(is_multiple_of(Num::Float(0.0075), Num::Float(0.0001)));
        assert!(is_multiple_of(Num::Float(4.5), Num::Float(1.5)));
        assert!(!is_multiple_of(Num::Float(35.0), Num::Float(1.5)));
        assert!(!is_multiple_of(Num::Float(1e308), Num::Float(0.123456789)));
        assert!(is_multiple_of(Num::Int(4), Num::Float(2.0)));
    }
}
