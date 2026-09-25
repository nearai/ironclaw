use chrono::{
    DateTime, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeDelta, TimeZone, Utc,
};
use chrono_tz::Tz;
use ironclaw_extension_registry::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::capability::{EffectKind, PermissionMode};
use ironclaw_host_api::dispatch::{
    DispatchInputIssue, DispatchInputIssueCode, RuntimeDispatchErrorKind,
};
use rust_decimal::Decimal;
use serde_json::{Number, Value, json};

use crate::FirstPartyCapabilityError;

use super::{first_party_capability_manifest, resource_profile};

pub const TIME_CAPABILITY_ID: &str = "builtin.time";
pub(super) const UNIX_MILLIS_THRESHOLD: i128 = 100_000_000_000;

/// Upper bound, in characters, on a model-authored value echoed back in an
/// input issue's `received` field.
const MAX_RECEIVED_CHARS: usize = 128;

// Issue text crosses the host_api `SafeSummary` boundary, which drops any value
// containing `/`, so these must not name example zones like `America/New_York`.
const TIMESTAMP_EXPECTED: &str =
    "ISO 8601, Unix seconds, or Unix milliseconds; for relative times use operation \"shift\"";
const TIMEZONE_EXPECTED: &str = "IANA timezone name";
const NAIVE_TIMEZONE_EXPECTED: &str =
    "IANA timezone name to interpret a timestamp that has no UTC offset";
const SHIFT_COMPONENTS_EXPECTED: &str = "at least one of seconds, minutes, hours, days, weeks";
const SHIFT_RANGE_EXPECTED: &str = "offset that keeps the shifted timestamp in range";

/// `shift` offset components and their length in seconds. A day is a fixed
/// 24 hours; `shift` does not apply calendar or DST rules.
const SHIFT_COMPONENTS: [(&str, i64); 5] = [
    ("seconds", 1),
    ("minutes", 60),
    ("hours", 3_600),
    ("days", 86_400),
    ("weeks", 604_800),
];

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        TIME_CAPABILITY_ID,
        "Get, parse, format, convert, or diff timestamps",
        vec![EffectKind::DispatchCapability],
        PermissionMode::Allow,
        resource_profile(),
    )
}

pub(super) fn dispatch(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let operation = match input.get("operation") {
        None => "now",
        Some(value) => value.as_str().ok_or_else(|| {
            time_input_error(type_mismatch(
                "operation",
                &received_text(value),
                "one of now, parse, convert, format, diff, shift",
            ))
        })?,
    };
    match operation {
        "now" => time_now(input),
        "parse" => time_parse(input),
        "convert" => time_convert(input),
        "format" => time_format(input),
        "diff" => time_diff(input),
        "shift" => time_shift(input),
        other => Err(time_input_error(invalid_value(
            "operation",
            other,
            "one of now, parse, convert, format, diff, shift",
        ))),
    }
}

fn time_now(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    instant_output(Utc::now(), input)
}

fn time_parse(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let (path, source) = required_input(input)?;
    let dt = parse_timestamp(
        source,
        optional_timezone(input, &["from_timezone", "timezone"])?
            .map(|(tz, _)| tz)
            .as_ref(),
        path,
    )?;
    Ok(json!({
        "iso": dt.to_rfc3339(),
        "unix": dt.timestamp(),
        "unix_millis": dt.timestamp_millis()
    }))
}

fn time_convert(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let (path, source) = required_input(input)?;
    let from_tz = optional_timezone(input, &["from_timezone", "timezone"])?.map(|(tz, _)| tz);
    let dt = parse_timestamp(source, from_tz.as_ref(), path)?;
    let (target_tz, target_name) = required_timezone(input, "to_timezone")?;
    Ok(json!({
        "input": source,
        "utc_iso": dt.to_rfc3339(),
        "output": dt.with_timezone(&target_tz).to_rfc3339(),
        "timezone": target_name
    }))
}

fn time_format(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let (path, source) = required_input(input)?;
    let output_tz = optional_timezone(input, &["timezone"])?;
    let from_tz = optional_timezone(input, &["from_timezone"])?.map(|(tz, _)| tz);
    let fallback_tz = output_tz.as_ref().map(|(tz, _)| *tz);
    let parse_tz = from_tz.as_ref().or(fallback_tz.as_ref());
    let dt = parse_timestamp(source, parse_tz, path)?;
    let fmt = input
        .get("format_string")
        .and_then(Value::as_str)
        .or_else(|| input.get("format").and_then(Value::as_str))
        .unwrap_or("%Y-%m-%d %H:%M:%S %Z");
    let mut output = if let Some((tz, name)) = output_tz {
        json!({
            "formatted": dt.with_timezone(&tz).format(fmt).to_string(),
            "timezone": name
        })
    } else {
        json!({ "formatted": dt.format(fmt).to_string() })
    };
    output["utc_iso"] = Value::String(dt.to_rfc3339());
    Ok(output)
}

fn time_diff(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let (first_path, first) = required_input(input)?;
    let Some(second) = input.get("timestamp2") else {
        return Err(time_input_error(missing_required(
            "timestamp2",
            TIMESTAMP_EXPECTED,
        )));
    };
    let tz = optional_timezone(input, &["from_timezone", "timezone"])?.map(|(tz, _)| tz);
    let dt1 = parse_timestamp(first, tz.as_ref(), first_path)?;
    let dt2 = parse_timestamp(second, tz.as_ref(), "timestamp2")?;
    let diff = dt2.signed_duration_since(dt1);
    Ok(json!({
        "seconds": diff.num_seconds(),
        "minutes": diff.num_minutes(),
        "hours": diff.num_hours(),
        "days": diff.num_days()
    }))
}

fn time_shift(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let components = shift_components(input)?;
    // Report the largest component if the summed offset or resulting date
    // is outside chrono's supported range.
    let Some(dominant) = components
        .iter()
        .max_by_key(|component| component.delta.abs())
    else {
        return Err(time_input_error(missing_required(
            "seconds",
            SHIFT_COMPONENTS_EXPECTED,
        )));
    };
    let base = match optional_input(input) {
        Some((path, source)) => {
            let from_tz =
                optional_timezone(input, &["from_timezone", "timezone"])?.map(|(tz, _)| tz);
            parse_timestamp(source, from_tz.as_ref(), path)?
        }
        None => Utc::now(),
    };
    // Signed components may cancel after a prefix exceeds TimeDelta's range.
    let total_seconds = components
        .iter()
        .map(|component| i128::from(component.delta.num_seconds()))
        .sum::<i128>();
    let offset = i64::try_from(total_seconds)
        .ok()
        .and_then(TimeDelta::try_seconds)
        .ok_or_else(|| dominant.out_of_range())?;
    let shifted = base
        .checked_add_signed(offset)
        .ok_or_else(|| dominant.out_of_range())?;
    instant_output(shifted, input)
}

struct ShiftComponent {
    field: &'static str,
    count: i64,
    delta: TimeDelta,
}

impl ShiftComponent {
    fn out_of_range(&self) -> FirstPartyCapabilityError {
        time_input_error(invalid_value(
            self.field,
            &self.count.to_string(),
            SHIFT_RANGE_EXPECTED,
        ))
    }
}

fn shift_components(input: &Value) -> Result<Vec<ShiftComponent>, FirstPartyCapabilityError> {
    let mut components = Vec::new();
    for (field, unit_seconds) in SHIFT_COMPONENTS {
        let Some(value) = input.get(field) else {
            continue;
        };
        let Some(count) = value.as_i64() else {
            return Err(time_input_error(type_mismatch(
                field,
                &received_text(value),
                "signed integer",
            )));
        };
        let Some(delta) = count
            .checked_mul(unit_seconds)
            .and_then(TimeDelta::try_seconds)
        else {
            return Err(time_input_error(invalid_value(
                field,
                &count.to_string(),
                SHIFT_RANGE_EXPECTED,
            )));
        };
        components.push(ShiftComponent {
            field,
            count,
            delta,
        });
    }
    Ok(components)
}

/// Shared `now`/`shift` output: the instant in UTC, plus `local_iso` when a
/// `timezone` or `utc_offset` is supplied.
fn instant_output(
    instant: DateTime<Utc>,
    input: &Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let mut output = json!({
        "iso": instant.to_rfc3339(),
        "utc_iso": instant.to_rfc3339(),
        "unix": instant.timestamp(),
        "unix_millis": instant.timestamp_millis()
    });
    if let Some((tz, name)) = optional_timezone(input, &["timezone"])? {
        output["local_iso"] = Value::String(instant.with_timezone(&tz).to_rfc3339());
        output["timezone"] = Value::String(name);
    } else if let Some((offset, name)) = optional_utc_offset(input)? {
        output["local_iso"] = Value::String(instant.with_timezone(&offset).to_rfc3339());
        output["utc_offset"] = Value::String(name);
    }
    Ok(output)
}

/// The timestamp operand and the field it was read from, so an issue names the
/// field the model actually sent (`input` or its `timestamp` alias).
fn optional_input(input: &Value) -> Option<(&'static str, &Value)> {
    ["input", "timestamp"]
        .into_iter()
        .find_map(|field| input.get(field).map(|value| (field, value)))
}

fn required_input(input: &Value) -> Result<(&'static str, &Value), FirstPartyCapabilityError> {
    optional_input(input)
        .ok_or_else(|| time_input_error(missing_required("input", TIMESTAMP_EXPECTED)))
}

fn required_timezone(
    input: &Value,
    field: &str,
) -> Result<(Tz, String), FirstPartyCapabilityError> {
    // A declared-optional field set to null is stripped upstream by
    // `normalize_optional_null_sentinels`, so absent and null both arrive here as absent;
    // anything else present is a wrong type, not a missing field.
    let Some(value) = input.get(field) else {
        return Err(time_input_error(missing_required(field, TIMEZONE_EXPECTED)));
    };
    let name = timezone_name(field, value)?;
    Ok((parse_timezone(field, name)?, name.to_string()))
}

fn optional_timezone(
    input: &Value,
    fields: &[&str],
) -> Result<Option<(Tz, String)>, FirstPartyCapabilityError> {
    for field in fields {
        let Some(value) = input.get(*field) else {
            continue;
        };
        let name = timezone_name(field, value)?;
        return Ok(Some((parse_timezone(field, name)?, name.to_string())));
    }
    Ok(None)
}

/// A timezone field has to be a string; a number or bool is a type error, not a
/// missing field and not an unparseable zone.
fn timezone_name<'a>(field: &str, value: &'a Value) -> Result<&'a str, FirstPartyCapabilityError> {
    value.as_str().ok_or_else(|| {
        time_input_error(type_mismatch(
            field,
            &received_text(value),
            TIMEZONE_EXPECTED,
        ))
    })
}

fn parse_timezone(field: &str, name: &str) -> Result<Tz, FirstPartyCapabilityError> {
    name.parse::<Tz>().map_err(|error| {
        tracing::debug!(field, %error, "time capability timezone did not parse");
        time_input_error(invalid_value(field, name, TIMEZONE_EXPECTED))
    })
}

fn optional_utc_offset(
    input: &Value,
) -> Result<Option<(FixedOffset, String)>, FirstPartyCapabilityError> {
    let Some(value) = input.get("utc_offset") else {
        return Ok(None);
    };
    let name = value.as_str().ok_or_else(|| {
        time_input_error(type_mismatch(
            "utc_offset",
            &received_text(value),
            "UTC offset such as +03:00 or -07:00",
        ))
    })?;
    parse_utc_offset(name).map(Some)
}

fn parse_utc_offset(value: &str) -> Result<(FixedOffset, String), FirstPartyCapabilityError> {
    let probe = format!("1970-01-01T00:00:00{value}");
    let offset = DateTime::parse_from_rfc3339(&probe)
        .map_err(|error| {
            tracing::debug!(%error, "time capability utc_offset did not parse");
            time_input_error(invalid_value(
                "utc_offset",
                value,
                "UTC offset such as +03:00 or -07:00",
            ))
        })?
        .offset()
        .fix();
    Ok((offset, value.to_string()))
}

fn parse_timestamp(
    input: &Value,
    timezone: Option<&Tz>,
    path: &str,
) -> Result<DateTime<Utc>, FirstPartyCapabilityError> {
    if let Some(text) = input.as_str() {
        if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
            return Ok(dt.with_timezone(&Utc));
        }
        if let Some(dt) = parse_unix_timestamp(text) {
            return Ok(dt);
        }
        if let Some(naive) = parse_naive_datetime(text) {
            let Some(timezone) = timezone else {
                return Err(time_input_error(missing_required(
                    "timezone",
                    NAIVE_TIMEZONE_EXPECTED,
                )));
            };
            return local_to_utc(naive, *timezone, path, text);
        }
    } else if let Some(number) = input.as_number()
        && let Some(dt) = parse_unix_number(number)
    {
        return Ok(dt);
    }
    let received = received_text(input);
    Err(time_input_error(
        if input.is_string() || input.is_number() {
            invalid_value(path, &received, TIMESTAMP_EXPECTED)
        } else {
            type_mismatch(path, &received, TIMESTAMP_EXPECTED)
        },
    ))
}

fn parse_unix_number(number: &Number) -> Option<DateTime<Utc>> {
    number.as_f64().filter(|value| value.is_finite())?;
    parse_unix_timestamp(&number.to_string())
}

fn parse_unix_timestamp(input: &str) -> Option<DateTime<Utc>> {
    const NANOS_PER_SECOND: i128 = 1_000_000_000;

    let input = input.trim();
    let normalized;
    // Normalize exponent notation so the decimal parser below applies the same
    // seconds, milliseconds, and fractional-precision rules to every numeric form.
    let input = if input.contains(['e', 'E']) {
        normalized = Decimal::from_scientific(input).ok()?.to_string();
        normalized.as_str()
    } else {
        input
    };
    let (negative, unsigned) = match input.as_bytes().first() {
        Some(b'-') => (true, &input[1..]),
        Some(b'+') => (false, &input[1..]),
        Some(_) => (false, input),
        None => return None,
    };
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.is_some_and(|fraction| {
            fraction.is_empty()
                || fraction.len() > 9
                || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return None;
    }

    let whole = whole.parse::<i128>().ok()?;
    let signed_whole = if negative {
        whole.checked_neg()?
    } else {
        whole
    };
    let Some(fraction) = fraction else {
        return parse_integral_unix_timestamp(signed_whole);
    };
    if fraction.bytes().all(|byte| byte == b'0') {
        return parse_integral_unix_timestamp(signed_whole);
    }

    let fractional_nanos = fraction.parse::<i128>().ok()?
        * 10_i128.pow(u32::try_from(9_usize.checked_sub(fraction.len())?).ok()?);
    // Build the unsigned magnitude first, then apply the sign once so fractions
    // such as -0.25 negate both the whole and fractional components.
    let total_nanos = whole
        .checked_mul(NANOS_PER_SECOND)?
        .checked_add(fractional_nanos)?;
    let total_nanos = if negative {
        total_nanos.checked_neg()?
    } else {
        total_nanos
    };
    // Euclidean division keeps nanos in [0, 1e9) for negative timestamps,
    // matching DateTime::from_timestamp's normalized (seconds, nanos) shape.
    let seconds = i64::try_from(total_nanos.div_euclid(NANOS_PER_SECOND)).ok()?;
    let nanos = u32::try_from(total_nanos.rem_euclid(NANOS_PER_SECOND)).ok()?;
    DateTime::from_timestamp(seconds, nanos)
}

fn parse_integral_unix_timestamp(timestamp: i128) -> Option<DateTime<Utc>> {
    let value = i64::try_from(timestamp).ok()?;
    if timestamp.abs() >= UNIX_MILLIS_THRESHOLD {
        DateTime::from_timestamp_millis(value)
    } else {
        DateTime::from_timestamp(value, 0)
    }
}

fn parse_naive_datetime(input: &str) -> Option<NaiveDateTime> {
    const DATETIME_FORMATS: &[&str] = &[
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
    ];

    for format in DATETIME_FORMATS {
        if let Ok(value) = NaiveDateTime::parse_from_str(input, format) {
            return Some(value);
        }
    }

    NaiveDate::parse_from_str(input, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
}

fn local_to_utc(
    naive: NaiveDateTime,
    tz: Tz,
    path: &str,
    source: &str,
) -> Result<DateTime<Utc>, FirstPartyCapabilityError> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Ok(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(_, _) | LocalResult::None => Err(time_input_error(invalid_value(
            path,
            source,
            "local time that occurs exactly once in the timezone; add a UTC offset near DST changes",
        ))),
    }
}

fn time_input_error(issue: DispatchInputIssue) -> FirstPartyCapabilityError {
    tracing::debug!(
        runtime_dispatch_error_kind = %RuntimeDispatchErrorKind::InputEncode,
        issue_path = issue.path.as_str(),
        "time capability input validation failed"
    );
    FirstPartyCapabilityError::invalid_input_issues("time input failed validation", vec![issue])
}

fn missing_required(path: &str, expected: &str) -> DispatchInputIssue {
    DispatchInputIssue::new(path, DispatchInputIssueCode::MissingRequired).expected(expected)
}

/// Time inputs are model-authored timestamps and timezone names, not secrets,
/// so the offending value is echoed back (bounded) to make the issue actionable.
fn invalid_value(path: &str, received: &str, expected: &str) -> DispatchInputIssue {
    DispatchInputIssue::new(path, DispatchInputIssueCode::InvalidValue)
        .expected(expected)
        .received(bounded_received(received))
}

/// The field carries the wrong JSON type, as opposed to a right-typed value that does not
/// parse ([`invalid_value`]) or a field that is not there at all ([`missing_required`]).
fn type_mismatch(path: &str, received: &str, expected: &str) -> DispatchInputIssue {
    DispatchInputIssue::new(path, DispatchInputIssueCode::TypeMismatch)
        .expected(expected)
        .received(bounded_received(received))
}

fn bounded_received(received: &str) -> String {
    received.chars().take(MAX_RECEIVED_CHARS).collect()
}

fn received_text(value: &Value) -> String {
    match value.as_str() {
        Some(text) => text.to_string(),
        None => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Utc};
    use ironclaw_host_api::dispatch::{
        DispatchFailureDetail, DispatchInputIssue, DispatchInputIssueCode, RuntimeDispatchErrorKind,
    };
    use ironclaw_host_api::safe_summary::SafeSummary;
    use serde_json::{Value, json};

    use super::{MAX_RECEIVED_CHARS, dispatch};
    use crate::FirstPartyCapabilityError;

    const VALID: &str = "2026-08-04T21:06:40Z";

    fn single_issue(input: Value, case_name: &str) -> DispatchInputIssue {
        let error = dispatch(&input).expect_err(case_name);
        let FirstPartyCapabilityError::Dispatch {
            kind,
            safe_summary,
            detail: Some(detail),
            ..
        } = error
        else {
            panic!("{case_name}: expected structured invalid-input error, got {error:?}");
        };
        // InputEncode maps to the "correct arguments before retry" recovery
        // hint; a semantic parse failure must not be downgraded.
        assert_eq!(kind, RuntimeDispatchErrorKind::InputEncode, "{case_name}");
        assert_eq!(
            safe_summary.as_deref(),
            Some("time input failed validation"),
            "{case_name}"
        );
        let DispatchFailureDetail::InvalidInput { mut issues } = *detail else {
            panic!("{case_name}: expected invalid-input detail");
        };
        assert_eq!(issues.len(), 1, "{case_name}: {issues:?}");
        issues.remove(0)
    }

    #[test]
    fn input_failures_carry_the_offending_path_and_code() {
        use DispatchInputIssueCode::{InvalidValue, MissingRequired, TypeMismatch};

        let cases: Vec<(&str, Value, &str, DispatchInputIssueCode, Option<&str>)> = vec![
            (
                "numeric operation",
                json!({"operation": 5}),
                "operation",
                TypeMismatch,
                Some("5"),
            ),
            (
                "numeric utc_offset",
                json!({"utc_offset": 5}),
                "utc_offset",
                TypeMismatch,
                Some("5"),
            ),
            (
                "unknown operation",
                json!({"operation": "rewind"}),
                "operation",
                InvalidValue,
                Some("rewind"),
            ),
            (
                "missing input",
                json!({"operation": "parse"}),
                "input",
                MissingRequired,
                None,
            ),
            (
                "convert without to_timezone",
                json!({"operation": "convert", "input": VALID}),
                "to_timezone",
                MissingRequired,
                None,
            ),
            (
                "diff without timestamp2",
                json!({"operation": "diff", "input": VALID}),
                "timestamp2",
                MissingRequired,
                None,
            ),
            (
                "unparseable timezone",
                json!({"operation": "parse", "input": VALID, "timezone": "Not/AZone"}),
                "timezone",
                InvalidValue,
                Some("Not/AZone"),
            ),
            (
                "unparseable from_timezone",
                json!({
                    "operation": "convert",
                    "input": VALID,
                    "from_timezone": "Mars",
                    "to_timezone": "UTC"
                }),
                "from_timezone",
                InvalidValue,
                Some("Mars"),
            ),
            (
                "unparseable to_timezone",
                json!({"operation": "convert", "input": VALID, "to_timezone": "Nowhere"}),
                "to_timezone",
                InvalidValue,
                Some("Nowhere"),
            ),
            (
                "unparseable utc_offset",
                json!({"operation": "now", "utc_offset": "not-an-offset"}),
                "utc_offset",
                InvalidValue,
                Some("not-an-offset"),
            ),
            (
                "natural-language input from the production trace",
                json!({
                    "operation": "parse",
                    "input": "24 hours ago",
                    "timezone": "America/Los_Angeles"
                }),
                "input",
                InvalidValue,
                Some("24 hours ago"),
            ),
            (
                "unparseable timestamp alias",
                json!({"operation": "parse", "timestamp": "soon"}),
                "timestamp",
                InvalidValue,
                Some("soon"),
            ),
            (
                "unparseable second diff operand",
                json!({"operation": "diff", "input": VALID, "timestamp2": "later"}),
                "timestamp2",
                InvalidValue,
                Some("later"),
            ),
            (
                "naive datetime without a timezone",
                json!({"operation": "parse", "input": "2026-08-04 21:06"}),
                "timezone",
                MissingRequired,
                None,
            ),
            (
                "local time inside a DST gap",
                json!({
                    "operation": "parse",
                    "input": "2026-03-08 02:30",
                    "timezone": "America/Los_Angeles"
                }),
                "input",
                InvalidValue,
                Some("2026-03-08 02:30"),
            ),
            (
                "shift without components",
                json!({"operation": "shift", "input": VALID}),
                "seconds",
                MissingRequired,
                None,
            ),
            (
                "shift component overflows the offset",
                json!({"operation": "shift", "days": i64::MAX}),
                "days",
                InvalidValue,
                Some("9223372036854775807"),
            ),
            (
                "positive summed shift overflows the offset",
                json!({
                    "operation": "shift",
                    "seconds": 9_223_372_036_854_775_i64,
                    "minutes": 153_722_867_280_912_i64
                }),
                "seconds",
                InvalidValue,
                Some("9223372036854775"),
            ),
            (
                "negative summed shift overflows the offset",
                json!({
                    "operation": "shift",
                    "seconds": -9_223_372_036_854_775_i64,
                    "minutes": -153_722_867_280_912_i64
                }),
                "seconds",
                InvalidValue,
                Some("-9223372036854775"),
            ),
            (
                "shift lands outside the supported date range",
                json!({"operation": "shift", "seconds": 1, "weeks": 100_000_000}),
                "weeks",
                InvalidValue,
                Some("100000000"),
            ),
            (
                "non-integer shift component",
                json!({"operation": "shift", "hours": 1.5}),
                "hours",
                TypeMismatch,
                Some("1.5"),
            ),
            (
                // A present-but-wrong-typed field is a type error; `MissingRequired` is
                // reserved for a field that is absent (or null, stripped upstream).
                "numeric to_timezone",
                json!({"operation": "convert", "input": VALID, "to_timezone": 5}),
                "to_timezone",
                TypeMismatch,
                Some("5"),
            ),
            (
                "boolean timezone",
                json!({"operation": "parse", "input": VALID, "timezone": true}),
                "timezone",
                TypeMismatch,
                Some("true"),
            ),
            (
                "timestamp operand of the wrong type",
                json!({"operation": "parse", "input": ["2026-08-04T21:06:40Z"]}),
                "input",
                TypeMismatch,
                Some("[\"2026-08-04T21:06:40Z\"]"),
            ),
        ];

        for (case_name, input, path, code, received) in cases {
            let issue = single_issue(input, case_name);
            assert_eq!(issue.path, path, "{case_name}");
            assert_eq!(issue.code, code, "{case_name}");
            assert_eq!(issue.received.as_deref(), received, "{case_name}");
            // `expected` crosses the host_api SafeSummary boundary on its way
            // to the model; text that fails it is silently dropped there.
            let expected = issue
                .expected
                .unwrap_or_else(|| panic!("{case_name}: expected text missing"));
            assert!(
                SafeSummary::new(expected.clone()).is_ok(),
                "{case_name}: expected text {expected:?} would be dropped at the model boundary"
            );
        }
    }

    #[test]
    fn unparseable_input_points_the_model_at_shift() {
        let issue = single_issue(
            json!({"operation": "parse", "input": "24 hours ago"}),
            "relative input",
        );
        assert!(
            issue
                .expected
                .as_deref()
                .is_some_and(|expected| expected.contains("operation \"shift\"")),
            "got {issue:?}"
        );
    }

    #[test]
    fn received_values_are_bounded_on_a_char_boundary() {
        let oversized = "\u{e9}".repeat(MAX_RECEIVED_CHARS * 2);
        let issue = single_issue(
            json!({"operation": "parse", "input": oversized}),
            "oversized input",
        );
        let received = issue.received.expect("received value");
        assert_eq!(received.chars().count(), MAX_RECEIVED_CHARS);
        assert!(oversized.starts_with(&received));
    }

    #[test]
    fn shift_offsets_an_explicit_input() {
        let cases = [
            (json!({"days": -14}), "2026-07-21T21:06:40+00:00"),
            (
                json!({"days": -1, "hours": -12}),
                "2026-08-03T09:06:40+00:00",
            ),
            (
                json!({"hours": 5, "minutes": 30}),
                "2026-08-05T02:36:40+00:00",
            ),
            (
                json!({"weeks": 1, "seconds": -40}),
                "2026-08-11T21:06:00+00:00",
            ),
        ];
        for (components, expected) in cases {
            let mut input = json!({"operation": "shift", "input": VALID});
            for (field, value) in components.as_object().expect("components object") {
                input[field] = value.clone();
            }
            let output = dispatch(&input).expect("shift succeeds");
            assert_eq!(output["iso"], json!(expected), "{components}");
            assert_eq!(output["utc_iso"], json!(expected), "{components}");
        }
    }

    #[test]
    fn shift_defaults_to_now_when_input_is_omitted() {
        let before = Utc::now() - TimeDelta::hours(24);
        let output = dispatch(&json!({"operation": "shift", "hours": -24})).expect("shift");
        let after = Utc::now() - TimeDelta::hours(24);
        let shifted = output["unix_millis"].as_i64().expect("unix_millis");
        assert!(
            (before.timestamp_millis()..=after.timestamp_millis()).contains(&shifted),
            "{shifted} not within [{before}, {after}]"
        );
    }

    #[test]
    fn shift_reports_local_time_like_now() {
        let output = dispatch(&json!({
            "operation": "shift",
            "input": VALID,
            "hours": 1,
            "timezone": "America/Los_Angeles"
        }))
        .expect("shift");
        assert_eq!(output["local_iso"], json!("2026-08-04T15:06:40-07:00"));
        assert_eq!(output["timezone"], json!("America/Los_Angeles"));
    }

    #[test]
    fn shift_interprets_a_naive_input_in_the_supplied_timezone() {
        let output = dispatch(&json!({
            "operation": "shift",
            "input": "2026-08-04 12:00",
            "timezone": "Asia/Tokyo",
            "days": 1
        }))
        .expect("shift");
        assert_eq!(output["utc_iso"], json!("2026-08-05T03:00:00+00:00"));
        assert_eq!(output["local_iso"], json!("2026-08-05T12:00:00+09:00"));
    }
}
