use serde::{Deserialize, Serialize};

/// Persistent session state, stored as base64 in the workspace at telegram/session.json.
///
/// Contains everything needed to resume an encrypted MTProto session between
/// WASM invocations: auth key, server salt, DC identifier, API credentials,
/// and transient login state.
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    /// 256-byte auth key from the DH exchange, hex-encoded for JSON safety.
    pub auth_key_hex: String,
    /// First salt from DH exchange (or most recent salt from server).
    pub first_salt: i64,
    /// Time offset from server, in seconds.
    pub time_offset: i32,
    /// Telegram data center ID (1-5).
    pub dc_id: u8,
    /// Telegram API ID from my.telegram.org.
    pub api_id: i32,
    /// Telegram API hash from my.telegram.org.
    pub api_hash: String,
    /// Whether this session has completed auth key generation.
    pub initialized: bool,
    /// Whether a user is logged in.
    pub logged_in: bool,
    /// Transient: phone_code_hash from auth.sendCode, needed for auth.signIn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_code_hash: Option<String>,
    /// Transient: phone number used during login.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
}

impl Session {
    pub fn new(api_id: i32, api_hash: String, dc_id: u8) -> Self {
        Self {
            auth_key_hex: String::new(),
            first_salt: 0,
            time_offset: 0,
            dc_id,
            api_id,
            api_hash,
            initialized: false,
            logged_in: false,
            phone_code_hash: None,
            phone_number: None,
        }
    }

    pub fn auth_key_bytes(&self) -> Result<[u8; 256], String> {
        let bytes = hex_decode(&self.auth_key_hex)
            .map_err(|e| format!("corrupt auth_key_hex in session: {e}"))?;
        if bytes.len() != 256 {
            return Err(format!(
                "auth_key_hex decoded to {} bytes, expected 256",
                bytes.len()
            ));
        }
        let mut key = [0u8; 256];
        key.copy_from_slice(&bytes);
        Ok(key)
    }

    pub fn set_auth_key(&mut self, key: &[u8; 256]) {
        self.auth_key_hex = hex_encode(key);
    }
}

/// Load session from workspace (returns None if not found or unparseable).
pub fn load_session() -> Option<Session> {
    let data = crate::near::agent::host::workspace_read("telegram/session.json")?;
    serde_json::from_str(&data).ok()
}

/// Serialize session to JSON for the agent to store via memory_write.
pub fn session_to_json(session: &Session) -> Result<String, String> {
    serde_json::to_string_pretty(session).map_err(|e| format!("session serialize failed: {e}"))
}

// Minimal hex encode/decode (no extra dep needed).

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("odd-length hex string".into());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_val(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("invalid hex char: {b}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let data = [0u8, 1, 15, 16, 255, 128, 64];
        let encoded = hex_encode(&data);
        assert_eq!(encoded, "00010f10ff8040");
        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn session_serialization() {
        let mut session = Session::new(12345, "abcdef".into(), 2);
        let key = [42u8; 256];
        session.set_auth_key(&key);
        session.initialized = true;

        let json = session_to_json(&session).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.auth_key_bytes().unwrap(), key);
        assert_eq!(restored.api_id, 12345);
        assert_eq!(restored.dc_id, 2);
        assert!(restored.initialized);
    }
}
