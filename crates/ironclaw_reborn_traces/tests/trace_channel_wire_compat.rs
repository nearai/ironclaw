//! Wire/disk compatibility for the generic `TraceChannel` (OUT-12).
//!
//! Trace envelopes persist as JSON in the contribution spool and on the
//! ingestion wire; a tag that stops deserializing silently quarantines the
//! row. Every historical tag — including the retired concrete channel
//! variants — must keep parsing forever (LLM data is never dropped over a
//! channel name).

use ironclaw_reborn_traces::contribution::{IronclawTraceMetadata, TraceChannel};

#[test]
fn every_historical_channel_tag_still_deserializes() {
    for (tag, expected) in [
        ("web", TraceChannel::Web),
        ("cli", TraceChannel::Cli),
        ("routine", TraceChannel::Routine),
        ("other", TraceChannel::Other),
        ("extension", TraceChannel::Extension),
        // Retired concrete tags land on the generic variant.
        ("telegram", TraceChannel::Extension),
        ("slack", TraceChannel::Extension),
        // Unknown future tags parse instead of quarantining the envelope.
        ("some_future_channel", TraceChannel::Extension),
    ] {
        let parsed: TraceChannel =
            serde_json::from_value(serde_json::Value::String(tag.to_string()))
                .unwrap_or_else(|error| panic!("tag {tag} must parse: {error}"));
        assert_eq!(parsed, expected, "tag {tag}");
    }
}

#[test]
fn legacy_metadata_json_parses_and_the_modern_form_round_trips() {
    // A pre-migration envelope's metadata: concrete tag, no origin field.
    let legacy = r#"{"version":"0.9.0","channel":"slack"}"#;
    let parsed: IronclawTraceMetadata = serde_json::from_str(legacy).expect("legacy parses");
    assert_eq!(parsed.channel, TraceChannel::Extension);
    assert_eq!(parsed.channel_origin, None);

    // The modern form carries the origin as data and round-trips.
    let modern = IronclawTraceMetadata {
        version: "1.0.0".to_string(),
        engine_version: None,
        feature_flags: Default::default(),
        channel: TraceChannel::Extension,
        channel_origin: Some("vendorx".to_string()),
        model_name: None,
    };
    let json = serde_json::to_string(&modern).expect("serializes");
    assert!(json.contains(r#""channel":"extension""#));
    assert!(json.contains(r#""channel_origin":"vendorx""#));
    let back: IronclawTraceMetadata = serde_json::from_str(&json).expect("round-trips");
    assert_eq!(back, modern);
}
