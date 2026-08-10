//! Manifest shape pins: the egress declarations are the deployment's push
//! host allowlist (composition reads them back for enrollment validation),
//! so their structure — https-only hosts, the VAPID credential handle, the
//! `vapid_authorization` injection — must not drift.

const MANIFEST: &str = ironclaw_web_push_extension::MANIFEST;

#[test]
fn every_egress_entry_is_a_vapid_injected_https_push_host() {
    let parsed: toml::Value = toml::from_str(MANIFEST).expect("manifest parses as TOML");
    let egress = parsed
        .get("channel")
        .and_then(|channel| channel.get("egress"))
        .and_then(toml::Value::as_array)
        .expect("manifest declares [[channel.egress]]");
    assert!(
        !egress.is_empty(),
        "an empty egress set would leave enrollment with no admissible push service"
    );

    for entry in egress {
        let host = entry
            .get("host")
            .and_then(toml::Value::as_str)
            .expect("every egress entry declares a host");
        assert!(!host.trim().is_empty() && !host.contains('*'));
        assert_eq!(
            entry.get("scheme").and_then(toml::Value::as_str),
            Some("https"),
            "push services are https-only"
        );
        assert_eq!(
            entry.get("credential_handle").and_then(toml::Value::as_str),
            Some(ironclaw_web_push::WEB_PUSH_VAPID_CREDENTIAL_HANDLE),
            "every egress entry injects the VAPID credential"
        );
        let injection = entry
            .get("injection")
            .and_then(|injection| injection.get("type"))
            .and_then(toml::Value::as_str);
        assert_eq!(
            injection,
            Some("vapid_authorization"),
            "every egress entry uses the vapid_authorization injection"
        );
        let prefixes = entry
            .get("path_prefixes")
            .and_then(toml::Value::as_array)
            .expect("endpoint paths are opaque tokens, so the constraint is a prefix");
        assert_eq!(
            prefixes
                .iter()
                .map(|prefix| prefix.as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["/"],
            "the whole-host prefix is the declared constraint"
        );
    }
}

#[test]
fn manifest_identity_matches_the_grammar_constants() {
    let parsed: toml::Value = toml::from_str(MANIFEST).expect("manifest parses as TOML");
    assert_eq!(
        parsed.get("id").and_then(toml::Value::as_str),
        Some(ironclaw_web_push::WEB_PUSH_EXTENSION_ID)
    );
    let channel = parsed.get("channel").expect("channel surface declared");
    assert_eq!(
        channel.get("inbound").and_then(toml::Value::as_bool),
        Some(false),
        "web push is outbound-only"
    );
    assert_eq!(
        channel.get("outbound").and_then(toml::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        channel.get("notifications").and_then(toml::Value::as_bool),
        Some(true),
        "the web app declares the notifications capability"
    );
}
