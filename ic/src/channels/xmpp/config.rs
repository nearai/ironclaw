//! XMPP channel utility functions.

use uuid::Uuid;

/// Extract bare JID (user@domain) from full JID (user@domain/resource).
pub fn bare_jid(jid: &str) -> &str {
    jid.split('/').next().unwrap_or(jid)
}

/// Generate a deterministic stable thread UUID from a bare JID.
pub fn thread_id_from_jid(jid: &str) -> String {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, jid.as_bytes()).to_string()
}

/// Check whether a JID is allowed by an allowlist.
/// `*` matches any JID. Empty list denies all.
pub fn is_jid_allowed(jid: &str, list: &[String]) -> bool {
    list.iter()
        .any(|entry| entry == "*" || entry.eq_ignore_ascii_case(jid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_jid_strips_resource() {
        assert_eq!(bare_jid("user@domain.com/phone"), "user@domain.com");
        assert_eq!(bare_jid("user@domain.com"), "user@domain.com");
        assert_eq!(bare_jid("user@domain.com/res/extra"), "user@domain.com");
    }

    #[test]
    fn is_jid_allowed_wildcard() {
        assert!(is_jid_allowed("any@host.com", &["*".to_string()]));
        assert!(!is_jid_allowed("any@host.com", &[]));
        assert!(is_jid_allowed(
            "alice@xmpp.org",
            &["alice@xmpp.org".to_string()]
        ));
        assert!(!is_jid_allowed(
            "eve@xmpp.org",
            &["alice@xmpp.org".to_string()]
        ));
    }

    #[test]
    fn thread_id_from_jid_is_deterministic() {
        let id1 = thread_id_from_jid("alice@example.com");
        let id2 = thread_id_from_jid("alice@example.com");
        assert_eq!(id1, id2);
        assert_ne!(id1, thread_id_from_jid("bob@example.com"));
    }
}
