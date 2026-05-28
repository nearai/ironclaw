use secrecy::{ExposeSecret, SecretString};
use url::form_urlencoded::Serializer;

pub(super) fn serialize_authorization_code_token_request(
    client_id: &str,
    redirect_uri: &str,
    client_secret: Option<&SecretString>,
    authorization_code: &str,
    pkce_verifier: &str,
) -> Vec<u8> {
    let mut serializer = Serializer::new(String::new());
    serializer
        .append_pair("grant_type", "authorization_code")
        .append_pair("code", authorization_code)
        .append_pair("code_verifier", pkce_verifier)
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri);
    if let Some(client_secret) = client_secret {
        serializer.append_pair("client_secret", client_secret.expose_secret());
    }
    serializer.finish().into_bytes()
}

pub(super) fn serialize_refresh_token_request(
    client_id: &str,
    client_secret: Option<&SecretString>,
    refresh_token: &str,
) -> Vec<u8> {
    let mut serializer = Serializer::new(String::new());
    serializer
        .append_pair("grant_type", "refresh_token")
        .append_pair("refresh_token", refresh_token)
        .append_pair("client_id", client_id);
    if let Some(client_secret) = client_secret {
        serializer.append_pair("client_secret", client_secret.expose_secret());
    }
    serializer.finish().into_bytes()
}
