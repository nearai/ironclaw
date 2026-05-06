//! NIP-19 bech32 encoding/decoding for npub and nsec.

use bech32::{decode, encode, Bech32, Hrp};

/// Convert between bit sizes (e.g. 5-bit to 8-bit and vice versa).
/// Simplified implementation for bech32 data conversion.
fn convert_bits(data: &[u8], from_bits: u8, to_bits: u8, pad: bool) -> Result<Vec<u8>, String> {
    let acc: u64 = 0;
    let bits: u64 = 0;
    let max_v: u64 = (1 << to_bits) - 1;
    let mut ret = Vec::new();

    let mut acc = acc;
    let mut bits = bits;

    for &value in data {
        let v = value as u64;
        if v >> from_bits != 0 {
            return Err("invalid value for input bits".into());
        }
        acc = (acc << from_bits) | v;
        bits += from_bits as u64;
        while bits >= to_bits as u64 {
            bits -= to_bits as u64;
            ret.push(((acc >> bits) & max_v) as u8);
        }
    }

    if pad {
        if bits > 0 {
            ret.push(((acc << (to_bits as u64 - bits)) & max_v) as u8);
        }
    } else if bits >= from_bits as u64 || ((acc << (to_bits as u64 - bits)) & max_v) != 0 {
        return Err("non-zero padding".into());
    }

    Ok(ret)
}

/// Decode a bech32-encoded nsec or npub string into raw 32-byte key.
pub fn decode_key(input: &str) -> Result<[u8; 32], String> {
    let (hrp, data5bit) = decode(input).map_err(|e| format!("bech32 decode error: {e}"))?;

    let expected = if input.starts_with("nsec1") {
        "nsec"
    } else if input.starts_with("npub1") {
        "npub"
    } else {
        return Err(format!("Expected nsec1... or npub1..., got: {input}"));
    };

    if hrp.as_str() != expected {
        return Err(format!("Expected HRP '{expected}', got '{}'", hrp.as_str()));
    }

    let decoded = convert_bits(&data5bit, 5, 8, false)?;

    if decoded.len() != 32 {
        return Err(format!("Expected 32 bytes, got {}", decoded.len()));
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&decoded);
    Ok(key)
}

/// Encode a 32-byte private key as nsec1...
pub fn encode_nsec(key: &[u8; 32]) -> Result<String, String> {
    let hrp = Hrp::parse("nsec").map_err(|e| format!("hrp parse: {e}"))?;
    let data = convert_bits(key, 8, 5, true)?;
    encode::<Bech32>(hrp, &data).map_err(|e| format!("bech32 encode: {e}"))
}

/// Encode a 32-byte public key as npub1...
pub fn encode_npub(key: &[u8; 32]) -> Result<String, String> {
    let hrp = Hrp::parse("npub").map_err(|e| format!("hrp parse: {e}"))?;
    let data = convert_bits(key, 8, 5, true)?;
    encode::<Bech32>(hrp, &data).map_err(|e| format!("bech32 encode: {e}"))
}

/// Parse a pubkey that may be hex or npub1... format.
pub fn parse_pubkey(input: &str) -> Result<[u8; 32], String> {
    if input.starts_with("npub1") {
        decode_key(input)
    } else {
        let bytes = hex::decode(input.trim())
            .map_err(|e| format!("invalid hex pubkey: {e}"))?;
        if bytes.len() != 32 {
            return Err(format!("pubkey must be 32 bytes, got {}", bytes.len()));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_nsec() {
        let key = [0x42u8; 32];
        let encoded = encode_nsec(&key).unwrap();
        assert!(encoded.starts_with("nsec1"));
        let decoded = decode_key(&encoded).unwrap();
        assert_eq!(key, decoded);
    }

    #[test]
    fn roundtrip_npub() {
        let key = [0xABu8; 32];
        let encoded = encode_npub(&key).unwrap();
        assert!(encoded.starts_with("npub1"));
        let decoded = decode_key(&encoded).unwrap();
        assert_eq!(key, decoded);
    }

    #[test]
    fn parse_hex_pubkey() {
        let hex = "00".repeat(32);
        let key = parse_pubkey(&hex).unwrap();
        assert_eq!(key, [0u8; 32]);
    }
}
