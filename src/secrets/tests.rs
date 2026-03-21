use super::crypto_from_hex;

#[test]
fn test_crypto_from_hex_valid() -> Result<(), String> {
    let hex = "0123456789abcdef".repeat(4);
    match crypto_from_hex(&hex) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("expected valid hex to work, got {e}")),
    }
}

#[test]
fn test_crypto_from_hex_invalid() -> Result<(), String> {
    match crypto_from_hex("too_short") {
        Ok(_) => Err("expected invalid hex to fail".to_string()),
        Err(_) => Ok(()),
    }
}
