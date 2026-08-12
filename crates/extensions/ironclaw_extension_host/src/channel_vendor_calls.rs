//! Generic execution of the manifest's declarative vendor calls
//! (`ChannelVendorCallRecipe`) — what replaced `ChannelAdapter::activate` and
//! `ChannelAdapter::cleanup`.
//!
//! Only one channel ever implemented those two methods, and what it did was
//! `setWebhook` / `deleteWebhook`: telling the vendor where to POST. Every
//! input was already host-known, because the host owns the webhook route and
//! therefore the URL. So they became `[channel.ingress.registration]` and
//! `[channel.ingress.deregistration]`, and this module runs them.
//!
//! **Three rules keep the substitution honest.**
//!
//! 1. Placeholders resolve from the installation's **non-secret** config
//!    only. A `{handle}` this module cannot resolve is left in place
//!    verbatim — that is not a bug, it is how a credential-in-path vendor
//!    works: the manifest's `injection = { type = "path_placeholder" }` makes
//!    the egress layer substitute the secret host-side, so secret bytes never
//!    pass through here.
//! 2. Secrets never enter a rendered body. A recipe names `body_credentials`
//!    handles; restricted egress resolves each and inserts its VALUE at the
//!    JSON pointer its `[[channel.egress]] body_credentials` entry declares.
//!    This module forwards the handle and nothing else.
//! 3. The call rides the same restricted egress an adapter send does, built
//!    from the same declared `[[channel.egress]]` targets. There is no second
//!    egress path and no unpinned host.

use ironclaw_extension_contracts::channel::{ChannelVendorCallMethod, ChannelVendorCallRecipe};
use ironclaw_extension_contracts::channel_adapter::ChannelError;
use ironclaw_extension_contracts::tool_adapter::{RestrictedEgress, RestrictedEgressRequest};
use ironclaw_host_api::action::NetworkMethod;

/// Substitute `{handle}` occurrences from non-secret config.
///
/// Unresolved placeholders survive verbatim (rule 1 above). Substitution is
/// single-pass over the config entries so a config *value* containing
/// `{other_handle}` can never be re-expanded into a second lookup.
fn substitute(template: &str, config: &[(String, String)]) -> String {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close_offset) = rest[open..].find('}') else {
            break;
        };
        let close = open + close_offset;
        let handle = &rest[open + 1..close];
        rendered.push_str(&rest[..open]);
        match config
            .iter()
            .find(|(candidate, _)| candidate == handle)
            .map(|(_, value)| value.as_str())
        {
            Some(value) => rendered.push_str(value),
            // Left in place for the egress layer's credential injection.
            None => rendered.push_str(&rest[open..=close]),
        }
        rest = &rest[close + 1..];
    }
    rendered.push_str(rest);
    rendered
}

/// Substitute placeholders inside a JSON body template's **string values**.
/// Keys are never templated: a manifest that could rename a vendor field by
/// config would let operator input reshape the request.
fn substitute_json(value: &serde_json::Value, config: &[(String, String)]) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => serde_json::Value::String(substitute(text, config)),
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| substitute_json(item, config))
                .collect(),
        ),
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), substitute_json(field, config)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Render one recipe into an egress request against the channel's declared
/// host.
///
/// `host` is the uniquely matching declared `[[channel.egress]]` host selected
/// by [`resolve_vendor_call_target`]. Egress policy re-checks it, so this is
/// composition of a URL, not a grant.
pub fn render_vendor_call(
    recipe: &ChannelVendorCallRecipe,
    host: &str,
    credential_handle: Option<&ironclaw_host_api::ids::SecretHandle>,
    config: &[(String, String)],
) -> Result<RestrictedEgressRequest, ChannelError> {
    let path = substitute(&recipe.path, config);
    if !path.starts_with('/') || !crate::egress::channel_path_shape_is_safe(&path) {
        return Err(ChannelError::VendorWiring {
            reason: "vendor call path must be rooted and contain only declared path structure"
                .to_string(),
        });
    }
    let body = match &recipe.body {
        Some(template) => {
            let rendered = substitute_json(template, config);
            Some(
                serde_json::to_vec(&rendered).map_err(|error| ChannelError::VendorWiring {
                    reason: format!("vendor call body did not serialize: {error}"),
                })?,
            )
        }
        None => None,
    };
    Ok(RestrictedEgressRequest {
        method: match recipe.method {
            ChannelVendorCallMethod::Post => NetworkMethod::Post,
            ChannelVendorCallMethod::Get => NetworkMethod::Get,
        },
        url: format!("https://{host}{path}"),
        headers: if body.is_some() {
            vec![("content-type".to_string(), "application/json".to_string())]
        } else {
            Vec::new()
        },
        body,
        // The lifecycle selected this manifest-declared egress target. Carry
        // its handle as the host-owned call's explicit opt-in so restricted
        // egress can resolve and inject the value at the boundary.
        credential: credential_handle.cloned(),
        body_credentials: recipe.body_credentials.clone(),
    })
}

/// Resolve the one declaration that authorizes a vendor-call recipe. Matching
/// by method and template path makes manifest ordering irrelevant; zero or
/// multiple matches fail closed instead of silently choosing the first host.
pub(crate) fn resolve_vendor_call_target<'a>(
    recipe: &ChannelVendorCallRecipe,
    declared: &'a [ironclaw_extension_contracts::channel::ChannelEgressDescriptor],
) -> Result<&'a ironclaw_extension_contracts::channel::ChannelEgressDescriptor, ChannelError> {
    let method = match recipe.method {
        ChannelVendorCallMethod::Post => NetworkMethod::Post,
        ChannelVendorCallMethod::Get => NetworkMethod::Get,
    };
    let mut matches = declared.iter().filter(|target| {
        target.methods.contains(&method)
            && crate::egress::DeclaredChannelEgress::from_descriptor(target)
                .matches_recipe_path(&recipe.path)
    });
    let Some(target) = matches.next() else {
        return Err(ChannelError::VendorWiring {
            reason: "vendor call matches no declared egress target".to_string(),
        });
    };
    if matches.next().is_some() {
        return Err(ChannelError::VendorWiring {
            reason: "vendor call matches more than one declared egress target".to_string(),
        });
    }
    Ok(target)
}

/// Run one recipe and classify the outcome. A non-2xx is vendor wiring
/// failure, never a partial success.
pub async fn run_vendor_call(
    recipe: &ChannelVendorCallRecipe,
    host: &str,
    credential_handle: Option<&ironclaw_host_api::ids::SecretHandle>,
    config: &[(String, String)],
    egress: &dyn RestrictedEgress,
    label: &str,
) -> Result<(), ChannelError> {
    let request = render_vendor_call(recipe, host, credential_handle, config)?;
    let response = egress
        .send(request)
        .await
        .map_err(|error| ChannelError::VendorWiring {
            reason: format!("{label} egress failed: {error}"),
        })?;
    if !(200..300).contains(&response.status) {
        return Err(ChannelError::VendorWiring {
            reason: format!("{label} returned status {}", response.status),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::tool_adapter::{
        RestrictedEgressError, RestrictedEgressResponse,
    };
    use ironclaw_host_api::ids::SecretHandle;

    struct ScriptedEgress(Result<RestrictedEgressResponse, RestrictedEgressError>);

    #[async_trait::async_trait]
    impl RestrictedEgress for ScriptedEgress {
        async fn send(
            &self,
            _request: RestrictedEgressRequest,
        ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
            self.0.clone()
        }
    }

    fn config() -> Vec<(String, String)> {
        vec![
            (
                "telegram_webhook_url".to_string(),
                "https://host.example/webhooks/extensions/telegram/updates".to_string(),
            ),
            ("bot_username".to_string(), "ironclaw_test_bot".to_string()),
        ]
    }

    fn registration_recipe() -> ChannelVendorCallRecipe {
        ChannelVendorCallRecipe {
            method: ChannelVendorCallMethod::Post,
            path: "/bot{telegram_bot_token}/setWebhook".to_string(),
            body: Some(serde_json::json!({ "url": "{telegram_webhook_url}" })),
            body_credentials: vec![SecretHandle::new("telegram_webhook_secret").expect("handle")],
        }
    }

    /// The whole point of rule 1: a placeholder naming a SECRET is not in the
    /// non-secret config, so it survives verbatim for the egress layer's
    /// `path_placeholder` injection. Resolving it here would mean the secret
    /// passed through host code that has no business holding it.
    #[test]
    fn an_unresolved_placeholder_survives_for_egress_injection() {
        let credential = SecretHandle::new("telegram_bot_token").expect("credential handle");
        let request = render_vendor_call(
            &registration_recipe(),
            "api.telegram.org",
            Some(&credential),
            &config(),
        )
        .expect("render");
        assert_eq!(
            request.url, "https://api.telegram.org/bot{telegram_bot_token}/setWebhook",
            "the credential placeholder must reach egress unresolved"
        );
    }

    /// Rule 2: the recipe names the secret HANDLE; the rendered body carries
    /// only the non-secret substitution. `secret_token` is inserted by egress
    /// at the manifest's declared pointer, so it must be absent here — and so
    /// must the handle name, which the vendor must never see.
    #[test]
    fn the_rendered_body_carries_no_secret_value_and_no_handle_name() {
        let credential = SecretHandle::new("telegram_bot_token").expect("credential handle");
        let request = render_vendor_call(
            &registration_recipe(),
            "api.telegram.org",
            Some(&credential),
            &config(),
        )
        .expect("render");
        let body: serde_json::Value =
            serde_json::from_slice(request.body.as_deref().expect("body")).expect("json");
        assert_eq!(
            body["url"],
            "https://host.example/webhooks/extensions/telegram/updates"
        );
        assert!(
            body.get("secret_token").is_none(),
            "the host inserts the secret value at the declared pointer, not this renderer"
        );
        assert!(
            !String::from_utf8_lossy(request.body.as_deref().unwrap())
                .contains("telegram_webhook_secret"),
            "a handle name must never be sent to the vendor"
        );
        assert_eq!(
            request
                .body_credentials
                .iter()
                .map(SecretHandle::as_str)
                .collect::<Vec<_>>(),
            vec!["telegram_webhook_secret"],
            "the handle rides as a declared body credential"
        );
        assert!(
            request.credential.as_ref() == Some(&credential),
            "the host-owned call must opt into the target's declared credential"
        );
    }

    /// A config value that itself looks like a placeholder must not be
    /// re-expanded — otherwise operator input could reach a second lookup.
    #[test]
    fn substitution_is_single_pass() {
        let config = vec![
            ("outer".to_string(), "{inner}".to_string()),
            ("inner".to_string(), "SECRET".to_string()),
        ];
        assert_eq!(substitute("/x/{outer}/y", &config), "/x/{inner}/y");
    }

    #[test]
    fn json_keys_are_never_templated() {
        let config = vec![("k".to_string(), "renamed".to_string())];
        let rendered = substitute_json(&serde_json::json!({ "{k}": "{k}" }), &config);
        assert_eq!(rendered, serde_json::json!({ "{k}": "renamed" }));
    }

    #[test]
    fn an_unrooted_path_fails_closed() {
        let mut recipe = registration_recipe();
        recipe.path = "setWebhook".to_string();
        assert!(matches!(
            render_vendor_call(&recipe, "api.telegram.org", None, &config()),
            Err(ChannelError::VendorWiring { .. })
        ));
    }

    #[test]
    fn rendered_vendor_paths_cannot_add_query_fragment_or_traversal() {
        for path in [
            "/setWebhook?override=1",
            "/setWebhook#fragment",
            "/files/%2e%2e%2fsecret",
        ] {
            let mut recipe = registration_recipe();
            recipe.path = path.to_string();
            assert!(
                matches!(
                    render_vendor_call(&recipe, "api.telegram.org", None, &config()),
                    Err(ChannelError::VendorWiring { .. })
                ),
                "rendered path must be rejected: {path}"
            );
        }
    }

    #[test]
    fn a_bodyless_recipe_sends_no_body_and_no_content_type() {
        let recipe = ChannelVendorCallRecipe {
            method: ChannelVendorCallMethod::Post,
            path: "/bot{telegram_bot_token}/deleteWebhook".to_string(),
            body: None,
            body_credentials: Vec::new(),
        };
        let request =
            render_vendor_call(&recipe, "api.telegram.org", None, &config()).expect("render");
        assert!(request.body.is_none());
        assert!(request.headers.is_empty());
        assert!(request.url.ends_with("/deleteWebhook"));
    }

    #[tokio::test]
    async fn a_non_success_vendor_status_is_a_labeled_wiring_failure() {
        let egress = ScriptedEgress(Ok(RestrictedEgressResponse {
            status: 503,
            body: Vec::new(),
        }));

        let error = run_vendor_call(
            &registration_recipe(),
            "api.telegram.org",
            None,
            &config(),
            &egress,
            "registration",
        )
        .await
        .expect_err("non-2xx must fail the lifecycle transition");

        assert!(matches!(
            error,
            ChannelError::VendorWiring { reason }
                if reason == "registration returned status 503"
        ));
    }

    #[tokio::test]
    async fn a_success_vendor_status_completes_the_vendor_call() {
        let egress = ScriptedEgress(Ok(RestrictedEgressResponse {
            status: 204,
            body: Vec::new(),
        }));

        run_vendor_call(
            &registration_recipe(),
            "api.telegram.org",
            None,
            &config(),
            &egress,
            "registration",
        )
        .await
        .expect("2xx completes the lifecycle vendor call");
    }

    #[tokio::test]
    async fn a_transport_failure_is_a_labeled_wiring_failure() {
        let egress = ScriptedEgress(Err(RestrictedEgressError::Transport {
            reason: "scripted timeout".to_string(),
        }));

        let error = run_vendor_call(
            &registration_recipe(),
            "api.telegram.org",
            None,
            &config(),
            &egress,
            "deregistration",
        )
        .await
        .expect_err("transport failure must fail the vendor call");

        assert!(matches!(
            error,
            ChannelError::VendorWiring { reason }
                if reason.contains("deregistration egress failed")
                    && reason.contains("scripted timeout")
        ));
    }
}
