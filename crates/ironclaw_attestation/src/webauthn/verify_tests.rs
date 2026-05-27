/// Test-only software authenticator that mints ES256 (P-256) and EdDSA
/// (Ed25519) assertions, used to drive the verifier through valid + adversarial
/// paths.
pub(crate) mod test_authenticator {
    use crate::webauthn::cose::CosePublicKey;
    use crate::webauthn::verify::*;

    /// A software authenticator over one of the supported algorithms.
    pub(crate) enum SoftwareAuthenticator {
        /// ES256 / P-256.
        Es256(p256::ecdsa::SigningKey),
        /// EdDSA / Ed25519.
        Ed25519(Box<ed25519_dalek::SigningKey>),
    }

    impl SoftwareAuthenticator {
        /// Generate a fresh P-256 (ES256) authenticator.
        pub(crate) fn new_p256() -> Self {
            SoftwareAuthenticator::Es256(p256::ecdsa::SigningKey::random(&mut rand_core::OsRng))
        }

        /// Generate a fresh Ed25519 (EdDSA) authenticator.
        pub(crate) fn new_ed25519() -> Self {
            SoftwareAuthenticator::Ed25519(Box::new(ed25519_dalek::SigningKey::generate(
                &mut rand_core::OsRng,
            )))
        }

        /// The COSE public key to register (in our pure-Rust representation).
        pub(crate) fn cose_key(&self) -> CosePublicKey {
            match self {
                SoftwareAuthenticator::Es256(sk) => {
                    let vk = sk.verifying_key();
                    let sec1 = vk.to_encoded_point(false).as_bytes().to_vec();
                    CosePublicKey::Es256 { sec1 }
                }
                SoftwareAuthenticator::Ed25519(sk) => CosePublicKey::Ed25519 {
                    public: sk.verifying_key().to_bytes(),
                },
            }
        }

        /// Build `authenticatorData` for the given rp_id + flags + sign count.
        pub(crate) fn authenticator_data(rp_id: &str, flag_byte: u8, sign_count: u32) -> Vec<u8> {
            let mut data = Vec::with_capacity(37);
            let rp_hash: [u8; 32] = Sha256::digest(rp_id.as_bytes()).into();
            data.extend_from_slice(&rp_hash);
            data.push(flag_byte);
            data.extend_from_slice(&sign_count.to_be_bytes());
            data
        }

        /// Build a `clientDataJSON` echoing `challenge` for the given origin
        /// (same-origin: `crossOrigin:false`, no `topOrigin`).
        pub(crate) fn client_data_json(
            type_: &str,
            challenge: &ChallengeCommitment,
            origin: &str,
        ) -> Vec<u8> {
            let chal_b64 = b64url_encode(challenge.as_bytes());
            format!(
                r#"{{"type":"{type_}","challenge":"{chal_b64}","origin":"{origin}","crossOrigin":false}}"#
            )
            .into_bytes()
        }

        /// Build a `clientDataJSON` with an explicit `crossOrigin` flag and an
        /// optional `topOrigin`, for cross-origin policy tests.
        pub(crate) fn client_data_json_cross(
            type_: &str,
            challenge: &ChallengeCommitment,
            origin: &str,
            cross_origin: bool,
            top_origin: Option<&str>,
        ) -> Vec<u8> {
            let chal_b64 = b64url_encode(challenge.as_bytes());
            let top = match top_origin {
                Some(t) => format!(r#","topOrigin":"{t}""#),
                None => String::new(),
            };
            format!(
                r#"{{"type":"{type_}","challenge":"{chal_b64}","origin":"{origin}","crossOrigin":{cross_origin}{top}}}"#
            )
            .into_bytes()
        }

        /// Build a `clientDataJSON` with NO `crossOrigin` key at all (it is
        /// OPTIONAL; absent must be treated as `false`).
        pub(crate) fn client_data_json_no_cross_key(
            type_: &str,
            challenge: &ChallengeCommitment,
            origin: &str,
        ) -> Vec<u8> {
            let chal_b64 = b64url_encode(challenge.as_bytes());
            format!(r#"{{"type":"{type_}","challenge":"{chal_b64}","origin":"{origin}"}}"#)
                .into_bytes()
        }

        /// Sign `authenticatorData ∥ SHA-256(clientDataJSON)`. ES256 returns a
        /// DER ECDSA signature; EdDSA returns the raw 64-byte signature — each
        /// matching what a real authenticator of that type emits.
        pub(crate) fn sign(&self, authenticator_data: &[u8], client_data_json: &[u8]) -> Vec<u8> {
            let client_data_hash: [u8; 32] = Sha256::digest(client_data_json).into();
            let mut msg = Vec::new();
            msg.extend_from_slice(authenticator_data);
            msg.extend_from_slice(&client_data_hash);
            match self {
                SoftwareAuthenticator::Es256(sk) => {
                    use p256::ecdsa::signature::Signer;
                    let sig: p256::ecdsa::Signature = sk.sign(&msg);
                    sig.to_der().as_bytes().to_vec()
                }
                SoftwareAuthenticator::Ed25519(sk) => {
                    use ed25519_dalek::Signer;
                    sk.sign(&msg).to_bytes().to_vec()
                }
            }
        }
    }

    /// base64url no-pad encode (test helper).
    fn b64url_encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        let mut acc: u32 = 0;
        let mut nbits = 0u32;
        for &b in bytes {
            acc = (acc << 8) | b as u32;
            nbits += 8;
            while nbits >= 6 {
                nbits -= 6;
                out.push(ALPHABET[((acc >> nbits) & 0x3f) as usize] as char);
            }
        }
        if nbits > 0 {
            out.push(ALPHABET[((acc << (6 - nbits)) & 0x3f) as usize] as char);
        }
        out
    }
}

mod tests {
    use super::test_authenticator::SoftwareAuthenticator;
    use crate::challenge::{
        ChallengeCommitment, ChallengePreimage, ConsumedChallenge, CredentialId, DeliveryAttemptId,
    };
    use crate::webauthn::registry::{
        AttestationPolicy, BackupFlagPolicy, BootstrapPolicy, InMemoryWebAuthnCredentialRegistry,
        OriginContext, OriginPolicy, RegistrationRequest, SignCountPolicy, StandardOriginPolicy,
    };
    use crate::webauthn::verify::*;
    use ironclaw_signing_provider::{
        ActorId, ApprovedTxHash, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, TenantId, UserId,
    };

    const RP_ID: &str = "ironclaw.example";
    const ORIGIN: &str = "https://ironclaw.example";
    const USER_HANDLE: &[u8] = b"alice-handle";

    struct AllowAll;
    impl AttestationPolicy for AllowAll {
        fn evaluate(&self, _r: &RegistrationRequest) -> Result<(), String> {
            Ok(())
        }
    }
    impl BackupFlagPolicy for AllowAll {
        fn evaluate(&self, _r: &RegistrationRequest) -> Result<(), String> {
            Ok(())
        }
    }
    impl BootstrapPolicy for AllowAll {
        fn evaluate(&self, _r: &RegistrationRequest, _n: usize) -> Result<(), String> {
            Ok(())
        }
    }

    /// Origin policy that only accepts the exact expected origin and rejects
    /// cross-origin assertions (the safe default posture).
    struct ExactOrigin;
    impl OriginPolicy for ExactOrigin {
        fn evaluate(&self, ctx: &OriginContext<'_>) -> Result<(), String> {
            if ctx.origin != ORIGIN {
                return Err(format!("disallowed origin {}", ctx.origin));
            }
            if ctx.cross_origin {
                return Err("cross-origin not permitted".to_string());
            }
            Ok(())
        }
    }

    /// Strict sign-count policy: asserted must be strictly greater than stored
    /// (rejects regression AND equal counts).
    struct StrictlyIncreasing;
    impl SignCountPolicy for StrictlyIncreasing {
        fn evaluate(&self, stored: u32, asserted: u32) -> Result<(), String> {
            if asserted > stored {
                Ok(())
            } else {
                Err(format!("non-increasing sign count {asserted} <= {stored}"))
            }
        }
    }

    fn preimage(commit_seed: u8) -> ChallengePreimage {
        ChallengePreimage {
            rp_id: RP_ID.to_string(),
            expected_origin: ORIGIN.to_string(),
            tenant: TenantId::new("tenant-a"),
            user: UserId::new("alice"),
            scope: ScopeId::new("scope-x"),
            actor: ActorId::new("actor-7"),
            credential_id: CredentialId::new(b"cred-1".to_vec()),
            run_id: RunId::new("run-42"),
            gate_ref: GateRef::new("gate:abc"),
            key_or_account_id: KeyOrAccountId::new("0xabc"),
            chain_id: ChainId::new("eip155:1"),
            expiry_ms: 10_000,
            delivery_attempt: DeliveryAttemptId::new("attempt-1"),
            rendered_tx_digest: ApprovedTxHash::from_bytes([commit_seed; 32]),
        }
    }

    fn consumed(preimage: ChallengePreimage) -> ConsumedChallenge {
        ConsumedChallenge {
            id: crate::challenge::ChallengeId::new("c1"),
            preimage,
        }
    }

    struct Fixture {
        registry: InMemoryWebAuthnCredentialRegistry,
        auth: SoftwareAuthenticator,
    }

    fn fixture(initial_sign_count: u32, backup_eligible: bool) -> Fixture {
        fixture_with(
            SoftwareAuthenticator::new_p256(),
            initial_sign_count,
            backup_eligible,
        )
    }

    fn fixture_with(
        auth: SoftwareAuthenticator,
        initial_sign_count: u32,
        backup_eligible: bool,
    ) -> Fixture {
        let registry = InMemoryWebAuthnCredentialRegistry::new(
            Box::new(AllowAll),
            Box::new(AllowAll),
            Box::new(AllowAll),
        );
        registry
            .register(RegistrationRequest {
                user: UserId::new("alice"),
                credential_id: CredentialId::new(b"cred-1".to_vec()),
                public_key: auth.cose_key(),
                aaguid: [0u8; 16],
                initial_sign_count,
                backup_eligible,
                backup_state: false,
            })
            .expect("register");
        Fixture { registry, auth }
    }

    /// Default valid flags: UP + UV set.
    const UP_UV: u8 = flags::UP | flags::UV;

    #[allow(clippy::too_many_arguments)]
    fn run(
        fx: &Fixture,
        pre: &ChallengePreimage,
        flag_byte: u8,
        asserted_count: u32,
        client_type: &str,
        origin: &str,
        challenge_commit: &ChallengeCommitment,
        user_handle: Option<&[u8]>,
        tamper_sig: bool,
    ) -> Result<VerifiedAssertion, VerificationError> {
        let ad = SoftwareAuthenticator::authenticator_data(RP_ID, flag_byte, asserted_count);
        let cdj = SoftwareAuthenticator::client_data_json(client_type, challenge_commit, origin);
        let mut sig = fx.auth.sign(&ad, &cdj);
        if tamper_sig {
            // Flip a byte in the DER signature body.
            let last = sig.len() - 1;
            sig[last] ^= 0xff;
        }
        let cons = consumed(pre.clone());
        let input = AssertionInput {
            expected_user: &UserId::new("alice"),
            user_handle,
            credential_id: &CredentialId::new(b"cred-1".to_vec()),
            authenticator_data: &ad,
            client_data_json: &cdj,
            signature: &sig,
            rp_id: RP_ID,
            consumed_challenge: &cons,
            expected_user_handle: USER_HANDLE,
        };
        verify_assertion(&fx.registry, &ExactOrigin, &StrictlyIncreasing, &input)
    }

    #[test]
    fn valid_assertion_with_uv_accepted() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let out = run(
            &fx,
            &pre,
            UP_UV,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            false,
        )
        .expect("valid assertion must verify");
        assert_eq!(out.user, UserId::new("alice"));
        assert_eq!(out.new_sign_count, 1);
    }

    #[test]
    fn authenticator_data_too_short_rejected() {
        // authenticatorData shorter than the 37-byte fixed header (32-byte
        // rpIdHash + 1 flag byte + 4-byte signCount) must fail closed with
        // AuthenticatorDataTooShort. The clientData (type/challenge/origin) is
        // valid so the verifier reaches the length check rather than bailing
        // earlier.
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        // 36 bytes: one short of the 37-byte minimum.
        let short_ad = vec![0u8; 36];
        let cdj = SoftwareAuthenticator::client_data_json("webauthn.get", &commit, ORIGIN);
        let sig = fx.auth.sign(&short_ad, &cdj);
        let cons = consumed(pre.clone());
        let input = AssertionInput {
            expected_user: &UserId::new("alice"),
            user_handle: None,
            credential_id: &CredentialId::new(b"cred-1".to_vec()),
            authenticator_data: &short_ad,
            client_data_json: &cdj,
            signature: &sig,
            rp_id: RP_ID,
            consumed_challenge: &cons,
            expected_user_handle: USER_HANDLE,
        };
        assert_eq!(
            verify_assertion(&fx.registry, &ExactOrigin, &StrictlyIncreasing, &input),
            Err(VerificationError::AuthenticatorDataTooShort)
        );
    }

    #[test]
    fn valid_ed25519_assertion_with_uv_accepted() {
        // Same full RP path, but the authenticator is EdDSA/Ed25519 — exercises
        // the pure-Rust ed25519-dalek verification leaf.
        let fx = fixture_with(SoftwareAuthenticator::new_ed25519(), 0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let out = run(
            &fx,
            &pre,
            UP_UV,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            false,
        )
        .expect("valid Ed25519 assertion must verify");
        assert_eq!(out.user, UserId::new("alice"));
        assert_eq!(out.new_sign_count, 1);
    }

    #[test]
    fn bad_ed25519_signature_rejected() {
        let fx = fixture_with(SoftwareAuthenticator::new_ed25519(), 0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let res = run(
            &fx,
            &pre,
            UP_UV,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            true,
        );
        assert!(matches!(
            res,
            Err(VerificationError::BadSignature)
                | Err(VerificationError::VerificationInternal { .. })
        ));
    }

    #[test]
    fn up_only_uv_clear_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let res = run(
            &fx,
            &pre,
            flags::UP,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            false,
        );
        assert_eq!(res, Err(VerificationError::UserVerificationMissing));
    }

    #[test]
    fn up_clear_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        // UV set but UP clear is malformed.
        let res = run(
            &fx,
            &pre,
            flags::UV,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            false,
        );
        assert_eq!(res, Err(VerificationError::UserPresenceMissing));
    }

    #[test]
    fn wrong_client_data_type_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let res = run(
            &fx,
            &pre,
            UP_UV,
            1,
            "webauthn.create",
            ORIGIN,
            &commit,
            None,
            false,
        );
        assert_eq!(res, Err(VerificationError::WrongClientDataType));
    }

    #[test]
    fn challenge_mismatch_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        // Echo a DIFFERENT challenge commitment than the one bound in `pre`.
        let other = preimage(99).commitment();
        let res = run(
            &fx,
            &pre,
            UP_UV,
            1,
            "webauthn.get",
            ORIGIN,
            &other,
            None,
            false,
        );
        assert_eq!(res, Err(VerificationError::ChallengeMismatch));
    }

    #[test]
    fn wrong_rp_id_hash_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        // Build authenticatorData for a DIFFERENT rp_id, but verify against RP_ID.
        let ad = SoftwareAuthenticator::authenticator_data("evil.example", UP_UV, 1);
        let cdj = SoftwareAuthenticator::client_data_json("webauthn.get", &commit, ORIGIN);
        let sig = fx.auth.sign(&ad, &cdj);
        let cons = consumed(pre.clone());
        let input = AssertionInput {
            expected_user: &UserId::new("alice"),
            user_handle: None,
            credential_id: &CredentialId::new(b"cred-1".to_vec()),
            authenticator_data: &ad,
            client_data_json: &cdj,
            signature: &sig,
            rp_id: RP_ID,
            consumed_challenge: &cons,
            expected_user_handle: USER_HANDLE,
        };
        assert_eq!(
            verify_assertion(&fx.registry, &ExactOrigin, &StrictlyIncreasing, &input),
            Err(VerificationError::RpIdHashMismatch)
        );
    }

    #[test]
    fn disallowed_origin_rejected_as_preimage_mismatch() {
        // The asserted origin differs from the origin bound in the preimage:
        // this now fails closed as a preimage-context mismatch (before the
        // injectable policy even runs).
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let res = run(
            &fx,
            &pre,
            UP_UV,
            1,
            "webauthn.get",
            "https://evil.example",
            &commit,
            None,
            false,
        );
        assert_eq!(
            res,
            Err(VerificationError::PreimageContextMismatch { field: "origin" })
        );
    }

    #[test]
    fn origin_policy_rejection_surfaces_as_origin_rejected() {
        // Origin matches the preimage, but an injectable policy that rejects the
        // posture (here: a cross-origin assertion under the same-origin-only
        // ExactOrigin) surfaces as OriginRejected — the policy path is live.
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let ad = SoftwareAuthenticator::authenticator_data(RP_ID, UP_UV, 1);
        let cdj = SoftwareAuthenticator::client_data_json_cross(
            "webauthn.get",
            &commit,
            ORIGIN,
            true,
            Some(ORIGIN),
        );
        let sig = fx.auth.sign(&ad, &cdj);
        let cons = consumed(pre.clone());
        let input = AssertionInput {
            expected_user: &UserId::new("alice"),
            user_handle: None,
            credential_id: &CredentialId::new(b"cred-1".to_vec()),
            authenticator_data: &ad,
            client_data_json: &cdj,
            signature: &sig,
            rp_id: RP_ID,
            consumed_challenge: &cons,
            expected_user_handle: USER_HANDLE,
        };
        assert!(matches!(
            verify_assertion(&fx.registry, &ExactOrigin, &StrictlyIncreasing, &input),
            Err(VerificationError::OriginRejected { .. })
        ));
    }

    /// Drive the verifier with a fully custom clientDataJSON (for crossOrigin /
    /// preimage-context tests), using `policy` as the origin policy.
    #[allow(clippy::too_many_arguments)]
    fn run_with(
        fx: &Fixture,
        pre: &ChallengePreimage,
        policy: &dyn OriginPolicy,
        expected_user: &UserId,
        credential_id: &CredentialId,
        rp_id: &str,
        cdj: &[u8],
    ) -> Result<VerifiedAssertion, VerificationError> {
        let ad = SoftwareAuthenticator::authenticator_data(RP_ID, UP_UV, 1);
        let sig = fx.auth.sign(&ad, cdj);
        let cons = consumed(pre.clone());
        let input = AssertionInput {
            expected_user,
            user_handle: None,
            credential_id,
            authenticator_data: &ad,
            client_data_json: cdj,
            signature: &sig,
            rp_id,
            consumed_challenge: &cons,
            expected_user_handle: USER_HANDLE,
        };
        verify_assertion(&fx.registry, policy, &StrictlyIncreasing, &input)
    }

    #[test]
    fn cross_origin_true_rejected_by_default() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let cdj = SoftwareAuthenticator::client_data_json_cross(
            "webauthn.get",
            &commit,
            ORIGIN,
            true,
            Some(ORIGIN),
        );
        let policy = StandardOriginPolicy::same_origin_only(ORIGIN);
        let res = run_with(
            &fx,
            &pre,
            &policy,
            &UserId::new("alice"),
            &CredentialId::new(b"cred-1".to_vec()),
            RP_ID,
            &cdj,
        );
        assert!(matches!(res, Err(VerificationError::OriginRejected { .. })));
    }

    #[test]
    fn cross_origin_true_missing_top_origin_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        // crossOrigin:true but NO topOrigin -> rejected even when cross-origin
        // is allowed.
        let cdj = SoftwareAuthenticator::client_data_json_cross(
            "webauthn.get",
            &commit,
            ORIGIN,
            true,
            None,
        );
        let policy = StandardOriginPolicy::allow_cross_origin_with_top(ORIGIN);
        let res = run_with(
            &fx,
            &pre,
            &policy,
            &UserId::new("alice"),
            &CredentialId::new(b"cred-1".to_vec()),
            RP_ID,
            &cdj,
        );
        assert!(matches!(res, Err(VerificationError::OriginRejected { .. })));
    }

    #[test]
    fn cross_origin_true_inconsistent_top_origin_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        // crossOrigin:true with a topOrigin that is NOT the expected origin.
        let cdj = SoftwareAuthenticator::client_data_json_cross(
            "webauthn.get",
            &commit,
            ORIGIN,
            true,
            Some("https://evil.example"),
        );
        let policy = StandardOriginPolicy::allow_cross_origin_with_top(ORIGIN);
        let res = run_with(
            &fx,
            &pre,
            &policy,
            &UserId::new("alice"),
            &CredentialId::new(b"cred-1".to_vec()),
            RP_ID,
            &cdj,
        );
        assert!(matches!(res, Err(VerificationError::OriginRejected { .. })));
    }

    #[test]
    fn cross_origin_true_with_matching_top_origin_accepted_when_allowed() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let cdj = SoftwareAuthenticator::client_data_json_cross(
            "webauthn.get",
            &commit,
            ORIGIN,
            true,
            Some(ORIGIN),
        );
        let policy = StandardOriginPolicy::allow_cross_origin_with_top(ORIGIN);
        let out = run_with(
            &fx,
            &pre,
            &policy,
            &UserId::new("alice"),
            &CredentialId::new(b"cred-1".to_vec()),
            RP_ID,
            &cdj,
        )
        .expect("cross-origin with matching topOrigin must verify when allowed");
        assert_eq!(out.user, UserId::new("alice"));
    }

    #[test]
    fn missing_cross_origin_key_treated_as_false() {
        // No crossOrigin key at all -> defaults to false -> accepted by the
        // same-origin-only policy.
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let cdj =
            SoftwareAuthenticator::client_data_json_no_cross_key("webauthn.get", &commit, ORIGIN);
        let policy = StandardOriginPolicy::same_origin_only(ORIGIN);
        let out = run_with(
            &fx,
            &pre,
            &policy,
            &UserId::new("alice"),
            &CredentialId::new(b"cred-1".to_vec()),
            RP_ID,
            &cdj,
        )
        .expect("absent crossOrigin must be treated as false and accepted");
        assert_eq!(out.new_sign_count, 1);
    }

    #[test]
    fn preimage_user_mismatch_fails_closed() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let cdj = SoftwareAuthenticator::client_data_json("webauthn.get", &commit, ORIGIN);
        // Caller claims a DIFFERENT user than the one bound in the preimage.
        let res = run_with(
            &fx,
            &pre,
            &ExactOrigin,
            &UserId::new("mallory"),
            &CredentialId::new(b"cred-1".to_vec()),
            RP_ID,
            &cdj,
        );
        assert_eq!(
            res,
            Err(VerificationError::PreimageContextMismatch { field: "user" })
        );
    }

    #[test]
    fn preimage_credential_mismatch_fails_closed() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let cdj = SoftwareAuthenticator::client_data_json("webauthn.get", &commit, ORIGIN);
        let res = run_with(
            &fx,
            &pre,
            &ExactOrigin,
            &UserId::new("alice"),
            &CredentialId::new(b"other-cred".to_vec()),
            RP_ID,
            &cdj,
        );
        assert_eq!(
            res,
            Err(VerificationError::PreimageContextMismatch {
                field: "credential_id"
            })
        );
    }

    #[test]
    fn preimage_rp_id_mismatch_fails_closed() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let cdj = SoftwareAuthenticator::client_data_json("webauthn.get", &commit, ORIGIN);
        let res = run_with(
            &fx,
            &pre,
            &ExactOrigin,
            &UserId::new("alice"),
            &CredentialId::new(b"cred-1".to_vec()),
            "other.rp",
            &cdj,
        );
        assert_eq!(
            res,
            Err(VerificationError::PreimageContextMismatch { field: "rp_id" })
        );
    }

    #[test]
    fn verified_assertion_carries_gate_and_tx_digest() {
        // The success output binds the gate_ref + rendered_tx_digest from the
        // consumed preimage (proof of THIS gate/tx).
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let out = run(
            &fx,
            &pre,
            UP_UV,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            false,
        )
        .expect("valid assertion must verify");
        assert_eq!(out.gate_ref, pre.gate_ref);
        assert_eq!(out.rendered_tx_digest, pre.rendered_tx_digest);
    }

    #[test]
    fn non_canonical_challenge_trailing_bits_rejected() {
        // A base64url string whose final char carries non-zero trailing bits is
        // a non-canonical (malleable) encoding and must be rejected at decode.
        let fx = fixture(0, false);
        let pre = preimage(1);
        let ad = SoftwareAuthenticator::authenticator_data(RP_ID, UP_UV, 1);
        // 43 base64url chars decode to 32 bytes (the commitment length); the
        // last char ('B' = value 1) leaves 2 non-zero trailing bits.
        let bad_challenge = format!("{}B", "A".repeat(42));
        let cdj = format!(
            r#"{{"type":"webauthn.get","challenge":"{bad_challenge}","origin":"{ORIGIN}","crossOrigin":false}}"#
        )
        .into_bytes();
        let sig = fx.auth.sign(&ad, &cdj);
        let cons = consumed(pre.clone());
        let input = AssertionInput {
            expected_user: &UserId::new("alice"),
            user_handle: None,
            credential_id: &CredentialId::new(b"cred-1".to_vec()),
            authenticator_data: &ad,
            client_data_json: &cdj,
            signature: &sig,
            rp_id: RP_ID,
            consumed_challenge: &cons,
            expected_user_handle: USER_HANDLE,
        };
        assert!(matches!(
            verify_assertion(&fx.registry, &ExactOrigin, &StrictlyIncreasing, &input),
            Err(VerificationError::MalformedClientData { .. })
        ));
    }

    #[test]
    fn sign_count_regression_rejected() {
        // Stored count 5; assert 3 -> regression.
        let fx = fixture(5, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let res = run(
            &fx,
            &pre,
            UP_UV,
            3,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            false,
        );
        assert!(matches!(
            res,
            Err(VerificationError::SignCountPolicy { .. })
        ));
    }

    #[test]
    fn equal_sign_count_rejected_under_strict_policy() {
        // Stored 5, assert 5 -> rejected by the strictly-increasing policy.
        let fx = fixture(5, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let res = run(
            &fx,
            &pre,
            UP_UV,
            5,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            false,
        );
        assert!(matches!(
            res,
            Err(VerificationError::SignCountPolicy { .. })
        ));
    }

    #[test]
    fn bad_signature_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let res = run(
            &fx,
            &pre,
            UP_UV,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            true,
        );
        assert!(matches!(
            res,
            Err(VerificationError::BadSignature)
                | Err(VerificationError::VerificationInternal { .. })
        ));
    }

    #[test]
    fn unregistered_credential_rejected() {
        let fx = fixture(0, false);
        // Bind the challenge to an unregistered credential id so the assertion
        // context MATCHES the preimage (passing the context-binding step) but
        // the registry lookup misses -> UnknownCredential.
        let mut pre = preimage(1);
        pre.credential_id = CredentialId::new(b"not-registered".to_vec());
        let commit = pre.commitment();
        let ad = SoftwareAuthenticator::authenticator_data(RP_ID, UP_UV, 1);
        let cdj = SoftwareAuthenticator::client_data_json("webauthn.get", &commit, ORIGIN);
        let sig = fx.auth.sign(&ad, &cdj);
        let cons = consumed(pre.clone());
        let input = AssertionInput {
            expected_user: &UserId::new("alice"),
            user_handle: None,
            credential_id: &CredentialId::new(b"not-registered".to_vec()),
            authenticator_data: &ad,
            client_data_json: &cdj,
            signature: &sig,
            rp_id: RP_ID,
            consumed_challenge: &cons,
            expected_user_handle: USER_HANDLE,
        };
        assert_eq!(
            verify_assertion(&fx.registry, &ExactOrigin, &StrictlyIncreasing, &input),
            Err(VerificationError::UnknownCredential)
        );
    }

    #[test]
    fn foreign_user_handle_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let res = run(
            &fx,
            &pre,
            UP_UV,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            Some(b"someone-else"),
            false,
        );
        assert_eq!(res, Err(VerificationError::ForeignUserHandle));
    }

    #[test]
    fn matching_user_handle_accepted() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let out = run(
            &fx,
            &pre,
            UP_UV,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            Some(USER_HANDLE),
            false,
        )
        .expect("matching handle must verify");
        assert_eq!(out.user, UserId::new("alice"));
    }

    #[test]
    fn backup_state_without_eligible_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        // BS set, BE clear -> spec-invalid.
        let res = run(
            &fx,
            &pre,
            UP_UV | flags::BS,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            false,
        );
        assert!(matches!(
            res,
            Err(VerificationError::BackupFlagPolicy { .. })
        ));
    }

    #[test]
    fn backup_eligible_assert_on_ineligible_registration_rejected() {
        // Registered as ineligible; assertion claims BE -> rejected.
        let fx = fixture(0, false);
        let pre = preimage(1);
        let commit = pre.commitment();
        let res = run(
            &fx,
            &pre,
            UP_UV | flags::BE,
            1,
            "webauthn.get",
            ORIGIN,
            &commit,
            None,
            false,
        );
        assert!(matches!(
            res,
            Err(VerificationError::BackupFlagPolicy { .. })
        ));
    }

    #[test]
    fn malformed_client_data_rejected() {
        let fx = fixture(0, false);
        let pre = preimage(1);
        let ad = SoftwareAuthenticator::authenticator_data(RP_ID, UP_UV, 1);
        let cdj = b"not json".to_vec();
        let sig = fx.auth.sign(&ad, &cdj);
        let cons = consumed(pre.clone());
        let input = AssertionInput {
            expected_user: &UserId::new("alice"),
            user_handle: None,
            credential_id: &CredentialId::new(b"cred-1".to_vec()),
            authenticator_data: &ad,
            client_data_json: &cdj,
            signature: &sig,
            rp_id: RP_ID,
            consumed_challenge: &cons,
            expected_user_handle: USER_HANDLE,
        };
        assert!(matches!(
            verify_assertion(&fx.registry, &ExactOrigin, &StrictlyIncreasing, &input),
            Err(VerificationError::MalformedClientData { .. })
        ));
    }
}
