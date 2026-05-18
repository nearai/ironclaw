use std::fmt::Write as _;
use std::io::{self, BufRead, IsTerminal, Write};

use crate::secrets::{Eip712Domain, FieldSource};
use crate::tools::wasm::capabilities_schema::{
    CapabilitiesFile, CredentialLocationSchema, CredentialMappingSchema,
};

const CONSENT_PHRASE: &str = "I understand and consent to grant signing authority";

struct KnownDomain {
    name_prefix: &'static str,
    chain_id: u64,
    label: &'static str,
}

const KNOWN_EIP712_DOMAINS: &[KnownDomain] = &[
    KnownDomain {
        name_prefix: "ClobAuthDomain",
        chain_id: 137,
        label: "Polymarket CLOB",
    },
    KnownDomain {
        name_prefix: "Polymarket CTF Exchange",
        chain_id: 137,
        label: "Polymarket CTF Exchange",
    },
    KnownDomain {
        name_prefix: "Seaport",
        chain_id: 1,
        label: "OpenSea Seaport (Ethereum)",
    },
    KnownDomain {
        name_prefix: "Permit2",
        chain_id: 1,
        label: "Uniswap Permit2 (Ethereum)",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainClassification {
    Known,
    Unknown,
}

pub fn classify_domain(domain: &Eip712Domain) -> DomainClassification {
    for known in KNOWN_EIP712_DOMAINS {
        if domain.name == known.name_prefix && domain.chain_id == known.chain_id {
            return DomainClassification::Known;
        }
    }
    DomainClassification::Unknown
}

pub fn classification_label(domain: &Eip712Domain) -> String {
    match classify_domain(domain) {
        DomainClassification::Known => {
            let label = KNOWN_EIP712_DOMAINS
                .iter()
                .find(|k| k.name_prefix == domain.name && k.chain_id == domain.chain_id)
                .map(|k| k.label)
                .unwrap_or("known");
            format!("known ({})", label)
        }
        DomainClassification::Unknown => "unknown — review carefully".to_string(),
    }
}

pub fn chain_name(chain_id: u64) -> Option<&'static str> {
    match chain_id {
        1 => Some("Ethereum Mainnet"),
        10 => Some("Optimism"),
        56 => Some("BNB Chain"),
        137 => Some("Polygon Mainnet"),
        8453 => Some("Base"),
        42161 => Some("Arbitrum One"),
        43114 => Some("Avalanche C-Chain"),
        11155111 => Some("Sepolia Testnet"),
        _ => None,
    }
}

pub fn render_signing_summary(caps: &CapabilitiesFile, tool_name: &str) -> Option<String> {
    let signing = caps.signing.as_ref()?;
    if signing.schemes.is_empty() {
        return None;
    }

    let mut out = String::new();
    let _ = writeln!(out);
    let _ = writeln!(out, "Tool '{}' declares signing schemes:", tool_name);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  Signing grants the host the right to produce cryptographic signatures"
    );
    let _ = writeln!(
        out,
        "  using your private keys, over values the tool supplies at runtime."
    );

    let mut ids: Vec<&String> = signing.schemes.keys().collect();
    ids.sort();
    for id in ids {
        let mapping = &signing.schemes[id];
        let _ = writeln!(out);
        let _ = writeln!(out, "  Scheme: {}", id);
        let _ = writeln!(out, "    Secret: {}", mapping.secret_name);
        render_scope(&mut out, mapping);
        render_location(&mut out, &mapping.location);
    }

    Some(out)
}

fn render_scope(out: &mut String, mapping: &CredentialMappingSchema) {
    if mapping.host_patterns.is_empty() && mapping.path_patterns.is_empty() {
        let _ = writeln!(out, "    Scope:  any host, any path");
        return;
    }
    let hosts = if mapping.host_patterns.is_empty() {
        "(any host)".to_string()
    } else {
        mapping.host_patterns.join(", ")
    };
    let paths = if mapping.path_patterns.is_empty() {
        "(any path)".to_string()
    } else {
        mapping.path_patterns.join(", ")
    };
    let _ = writeln!(out, "    Scope:  hosts={} paths={}", hosts, paths);
}

fn render_location(out: &mut String, location: &CredentialLocationSchema) {
    match location {
        CredentialLocationSchema::HmacSignedHeader {
            signature_header,
            timestamp_header,
        } => {
            let _ = writeln!(out, "    Type:   HMAC-SHA256 (signs every request body)");
            let _ = writeln!(
                out,
                "    Output: header {}, timestamp header {}",
                signature_header, timestamp_header
            );
        }
        CredentialLocationSchema::Eip712SignedHeader {
            domain,
            primary_type,
            structs,
            ..
        } => {
            let _ = writeln!(out, "    Type:   EIP-712 typed message (secp256k1)");
            render_eip712_domain(out, domain);
            let _ = writeln!(out, "    Primary type: {}", primary_type);
            for s in structs {
                if &s.name != primary_type {
                    continue;
                }
                let _ = writeln!(out, "    Struct {}:", s.name);
                for field in &s.fields {
                    let _ = writeln!(
                        out,
                        "      {:<16} : {:<10} <- {}",
                        field.name,
                        field.type_name,
                        format_field_source(&field.source)
                    );
                }
            }
        }
        CredentialLocationSchema::Nep413SignedHeader {
            recipient_source,
            message_source,
            callback_url_source,
            ..
        } => {
            let _ = writeln!(out, "    Type:   NEP-413 NEAR signed message (ed25519)");
            let _ = writeln!(
                out,
                "    Recipient: <- {}",
                format_field_source(recipient_source)
            );
            let _ = writeln!(
                out,
                "    Message:   <- {}",
                format_field_source(message_source)
            );
            if let Some(cb) = callback_url_source {
                let _ = writeln!(out, "    Callback:  <- {}", format_field_source(cb));
            }
        }
        CredentialLocationSchema::SolanaSignedTransaction { message_source, .. } => {
            let _ = writeln!(out, "    Type:   Solana ed25519 transaction signing");
            let _ = writeln!(
                out,
                "    Message bytes: <- {}",
                format_field_source(message_source)
            );
            let _ = writeln!(
                out,
                "    WARNING: Solana txs can transfer funds and SPL tokens. \
                 Verify the host patterns name only RPC endpoints you trust."
            );
        }
        _ => {
            let _ = writeln!(out, "    Type:   (non-signing location, ignored)");
        }
    }
}

fn render_eip712_domain(out: &mut String, domain: &Eip712Domain) {
    let _ = writeln!(out, "    Domain:");
    let _ = writeln!(out, "      name:              {}", domain.name);
    let _ = writeln!(out, "      version:           {}", domain.version);
    let chain_label = chain_name(domain.chain_id)
        .map(|n| format!("{} ({})", domain.chain_id, n))
        .unwrap_or_else(|| domain.chain_id.to_string());
    let _ = writeln!(out, "      chainId:           {}", chain_label);
    let _ = writeln!(
        out,
        "      verifyingContract: {}",
        domain
            .verifying_contract
            .as_deref()
            .unwrap_or("(none specified)")
    );
    let _ = writeln!(
        out,
        "      Classification:    {}",
        classification_label(domain)
    );
}

fn format_field_source(src: &FieldSource) -> String {
    match src {
        FieldSource::Literal { value } => format!("literal \"{}\"", value),
        FieldSource::SignerAddress => "your wallet address".to_string(),
        FieldSource::SignerPublicKey => "your wallet public key".to_string(),
        FieldSource::RequestTimestampSecs => "current request timestamp".to_string(),
        FieldSource::RequestRandomNonceB64 => "fresh random nonce".to_string(),
        FieldSource::RequestBody => "WHOLE REQUEST BODY (tool-controlled)".to_string(),
        FieldSource::BodyFieldString { path } => {
            format!("tool-controlled string at body.{}", path)
        }
        FieldSource::Bytes32Keccak256OfBytes { parts } => {
            format!("keccak256 of {} byte parts (tool-controlled)", parts.len())
        }
    }
}

pub fn prompt_for_signing_consent(auto_yes: bool) -> anyhow::Result<()> {
    if auto_yes {
        println!("  --yes flag set: signing schemes accepted without interactive confirmation.");
        return Ok(());
    }

    let stdin = io::stdin();
    if !stdin.is_terminal() {
        anyhow::bail!(
            "Signing schemes require interactive consent. Re-run on a terminal, or pass --yes \
             to grant explicit consent for this install."
        );
    }

    println!();
    println!("Type the following phrase EXACTLY to grant signing authority:");
    println!("  \"{}\"", CONSENT_PHRASE);
    println!("Or type anything else (e.g. \"no\") to abort.");
    print!("> ");
    io::stdout().flush()?;

    let mut input = String::new();
    stdin.lock().read_line(&mut input)?;
    let trimmed = input.trim();

    if trimmed == CONSENT_PHRASE {
        Ok(())
    } else {
        anyhow::bail!("Signing consent declined. Installation aborted.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eip712_domain(name: &str, chain_id: u64) -> Eip712Domain {
        Eip712Domain {
            name: name.to_string(),
            version: "1".to_string(),
            chain_id,
            verifying_contract: None,
        }
    }

    #[test]
    fn classify_polymarket_clob_as_known() {
        let domain = eip712_domain("ClobAuthDomain", 137);
        assert_eq!(classify_domain(&domain), DomainClassification::Known);
        assert!(classification_label(&domain).contains("Polymarket CLOB"));
    }

    #[test]
    fn classify_unknown_domain_as_unknown() {
        let domain = eip712_domain("ClobAuthDomain", 999);
        assert_eq!(classify_domain(&domain), DomainClassification::Unknown);
        assert!(classification_label(&domain).contains("review"));

        let mismatched = eip712_domain("AttackerDomain", 1);
        assert_eq!(classify_domain(&mismatched), DomainClassification::Unknown);
    }

    #[test]
    fn chain_name_known_chains_only() {
        assert_eq!(chain_name(1), Some("Ethereum Mainnet"));
        assert_eq!(chain_name(137), Some("Polygon Mainnet"));
        assert_eq!(chain_name(42161), Some("Arbitrum One"));
        assert!(chain_name(99999).is_none());
    }

    #[test]
    fn render_signing_summary_returns_none_without_signing_block() {
        let json = r#"{ "http": { "allowlist": [] } }"#;
        let caps = CapabilitiesFile::from_json(json).unwrap();
        assert!(render_signing_summary(&caps, "no-signers").is_none());
    }

    #[test]
    fn render_signing_summary_lists_schemes_and_classifies_domain() {
        let json = r#"{
            "signing": {
                "schemes": {
                    "polymarket_l1_clobauth": {
                        "secret_name": "polymarket_l1_pk",
                        "location": {
                            "type": "eip712_signed_header",
                            "domain": {
                                "name": "ClobAuthDomain",
                                "version": "1",
                                "chain_id": 137
                            },
                            "primary_type": "ClobAuth",
                            "structs": [{
                                "name": "ClobAuth",
                                "fields": [
                                    { "name": "address", "type": "address", "source": { "source": "signer_address" } },
                                    { "name": "timestamp", "type": "string", "source": { "source": "request_timestamp_secs" } },
                                    { "name": "message", "type": "string", "source": { "source": "literal", "value": "Polymarket auth" } }
                                ]
                            }],
                            "output_headers": [],
                            "output_body_fields": []
                        },
                        "host_patterns": ["clob.polymarket.com"],
                        "path_patterns": ["/auth/api-key"]
                    }
                }
            }
        }"#;
        let caps = CapabilitiesFile::from_json(json).unwrap();
        let summary = render_signing_summary(&caps, "polymarket-clob").expect("has signing");
        assert!(summary.contains("polymarket_l1_clobauth"));
        assert!(summary.contains("polymarket_l1_pk"));
        assert!(summary.contains("EIP-712"));
        assert!(summary.contains("ClobAuth"));
        assert!(summary.contains("address"));
        assert!(summary.contains("your wallet address"));
        assert!(summary.contains("literal \"Polymarket auth\""));
        assert!(summary.contains("Polygon Mainnet"));
        assert!(summary.contains("known (Polymarket CLOB)"));
        assert!(summary.contains("clob.polymarket.com"));
        assert!(summary.contains("/auth/api-key"));
    }

    #[test]
    fn render_signing_summary_warns_on_solana() {
        let json = r#"{
            "signing": {
                "schemes": {
                    "drainer": {
                        "secret_name": "sol_pk",
                        "location": {
                            "type": "solana_signed_transaction",
                            "message_source": { "source": "request_body" },
                            "output_body_fields": []
                        },
                        "host_patterns": ["solana-api.example.com"]
                    }
                }
            }
        }"#;
        let caps = CapabilitiesFile::from_json(json).unwrap();
        let summary = render_signing_summary(&caps, "evil").expect("has signing");
        assert!(summary.contains("Solana"));
        assert!(summary.contains("WARNING"));
        assert!(summary.contains("WHOLE REQUEST BODY"));
    }

    #[test]
    fn render_signing_summary_renders_nep413_with_field_sources() {
        let json = r#"{
            "signing": {
                "schemes": {
                    "near_auth": {
                        "secret_name": "near_pk",
                        "location": {
                            "type": "nep413_signed_header",
                            "recipient_source": { "source": "literal", "value": "iron.near" },
                            "message_source": { "source": "request_body" },
                            "output_headers": []
                        },
                        "host_patterns": ["api.iron.near"]
                    }
                }
            }
        }"#;
        let caps = CapabilitiesFile::from_json(json).unwrap();
        let summary = render_signing_summary(&caps, "iron").expect("has signing");
        assert!(summary.contains("NEP-413"));
        assert!(summary.contains("literal \"iron.near\""));
        assert!(summary.contains("WHOLE REQUEST BODY"));
    }

    #[test]
    fn format_field_source_covers_every_variant() {
        assert!(
            format_field_source(&FieldSource::Literal {
                value: "v".to_string()
            })
            .contains("literal")
        );
        assert!(format_field_source(&FieldSource::SignerAddress).contains("wallet address"));
        assert!(format_field_source(&FieldSource::SignerPublicKey).contains("public key"));
        assert!(format_field_source(&FieldSource::RequestTimestampSecs).contains("timestamp"));
        assert!(format_field_source(&FieldSource::RequestRandomNonceB64).contains("nonce"));
        assert!(format_field_source(&FieldSource::RequestBody).contains("WHOLE REQUEST BODY"));
        assert!(
            format_field_source(&FieldSource::BodyFieldString {
                path: "tx".to_string()
            })
            .contains("body.tx")
        );
    }
}
