//! DID document generation for instance identities.

use serde::{Deserialize, Serialize};

use crate::did::did_key;

/// Serializable DID document for the instance identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DidDocument {
    #[serde(rename = "@context")]
    pub context: Vec<String>,
    pub id: String,
    #[serde(rename = "verificationMethod")]
    pub verification_method: Vec<VerificationMethod>,
    pub authentication: Vec<String>,
    #[serde(rename = "assertionMethod")]
    pub assertion_method: Vec<String>,
    #[serde(rename = "capabilityInvocation")]
    pub capability_invocation: Vec<String>,
    #[serde(rename = "capabilityDelegation")]
    pub capability_delegation: Vec<String>,
}

/// Verification method entry inside the DID document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationMethod {
    pub id: String,
    #[serde(rename = "type")]
    pub verification_type: String,
    pub controller: String,
    #[serde(rename = "publicKeyMultibase")]
    pub public_key_multibase: String,
}

/// Build a conservative `did:key` DID document for the instance identity.
pub fn did_key_document(did: &str, public_key_multibase: &str) -> DidDocument {
    let key_id = did_key::key_id(did, public_key_multibase);
    let verification_method = VerificationMethod {
        id: key_id.clone(),
        verification_type: "Multikey".to_string(),
        controller: did.to_string(),
        public_key_multibase: public_key_multibase.to_string(),
    };

    DidDocument {
        context: vec![
            "https://www.w3.org/ns/did/v1".to_string(),
            "https://w3id.org/security/multikey/v1".to_string(),
        ],
        id: did.to_string(),
        verification_method: vec![verification_method],
        authentication: vec![key_id.clone()],
        assertion_method: vec![key_id.clone()],
        capability_invocation: vec![key_id.clone()],
        capability_delegation: vec![key_id],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_document_includes_authentication_method() {
        let did = "did:key:zExample";
        let multibase = "zExample";
        let document = did_key_document(did, multibase);

        assert_eq!(document.id, did);
        assert_eq!(document.verification_method.len(), 1);
        assert_eq!(document.authentication.len(), 1);
        assert_eq!(document.authentication[0], format!("{did}#{multibase}"));
        assert_eq!(
            document.verification_method[0].public_key_multibase,
            multibase.to_string()
        );
    }
}
