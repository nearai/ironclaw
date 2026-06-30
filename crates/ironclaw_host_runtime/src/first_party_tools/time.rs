use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode};
use serde_json::{Value, json};

use crate::FirstPartyCapabilityError;

use super::{first_party_capability_manifest, input_error, resource_profile};

pub const TIME_CAPABILITY_ID: &str = "builtin.time";

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
    let operation = input
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or("now");
    match operation {
        "now" => time_now(input),
        "parse" => time_parse(input),
        "convert" => time_convert(input),
        "format" => time_format(input),
        "diff" => time_diff(input),
        _ => Err(input_error()),
    }
}

fn time_now(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let now = Utc::now();
    let mut output = json!({
        "iso": now.to_rfc3339(),
        "utc_iso": now.to_rfc3339(),
        "unix": now.timestamp(),
        "unix_millis": now.timestamp_millis()
    });
    if let Some((tz, name)) = optional_timezone(input, &["timezone"])? {
        output["local_iso"] = Value::String(now.with_timezone(&tz).to_rfc3339());
        output["timezone"] = Value::String(name);
    }
    Ok(output)
}

fn time_parse(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let source = required_input(input)?;
    let dt = parse_timestamp(
        source,
        optional_timezone(input, &["from_timezone", "timezone"])?
            .map(|(tz, _)| tz)
            .as_ref(),
    )?;
    Ok(json!({
        "iso": dt.to_rfc3339(),
        "unix": dt.timestamp(),
        "unix_millis": dt.timestamp_millis()
    }))
}

fn time_convert(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let source = required_input(input)?;
    let from_tz = optional_timezone(input, &["from_timezone", "timezone"])?.map(|(tz, _)| tz);
    let dt = parse_timestamp(source, from_tz.as_ref())?;
    let (target_tz, target_name) = required_timezone(input, "to_timezone")?;
    Ok(json!({
        "input": source,
        "utc_iso": dt.to_rfc3339(),
        "output": dt.with_timezone(&target_tz).to_rfc3339(),
        "timezone": target_name
    }))
}

fn time_format(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    let source = required_input(input)?;
    let output_tz = optional_timezone(input, &["timezone"])?;
    let from_tz = optional_timezone(input, &["from_timezone"])?.map(|(tz, _)| tz);
    let fallback_tz = output_tz.as_ref().map(|(tz, _)| *tz);
    let parse_tz = from_tz.as_ref().or(fallback_tz.as_ref());
    let dt = parse_timestamp(source, parse_tz)?;
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
    let first = required_input(input)?;
    let second = input
        .get("timestamp2")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    let tz = optional_timezone(input, &["from_timezone", "timezone"])?.map(|(tz, _)| tz);
    let dt1 = parse_timestamp(first, tz.as_ref())?;
    let dt2 = parse_timestamp(second, tz.as_ref())?;
    let diff = dt2.signed_duration_since(dt1);
    Ok(json!({
        "seconds": diff.num_seconds(),
        "minutes": diff.num_minutes(),
        "hours": diff.num_hours(),
        "days": diff.num_days()
    }))
}

fn required_input(input: &Value) -> Result<&str, FirstPartyCapabilityError> {
    input
        .get("input")
        .or_else(|| input.get("timestamp"))
        .and_then(Value::as_str)
        .ok_or_else(input_error)
}

fn required_timezone(
    input: &Value,
    field: &str,
) -> Result<(Tz, String), FirstPartyCapabilityError> {
    let name = input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    let tz = name.parse::<Tz>().map_err(|_| input_error())?;
    Ok((tz, name.to_string()))
}

fn optional_timezone(
    input: &Value,
    fields: &[&str],
) -> Result<Option<(Tz, String)>, FirstPartyCapabilityError> {
    for field in fields {
        if let Some(name) = input.get(*field).and_then(Value::as_str) {
            let tz = name.parse::<Tz>().map_err(|_| input_error())?;
            return Ok(Some((tz, name.to_string())));
        }
    }
    Ok(None)
}

fn parse_timestamp(
    input: &str,
    timezone: Option<&Tz>,
) -> Result<DateTime<Utc>, FirstPartyCapabilityError> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(input) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Some(naive) = parse_naive_datetime(input) {
        let Some(timezone) = timezone else {
            return Err(input_error());
        };
        return local_to_utc(naive, *timezone);
    }
    Err(input_error())
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

fn local_to_utc(naive: NaiveDateTime, tz: Tz) -> Result<DateTime<Utc>, FirstPartyCapabilityError> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => Ok(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(_, _) | LocalResult::None => Err(input_error()),
    }
}
