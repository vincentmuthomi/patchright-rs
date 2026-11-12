//! Property-based tests for the pure protocol helpers.
//!
//! These cover the input space *between* the example tests. The fuzz targets
//! in `fuzz/` already assert "never panics" on arbitrary bytes; that is a
//! weak oracle for a pure transform, so nothing here restates it. The
//! property below is held to something real: an inverse function.
//!
//! Kept cheap on purpose — these run in the ordinary suite on every push, so
//! the value is in covering shapes the examples missed, not in a long
//! campaign. Long runs belong in the nightly fuzz job.
//!
//! Properties for `pub(crate)` helpers live beside the code instead (see the
//! `glob` module), since an integration test cannot reach them and widening
//! the public API to be testable would be the wrong trade.

use patchright_rs::protocol::{parse_result, serialize_argument};
use proptest::prelude::*;
use serde_json::{Value, json};

/// The largest integer a JS `number` represents exactly (`2^53 - 1`).
const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// A JSON value the evaluate conversion is expected to round-trip.
///
/// Two deliberate exclusions, each pinned by a test below:
///
/// - **Non-finite floats.** `serde_json` cannot hold them: they become
///   `Null` before the serializer ever sees them, so they are unreachable
///   rather than lossy.
/// - **Integers beyond the JS safe range.** The wire format is a JS
///   `number`, so anything past `2^53 - 1` is rounded.
fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        (-JS_MAX_SAFE_INTEGER..=JS_MAX_SAFE_INTEGER).prop_map(Value::from),
        any::<f64>()
            .prop_filter(
                "non-finite floats cannot exist in a serde_json::Value",
                |f| f.is_finite()
            )
            .prop_map(Value::from),
        "[a-z0-9 ]{0,8}".prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 12, 3, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..3).prop_map(Value::from),
            proptest::collection::hash_map("[a-z]{1,4}", inner, 0..3)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

/// Rewrites every number to its `f64` form, recursively.
///
/// The wire format is JavaScript, where there is one number type. An `i64`
/// therefore comes back as a float with the same value, and comparing the
/// raw `serde_json::Value`s would fail on `0` vs `0.0` while telling us
/// nothing about the conversion. The normal form compares what the protocol
/// can actually carry.
fn js_number_normal_form(value: &Value) -> Value {
    match value {
        Value::Number(n) => n
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Value::Array(items) => Value::Array(items.iter().map(js_number_normal_form).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), js_number_normal_form(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `parse_result` is the left inverse of `serialize_argument`, up to the
    /// JS number normal form. Every `evaluate` call in the crate goes out
    /// through one and comes back through the other, so a value that does
    /// not survive the pair is silently corrupted data handed to the caller.
    #[test]
    fn evaluate_conversion_round_trips(value in arb_json()) {
        let wire = serialize_argument(&value);
        let back = parse_result(&wire["value"]);
        prop_assert_eq!(js_number_normal_form(&back), js_number_normal_form(&value));
    }
}

/// Pins the non-finite-float exclusion. If `serde_json` ever represents
/// these, the exclusion is vestigial and the generator should widen.
#[test]
fn the_non_finite_float_exclusion_is_current() {
    assert_eq!(
        serde_json::to_value(f64::INFINITY).unwrap(),
        Value::Null,
        "non-finite floats still cannot reach the serializer through a Value"
    );
    assert!(serde_json::Number::from_f64(f64::NAN).is_none());
}

/// Pins the safe-integer exclusion, and the rounding that happens past it.
///
/// Playwright's answer for larger integers is the `bi` (BigInt) tag, which
/// `serialize_argument` does not emit — it widens every number to `f64`. If
/// that changes, widen the generator and delete this.
#[test]
fn integers_beyond_the_js_safe_range_are_rounded() {
    let beyond = JS_MAX_SAFE_INTEGER + 2;
    let round_tripped = parse_result(&serialize_argument(&json!(beyond))["value"]);

    assert_eq!(
        round_tripped.as_f64(),
        Some(9_007_199_254_740_992.0),
        "an integer past 2^53-1 is still rounded rather than sent as a BigInt"
    );
}
