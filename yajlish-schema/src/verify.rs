//! Kani proofs for the number code (Plan 4, K5), see
//! `plans/04-kani-bolero.md` in the workspace root. Most are over all `i64`
//! and `f64` values: there are no loops, and Kani's floats are bit-precise.
//! Where Kani did not finish (128-bit remainders over all pairs, float
//! division and rounding), the proof is narrowed or is a Bolero test, and
//! says so. The runtime's harnesses are in `tests/test_verify.rs`.

// `cargo kani --tests` also builds this crate without `cfg(test)`, where
// the `#[test]` harnesses are gone.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use crate::{
    ir::Num,
    num::{cmp, is_multiple_of},
};
#[cfg(kani)]
use crate::{FromValue, Value};
#[cfg(kani)]
use std::cmp::Ordering;

/// `i` compared to `f` by mathematical value, written independently of
/// `num::cmp_int_float`: outside i64's range the sign decides; inside it,
/// compare with `f`'s integer part, then with its fraction.
#[cfg(kani)]
fn reference_cmp(i: i64, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    // ±2^63 are exact in f64.
    if f >= 9_223_372_036_854_775_808.0 {
        return Some(Ordering::Less);
    }
    if f < -9_223_372_036_854_775_808.0 {
        return Some(Ordering::Greater);
    }
    let whole = f.trunc();
    #[allow(clippy::cast_possible_truncation)]
    let whole_int = whole as i64; // exact: |whole| < 2^63
    match i.cmp(&whole_int) {
        Ordering::Equal => {
            let frac = f - whole;
            Some(if frac > 0.0 {
                Ordering::Less
            } else if frac < 0.0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            })
        }
        other => Some(other),
    }
}

#[cfg(kani)]
fn any_num() -> Num {
    if kani::any() {
        Num::Int(kani::any())
    } else {
        Num::Float(kani::any())
    }
}

/// Stands in for `std::fmt::format`: `FromValue`'s errors format the
/// float, and formatting floats is far too large for Kani. The proofs only
/// look at `Ok` versus `Err`.
#[cfg(kani)]
#[allow(dead_code)] // Only named by `kani::stub`.
fn stub_format(_args: std::fmt::Arguments<'_>) -> String {
    // Not `String::new()`: Kani 0.68 models one returned from a stub with a
    // dangling pointer and capacity 1, and dropping it fails.
    String::from("e")
}

#[cfg(kani)]
#[kani::proof]
fn cmp_int_float_is_exact() {
    let i: i64 = kani::any();
    let f: f64 = kani::any();
    assert_eq!(cmp(Num::Int(i), Num::Float(f)), reference_cmp(i, f));
}

#[cfg(kani)]
#[kani::proof]
fn cmp_is_antisymmetric() {
    let a = any_num();
    let b = any_num();
    assert_eq!(cmp(a, b), cmp(b, a).map(Ordering::reverse));
}

/// `is_multiple_of` on two integers agrees with `checked_rem` wherever
/// that is defined, and calls `i64::MIN` a multiple of `-1`, where it is
/// not.
#[cfg(kani)]
fn check_int_multiple_of(x: i64, y: i64) {
    let multiple = is_multiple_of(Num::Int(x), Num::Int(y));
    if y == 0 {
        assert!(!multiple);
    } else if let Some(rem) = x.checked_rem(y) {
        assert_eq!(multiple, rem == 0);
    } else {
        // i64::MIN % -1 overflows i64, but it is a multiple.
        assert!(multiple);
    }
}

/// Any `x` by the divisors where overflow or sign could go wrong. A proof
/// over all `i64` pairs did not finish in 10 minutes: CBMC bit-blasts the
/// 128-bit remainder.
#[cfg(kani)]
#[kani::proof]
fn int_multiple_of_edge_divisors() {
    let x: i64 = kani::any();
    let y = [0, 1, -1, 2, -2, i64::MIN, i64::MAX][kani::any_where(|&i: &usize| i < 7)];
    check_int_multiple_of(x, y);
}

/// Every pair within `i16`'s range, so the solver can cope.
#[cfg(kani)]
#[kani::proof]
fn int_multiple_of_small() {
    let x: i16 = kani::any();
    let y: i16 = kani::any();
    check_int_multiple_of(i64::from(x), i64::from(y));
}

/// The float branch: an infinite quotient is not a multiple. A Bolero
/// test, not a Kani proof: Kani did not finish the floating-point division
/// and rounding in 5 minutes. Float arithmetic cannot panic in Rust, so
/// this only gives up the bounded proof of the "infinite" rule.
/// `tests::float_multiple_of_special_values` covers the NaN quotients.
#[test]
fn float_multiple_of_never_panics() {
    bolero::check!()
        .with_type::<(f64, f64, bool, i64)>()
        .cloned()
        .for_each(|(x, y, int_first, i)| {
            #[allow(clippy::cast_precision_loss)]
            let (n, m, xf, yf) = if int_first {
                (Num::Int(i), Num::Float(y), i as f64, y)
            } else {
                (Num::Float(x), Num::Float(y), x, y)
            };
            let multiple = is_multiple_of(n, m);
            if !(xf / yf).is_finite() {
                assert!(!multiple, "{:?} / {:?}", n, m);
            }
        });
}

/// `FromValue for i64` on a float: `Ok(i)` only when `i` is exactly the
/// float, and an error exactly when it is fractional, not finite, or
/// outside `[-2^63, 2^63)`.
/// For finite `x`: for `±inf` and NaN, `fract` produces a NaN, which the
/// code relies on (`NaN != 0.0`) but Kani reports as a failure. Those are
/// `tests::i64_from_non_finite_float`.
#[cfg(kani)]
#[kani::proof]
#[kani::stub(std::fmt::format, stub_format)]
fn i64_from_float_is_exact() {
    let x: f64 = kani::any();
    kani::assume(x.is_finite());
    let integral_in_range = x.is_finite()
        && x.trunc() == x
        && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&x);
    match i64::from_value(Value::Float(x)) {
        #[allow(clippy::cast_precision_loss)]
        Ok(i) => {
            assert!(integral_in_range);
            assert_eq!(i as f64, x);
        }
        Err(_) => {
            assert!(!integral_in_range);
        }
    }
}

/// The same properties on sample values, so that the harness code is
/// compiled and checked by `cargo test` too.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples() {
        let nums = [
            Num::Int(0),
            Num::Int(i64::MIN),
            Num::Int(i64::MAX),
            Num::Int(-1),
            Num::Float(0.5),
            Num::Float(-0.0),
            Num::Float(f64::NAN),
            Num::Float(f64::INFINITY),
            Num::Float(9_223_372_036_854_775_808.0),
        ];
        for a in nums {
            for b in nums {
                assert_eq!(cmp(a, b), cmp(b, a).map(std::cmp::Ordering::reverse));
                let _ = is_multiple_of(a, b);
            }
        }
        assert!(is_multiple_of(Num::Int(i64::MIN), Num::Int(-1)));
    }

    /// The pairs `float_multiple_of_never_panics` leaves out, whose
    /// quotient is NaN: none is a multiple.
    #[test]
    fn float_multiple_of_special_values() {
        let specials = [
            0.0,
            -0.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            1.5,
            -2.0,
        ];
        for x in specials {
            for y in specials {
                if (x / y).is_nan() {
                    assert!(
                        !is_multiple_of(Num::Float(x), Num::Float(y)),
                        "{} / {}",
                        x,
                        y
                    );
                }
            }
            assert!(!is_multiple_of(Num::Float(x), Num::Float(f64::NAN)));
        }
        // An integer by a float goes through the float branch too: 0 / 0.0.
        assert!(!is_multiple_of(Num::Int(0), Num::Float(0.0)));
    }

    /// The floats `i64_from_float_is_exact` leaves out are all errors.
    #[test]
    fn i64_from_non_finite_float() {
        use crate::{FromValue, Value};
        for x in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN, -f64::NAN] {
            assert!(i64::from_value(Value::Float(x)).is_err(), "{}", x);
        }
    }
}
