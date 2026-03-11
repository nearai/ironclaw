//! OpenClaw credential import with secure handling.
//!
//! Credential extraction and import is handled in the main importer (mod.rs).
//! The credentials module focuses on security validation and testing.

#[cfg(test)]
mod tests {
    use crate::secrets::CreateSecretParams;
    use secrecy::SecretString;

    #[test]
    fn test_secret_string_not_logged() {
        let secret = SecretString::new("super-secret-key".to_string().into_boxed_str());
        let debug_output = format!("{:?}", secret);

        // Verify that the actual secret is not in the debug output
        assert!(!debug_output.contains("super-secret-key"));
    }

    #[test]
    fn test_create_secret_params_normalized() {
        let params = CreateSecretParams::new("MY_API_KEY", "value123");
        // Secret names should be normalized to lowercase
        assert_eq!(params.name, "my_api_key");
    }
}
