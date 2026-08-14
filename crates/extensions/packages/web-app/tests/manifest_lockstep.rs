//! Manifest shape pins: the egress declarations are the deployment's push
//! host allowlist (composition reads them back for enrollment validation),
//! so their structure — https-only hosts, the VAPID credential handle, the
//! `vapid_authorization` injection — must not drift.

const MANIFEST: &str = ironclaw_web_app_extension::MANIFEST;

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
            Some(ironclaw_web_app::WEB_APP_VAPID_CREDENTIAL_HANDLE),
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
        Some(ironclaw_web_app::WEB_APP_EXTENSION_ID)
    );
    let channel = parsed.get("channel").expect("channel surface declared");
    assert!(
        channel.get("inbound").is_none(),
        "presence of [channel.ingress], not a retired boolean, declares inbound"
    );
    let ingress = channel
        .get("ingress")
        .expect("inbound requires a declared entrypoint");
    assert!(
        ingress.get("route_suffix").is_none(),
        "a session channel must never mount a webhook route"
    );
    assert_eq!(
        ingress
            .get("verification")
            .and_then(|verification| verification.get("kind"))
            .and_then(toml::Value::as_str),
        Some("authenticated_session"),
        "the web app's entrypoint is the authenticated session"
    );
    assert!(channel.get("outbound").is_none());
    assert!(channel.get("notifications").is_none());
    let reply = channel.get("reply").expect("stream reply is declared");
    assert_eq!(
        reply.get("transport").and_then(toml::Value::as_str),
        Some("stream")
    );
    let delivery = channel.get("delivery").expect("push delivery is declared");
    assert_eq!(
        delivery.get("transport").and_then(toml::Value::as_str),
        Some("push")
    );
    assert_eq!(
        delivery
            .get("requires_enrollment")
            .and_then(toml::Value::as_bool),
        Some(true)
    );
}
