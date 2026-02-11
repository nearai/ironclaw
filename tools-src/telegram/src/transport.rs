use grammers_crypto::DequeBuffer;
use grammers_mtproto::mtp::{Deserialization, Encrypted, Mtp, Plain};
use grammers_tl_types::Serializable;

use crate::near::agent::host;

/// DC names indexed by dc_id (1-based). DC1=pluto, DC2=venus, etc.
const DC_NAMES: &[&str] = &["", "pluto", "venus", "aurora", "vesta", "flora"];

/// Build the HTTPS URL for a Telegram data center's web transport endpoint.
pub fn dc_url(dc_id: u8) -> Result<String, String> {
    let idx = dc_id as usize;
    if idx == 0 || idx >= DC_NAMES.len() {
        return Err(format!("invalid dc_id {dc_id}, must be 1-5"));
    }
    Ok(format!("https://{}.web.telegram.org/apiw", DC_NAMES[idx]))
}

/// Send a plaintext (unencrypted) MTProto request via HTTP POST.
///
/// Used during auth key generation. The request is a TL-serializable type;
/// the response bytes are returned raw for the caller to deserialize.
pub fn post_plain<R: Serializable>(dc_id: u8, request: &R) -> Result<Vec<u8>, String> {
    let url = dc_url(dc_id)?;
    let mut plain = Plain::new();
    let mut buffer = DequeBuffer::with_capacity(0, 0);

    let request_bytes = request.to_bytes();
    plain
        .push(&mut buffer, &request_bytes)
        .ok_or("plain push returned None")?;
    plain.finalize(&mut buffer);

    let body: Vec<u8> = buffer[..].to_vec();
    let response = http_post_binary(&url, &body)?;

    let results = plain
        .deserialize(&response)
        .map_err(|e| format!("plain deserialize: {e}"))?;

    for result in results {
        if let Deserialization::RpcResult(rpc) = result {
            return Ok(rpc.body);
        }
    }
    Err("no RPC result in plain response".into())
}

/// Send an encrypted MTProto RPC request via HTTP POST.
///
/// Pushes a serialized TL request into the Encrypted MTP, finalizes (encrypts),
/// POSTs the ciphertext, then deserializes the response.
///
/// Returns the first RPC result body for the caller to deserialize as the
/// expected response type.
pub fn post_encrypted(
    mtp: &mut Encrypted,
    dc_id: u8,
    request_bytes: &[u8],
) -> Result<Vec<u8>, String> {
    let url = dc_url(dc_id)?;
    let mut buffer = DequeBuffer::with_capacity(0, 0);

    mtp.push(&mut buffer, request_bytes)
        .ok_or("encrypted push returned None")?;
    mtp.finalize(&mut buffer);

    let body: Vec<u8> = buffer[..].to_vec();
    let response = http_post_binary(&url, &body)?;

    let results = mtp
        .deserialize(&response)
        .map_err(|e| format!("encrypted deserialize: {e}"))?;

    for result in results {
        match result {
            Deserialization::RpcResult(rpc) => return Ok(rpc.body),
            Deserialization::RpcError(err) => {
                return Err(format!(
                    "RPC error {}: {}",
                    err.error.error_code, err.error.error_message
                ));
            }
            _ => {}
        }
    }
    Err("no RPC result in encrypted response".into())
}

/// HTTP POST with raw binary body via the WASM host's http-request capability.
fn http_post_binary(url: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    let resp = host::http_request("POST", url, "{}", Some(body), None)?;

    if resp.status < 200 || resp.status >= 300 {
        let body_text = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "HTTP {} from {}: {}",
            resp.status,
            url,
            truncate(&body_text, 200)
        ));
    }

    Ok(resp.body)
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_url_valid() {
        assert_eq!(dc_url(1).unwrap(), "https://pluto.web.telegram.org/apiw");
        assert_eq!(dc_url(2).unwrap(), "https://venus.web.telegram.org/apiw");
        assert_eq!(dc_url(5).unwrap(), "https://flora.web.telegram.org/apiw");
    }

    #[test]
    fn dc_url_invalid() {
        assert!(dc_url(0).is_err());
        assert!(dc_url(6).is_err());
    }
}
