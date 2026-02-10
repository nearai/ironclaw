//! Transaction analysis and policy engine for NEAR key operations.
//!
//! Every transaction is decomposed into a `TransactionAnalysis` before any
//! signing happens. The policy engine then evaluates the analysis against
//! a configurable ruleset. Most restrictive rule always wins.
//!
//! # Pipeline
//!
//! ```text
//! Transaction -> analyze_transaction() -> TransactionAnalysis
//!                                              |
//!              PolicyConfig.evaluate() <-------+
//!                     |
//!          PolicyDecision { AutoApprove | RequireApproval | Deny }
//! ```

use serde::{Deserialize, Serialize};

use crate::keys::transaction::{Action, ONE_NEAR};
use crate::keys::types::{AccessKeyPermission, format_yocto};

/// Risk level for a single action within a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskLevel::Low => write!(f, "LOW"),
            RiskLevel::Medium => write!(f, "MEDIUM"),
            RiskLevel::High => write!(f, "HIGH"),
            RiskLevel::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// Category of a transaction action for policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionCategory {
    Transfer,
    FunctionCall,
    Stake,
    AddKey { is_full_access: bool },
    DeleteKey,
    DeployContract,
    CreateAccount,
    DeleteAccount,
}

/// Analysis of a single action within a transaction.
#[derive(Debug, Clone)]
pub struct ActionAnalysis {
    pub category: ActionCategory,
    pub value_yocto: u128,
    pub receiver: String,
    pub method: Option<String>,
    pub description: String,
    pub risk_level: RiskLevel,
}

/// Complete analysis of a transaction.
#[derive(Debug, Clone)]
pub struct TransactionAnalysis {
    pub actions: Vec<ActionAnalysis>,
    pub total_value_yocto: u128,
    pub receivers: Vec<String>,
    pub uses_full_access_key: bool,
    pub summary: String,
}

/// Analyze a transaction's actions for policy evaluation.
pub fn analyze_transaction(
    receiver_id: &str,
    actions: &[Action],
    key_permission: &AccessKeyPermission,
    policy: &PolicyConfig,
) -> TransactionAnalysis {
    let uses_full_access_key = matches!(key_permission, AccessKeyPermission::FullAccess);
    let mut action_analyses = Vec::new();
    let mut total_value = 0u128;

    for action in actions {
        let analysis = analyze_action(action, receiver_id, policy);
        total_value = total_value.saturating_add(analysis.value_yocto);
        action_analyses.push(analysis);
    }

    let receivers: Vec<String> = action_analyses
        .iter()
        .map(|a| a.receiver.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let summary = build_summary(&action_analyses, total_value);

    TransactionAnalysis {
        actions: action_analyses,
        total_value_yocto: total_value,
        receivers,
        uses_full_access_key,
        summary,
    }
}

fn analyze_action(action: &Action, receiver_id: &str, policy: &PolicyConfig) -> ActionAnalysis {
    match action {
        Action::Transfer(t) => {
            let is_whitelisted = policy.transfer_whitelist.contains(&receiver_id.to_string());
            let risk = if t.deposit == 0 || (t.deposit < ONE_NEAR && is_whitelisted) {
                RiskLevel::Low
            } else if t.deposit < policy.transfer_whitelist_max_yocto && is_whitelisted {
                RiskLevel::Medium
            } else {
                RiskLevel::High
            };

            ActionAnalysis {
                category: ActionCategory::Transfer,
                value_yocto: t.deposit,
                receiver: receiver_id.to_string(),
                method: None,
                description: format!("Transfer {} to {}", format_yocto(t.deposit), receiver_id),
                risk_level: risk,
            }
        }

        Action::FunctionCall(fc) => {
            let has_matching_rule = policy
                .function_call_rules
                .iter()
                .any(|r| r.receiver_id == receiver_id && fc.deposit <= r.max_deposit_yocto);

            let risk = if fc.deposit == 0 && has_matching_rule {
                RiskLevel::Low
            } else if fc.deposit == 0 || has_matching_rule {
                RiskLevel::Medium
            } else {
                RiskLevel::High
            };

            ActionAnalysis {
                category: ActionCategory::FunctionCall,
                value_yocto: fc.deposit,
                receiver: receiver_id.to_string(),
                method: Some(fc.method_name.clone()),
                description: format!(
                    "FunctionCall {}::{}{}",
                    receiver_id,
                    fc.method_name,
                    if fc.deposit > 0 {
                        format!(" ({})", format_yocto(fc.deposit))
                    } else {
                        String::new()
                    }
                ),
                risk_level: risk,
            }
        }

        Action::Stake(s) => {
            let risk = if policy
                .stake_validator_whitelist
                .contains(&receiver_id.to_string())
                && s.stake <= policy.stake_auto_approve_max_yocto
            {
                RiskLevel::Medium
            } else {
                RiskLevel::High
            };

            ActionAnalysis {
                category: ActionCategory::Stake,
                value_yocto: s.stake,
                receiver: receiver_id.to_string(),
                method: None,
                description: format!("Stake {} with {}", format_yocto(s.stake), receiver_id),
                risk_level: risk,
            }
        }

        Action::AddKey(ak) => {
            let is_full_access = borsh_permission_is_full_access(&ak.access_key.permission);
            ActionAnalysis {
                category: ActionCategory::AddKey { is_full_access },
                value_yocto: 0,
                receiver: receiver_id.to_string(),
                method: None,
                description: if is_full_access {
                    format!("AddKey (FullAccess) to {}", receiver_id)
                } else {
                    format!("AddKey (FunctionCall) to {}", receiver_id)
                },
                risk_level: if is_full_access {
                    RiskLevel::Critical
                } else {
                    RiskLevel::High
                },
            }
        }

        Action::DeleteKey(_) => ActionAnalysis {
            category: ActionCategory::DeleteKey,
            value_yocto: 0,
            receiver: receiver_id.to_string(),
            method: None,
            description: format!("DeleteKey on {}", receiver_id),
            risk_level: RiskLevel::High,
        },

        Action::DeployContract(_) => ActionAnalysis {
            category: ActionCategory::DeployContract,
            value_yocto: 0,
            receiver: receiver_id.to_string(),
            method: None,
            description: format!("DeployContract to {}", receiver_id),
            risk_level: RiskLevel::Critical,
        },

        Action::CreateAccount => ActionAnalysis {
            category: ActionCategory::CreateAccount,
            value_yocto: 0,
            receiver: receiver_id.to_string(),
            method: None,
            description: format!("CreateAccount {}", receiver_id),
            risk_level: RiskLevel::Medium,
        },

        Action::DeleteAccount(_) => ActionAnalysis {
            category: ActionCategory::DeleteAccount,
            value_yocto: 0,
            receiver: receiver_id.to_string(),
            method: None,
            description: format!("DeleteAccount {}", receiver_id),
            risk_level: RiskLevel::Critical,
        },
    }
}

fn borsh_permission_is_full_access(
    perm: &crate::keys::transaction::AccessKeyPermissionBorsh,
) -> bool {
    matches!(
        perm,
        crate::keys::transaction::AccessKeyPermissionBorsh::FullAccess
    )
}

fn build_summary(actions: &[ActionAnalysis], total_value: u128) -> String {
    let mut lines = Vec::new();
    for (i, a) in actions.iter().enumerate() {
        lines.push(format!("  {}. {} [{}]", i + 1, a.description, a.risk_level));
    }
    if total_value > 0 {
        lines.push(format!("  Total value: {}", format_yocto(total_value)));
    }
    lines.join("\n")
}

/// Policy decision after evaluating a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Transaction can proceed without user interaction.
    AutoApprove,
    /// User must approve before signing.
    RequireApproval { reasons: Vec<String> },
    /// Transaction is denied by policy (not even user can override).
    Deny { reason: String },
}

/// Configurable policy rules for transaction approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    // Transfer rules
    pub transfer_auto_approve_max_yocto: u128,
    pub transfer_whitelist_max_yocto: u128,
    pub transfer_whitelist: Vec<String>,

    // Function call rules
    pub function_call_rules: Vec<FunctionCallRule>,

    // Staking rules
    pub stake_validator_whitelist: Vec<String>,
    pub stake_auto_approve_max_yocto: u128,

    // Key management rules
    pub allow_add_scoped_keys_to: Vec<String>,

    // Chain signature rules
    pub chain_sig_rules: Vec<ChainSigRule>,

    // Global limits
    pub daily_spend_limit_yocto: Option<u128>,
    pub per_tx_auto_approve_max_yocto: u128,

    // Blanket denials
    pub deny_full_access_operations: bool,
    pub deny_delete_account: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            transfer_auto_approve_max_yocto: 0,
            transfer_whitelist_max_yocto: ONE_NEAR,
            transfer_whitelist: Vec::new(),
            function_call_rules: Vec::new(),
            stake_validator_whitelist: Vec::new(),
            stake_auto_approve_max_yocto: 0,
            allow_add_scoped_keys_to: Vec::new(),
            chain_sig_rules: Vec::new(),
            daily_spend_limit_yocto: None,
            per_tx_auto_approve_max_yocto: 0,
            deny_full_access_operations: false,
            deny_delete_account: true,
        }
    }
}

/// A function call rule for policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallRule {
    pub receiver_id: String,
    /// Empty = all methods on this contract.
    pub allowed_methods: Vec<String>,
    pub max_deposit_yocto: u128,
    pub max_gas: Option<u64>,
    pub auto_approve: bool,
}

/// Signature domain for chain signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignatureDomain {
    Secp256k1 = 0,
    Ed25519 = 1,
}

/// A chain signature rule for policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainSigRule {
    pub allowed_paths: Vec<String>,
    pub allowed_domains: Vec<SignatureDomain>,
    pub max_payload_bytes: usize,
    pub auto_approve: bool,
}

/// Analysis specific to chain signature requests.
#[derive(Debug, Clone)]
pub struct ChainSigAnalysis {
    pub derivation_path: String,
    pub domain: SignatureDomain,
    pub target_chain: Option<String>,
    pub payload_size: usize,
    pub risk_level: RiskLevel,
}

impl PolicyConfig {
    /// Evaluate a transaction analysis against this policy.
    ///
    /// Returns the most restrictive decision across all actions.
    pub fn evaluate(
        &self,
        analysis: &TransactionAnalysis,
        key_permission: &AccessKeyPermission,
        daily_spend: u128,
    ) -> PolicyDecision {
        let mut reasons = Vec::new();

        // Blanket denials first
        if self.deny_full_access_operations && analysis.uses_full_access_key {
            return PolicyDecision::Deny {
                reason: "full-access key operations are denied by policy".to_string(),
            };
        }

        for action in &analysis.actions {
            if self.deny_delete_account && matches!(action.category, ActionCategory::DeleteAccount)
            {
                return PolicyDecision::Deny {
                    reason: "account deletion is denied by policy".to_string(),
                };
            }
        }

        // Daily spend limit
        if let Some(limit) = self.daily_spend_limit_yocto {
            if daily_spend.saturating_add(analysis.total_value_yocto) > limit {
                reasons.push(format!(
                    "daily spend limit exceeded: {} + {} > {}",
                    format_yocto(daily_spend),
                    format_yocto(analysis.total_value_yocto),
                    format_yocto(limit)
                ));
            }
        }

        // Per-transaction limit
        if analysis.total_value_yocto > self.per_tx_auto_approve_max_yocto
            && self.per_tx_auto_approve_max_yocto > 0
        {
            reasons.push(format!(
                "transaction value {} exceeds per-tx auto-approve limit {}",
                format_yocto(analysis.total_value_yocto),
                format_yocto(self.per_tx_auto_approve_max_yocto)
            ));
        }

        // Per-action evaluation
        for action in &analysis.actions {
            if let Some(reason) = self.evaluate_action(action, key_permission) {
                reasons.push(reason);
            }
        }

        if reasons.is_empty() {
            PolicyDecision::AutoApprove
        } else {
            PolicyDecision::RequireApproval { reasons }
        }
    }

    /// Evaluate a chain signature request.
    pub fn evaluate_chain_sig(
        &self,
        chain_sig: &ChainSigAnalysis,
        daily_spend: u128,
    ) -> PolicyDecision {
        let mut reasons = Vec::new();

        // Check daily limit (chain sigs don't have a value, but check anyway)
        if let Some(limit) = self.daily_spend_limit_yocto {
            if daily_spend > limit {
                reasons.push("daily spend limit exceeded".to_string());
            }
        }

        // Find matching chain sig rule
        let matching_rule = self.chain_sig_rules.iter().find(|rule| {
            rule.allowed_domains.contains(&chain_sig.domain)
                && chain_sig.payload_size <= rule.max_payload_bytes
                && rule
                    .allowed_paths
                    .iter()
                    .any(|pattern| glob_matches(pattern, &chain_sig.derivation_path))
        });

        match matching_rule {
            Some(rule) if rule.auto_approve => PolicyDecision::AutoApprove,
            Some(_) => {
                reasons.push(format!(
                    "chain signature for path '{}' requires approval",
                    chain_sig.derivation_path
                ));
                PolicyDecision::RequireApproval { reasons }
            }
            None => {
                reasons.push(format!(
                    "no matching chain signature rule for path '{}'",
                    chain_sig.derivation_path
                ));
                PolicyDecision::RequireApproval { reasons }
            }
        }
    }

    fn evaluate_action(
        &self,
        action: &ActionAnalysis,
        key_permission: &AccessKeyPermission,
    ) -> Option<String> {
        match &action.category {
            ActionCategory::Transfer => {
                // Auto-approve to whitelisted accounts under threshold
                if self.transfer_whitelist.contains(&action.receiver)
                    && action.value_yocto <= self.transfer_whitelist_max_yocto
                {
                    return None;
                }
                // Auto-approve small transfers to anyone
                if action.value_yocto <= self.transfer_auto_approve_max_yocto {
                    return None;
                }
                Some(format!(
                    "transfer {} to {} exceeds auto-approve threshold",
                    format_yocto(action.value_yocto),
                    action.receiver
                ))
            }

            ActionCategory::FunctionCall => {
                // Check if key is already scoped to this receiver with zero deposit
                if let AccessKeyPermission::FunctionCall {
                    receiver_id,
                    method_names,
                    ..
                } = key_permission
                {
                    if receiver_id == &action.receiver
                        && action.value_yocto == 0
                        && (method_names.is_empty()
                            || action
                                .method
                                .as_ref()
                                .map(|m| method_names.contains(m))
                                .unwrap_or(false))
                    {
                        return None;
                    }
                }

                // Check function call rules
                if let Some(method) = &action.method {
                    for rule in &self.function_call_rules {
                        if rule.receiver_id == action.receiver
                            && (rule.allowed_methods.is_empty()
                                || rule.allowed_methods.contains(method))
                            && action.value_yocto <= rule.max_deposit_yocto
                            && rule.auto_approve
                        {
                            return None;
                        }
                    }
                }

                Some(format!(
                    "function call {} requires approval",
                    action.description
                ))
            }

            ActionCategory::Stake => {
                if self.stake_validator_whitelist.contains(&action.receiver)
                    && action.value_yocto <= self.stake_auto_approve_max_yocto
                {
                    return None;
                }
                Some(format!("stake {} requires approval", action.description))
            }

            ActionCategory::AddKey { is_full_access } => {
                if *is_full_access {
                    Some("adding full-access key requires approval".to_string())
                } else {
                    Some("adding function-call key requires approval".to_string())
                }
            }

            ActionCategory::DeleteKey
            | ActionCategory::DeployContract
            | ActionCategory::CreateAccount
            | ActionCategory::DeleteAccount => {
                Some(format!("{} requires approval", action.description))
            }
        }
    }
}

/// Simple glob matching: supports `*` as wildcard for any suffix.
fn glob_matches(pattern: &str, value: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        value.starts_with(prefix)
    } else {
        pattern == value
    }
}

/// Infer target chain from a derivation path.
pub fn infer_target_chain(derivation_path: &str) -> Option<String> {
    let lower = derivation_path.to_lowercase();
    if lower.starts_with("ethereum") || lower.starts_with("eth") {
        Some("Ethereum".to_string())
    } else if lower.starts_with("bitcoin") || lower.starts_with("btc") {
        Some("Bitcoin".to_string())
    } else if lower.starts_with("near") {
        Some("NEAR".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::keys::policy::{
        ChainSigAnalysis, ChainSigRule, FunctionCallRule, PolicyConfig, PolicyDecision, RiskLevel,
        SignatureDomain, analyze_transaction, glob_matches, infer_target_chain,
    };
    use crate::keys::transaction::{Action, FunctionCall, ONE_NEAR, TGAS, Transfer};
    use crate::keys::types::AccessKeyPermission;

    fn default_policy() -> PolicyConfig {
        PolicyConfig::default()
    }

    fn permissive_policy() -> PolicyConfig {
        PolicyConfig {
            transfer_auto_approve_max_yocto: ONE_NEAR,
            transfer_whitelist_max_yocto: 10 * ONE_NEAR,
            transfer_whitelist: vec!["bob.near".to_string()],
            function_call_rules: vec![FunctionCallRule {
                receiver_id: "intents.near".to_string(),
                allowed_methods: vec!["execute_intents".to_string()],
                max_deposit_yocto: 0,
                max_gas: None,
                auto_approve: true,
            }],
            per_tx_auto_approve_max_yocto: 5 * ONE_NEAR,
            daily_spend_limit_yocto: Some(50 * ONE_NEAR),
            ..default_policy()
        }
    }

    // -- Transfer tests --

    #[test]
    fn test_transfer_below_auto_approve() {
        let policy = permissive_policy();
        let actions = vec![Action::Transfer(Transfer {
            deposit: ONE_NEAR / 2,
        })];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("someone.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        assert_eq!(decision, PolicyDecision::AutoApprove);
    }

    #[test]
    fn test_transfer_above_threshold_requires_approval() {
        let policy = permissive_policy();
        let actions = vec![Action::Transfer(Transfer {
            deposit: 2 * ONE_NEAR,
        })];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("unknown.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        assert!(matches!(decision, PolicyDecision::RequireApproval { .. }));
    }

    #[test]
    fn test_transfer_to_whitelisted_account() {
        let policy = permissive_policy();
        let actions = vec![Action::Transfer(Transfer {
            deposit: 5 * ONE_NEAR,
        })];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("bob.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        assert_eq!(decision, PolicyDecision::AutoApprove);
    }

    #[test]
    fn test_transfer_to_whitelisted_above_whitelist_limit() {
        let policy = permissive_policy();
        let actions = vec![Action::Transfer(Transfer {
            deposit: 15 * ONE_NEAR,
        })];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("bob.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        // 15 NEAR > whitelist max (10 NEAR), and > per_tx limit (5 NEAR)
        assert!(matches!(decision, PolicyDecision::RequireApproval { .. }));
    }

    // -- Function call tests --

    #[test]
    fn test_function_call_matching_rule_auto_approve() {
        let policy = permissive_policy();
        let actions = vec![Action::FunctionCall(FunctionCall {
            method_name: "execute_intents".to_string(),
            args: vec![],
            gas: 30 * TGAS,
            deposit: 0,
        })];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("intents.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        assert_eq!(decision, PolicyDecision::AutoApprove);
    }

    #[test]
    fn test_function_call_scoped_key_auto_approve() {
        let policy = default_policy();
        let actions = vec![Action::FunctionCall(FunctionCall {
            method_name: "deposit".to_string(),
            args: vec![],
            gas: 30 * TGAS,
            deposit: 0,
        })];
        let perm = AccessKeyPermission::FunctionCall {
            allowance: None,
            receiver_id: "contract.near".to_string(),
            method_names: vec!["deposit".to_string()],
        };
        let analysis = analyze_transaction("contract.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        assert_eq!(decision, PolicyDecision::AutoApprove);
    }

    #[test]
    fn test_function_call_no_rule_requires_approval() {
        let policy = default_policy();
        let actions = vec![Action::FunctionCall(FunctionCall {
            method_name: "dangerous_method".to_string(),
            args: vec![],
            gas: 30 * TGAS,
            deposit: ONE_NEAR,
        })];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("unknown.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        assert!(matches!(decision, PolicyDecision::RequireApproval { .. }));
    }

    // -- Blanket denial tests --

    #[test]
    fn test_deny_full_access_operations() {
        let policy = PolicyConfig {
            deny_full_access_operations: true,
            ..default_policy()
        };
        let actions = vec![Action::Transfer(Transfer { deposit: 0 })];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("bob.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn test_deny_delete_account() {
        let policy = PolicyConfig {
            deny_delete_account: true,
            ..default_policy()
        };
        let actions = vec![Action::DeleteAccount(
            crate::keys::transaction::DeleteAccount {
                beneficiary_id: crate::keys::types::NearAccountId::new("bob.near").unwrap(),
            },
        )];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("alice.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    // -- Daily spend limit tests --

    #[test]
    fn test_daily_spend_limit_under() {
        let policy = permissive_policy();
        let actions = vec![Action::Transfer(Transfer {
            deposit: ONE_NEAR / 2,
        })];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("bob.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 10 * ONE_NEAR);
        assert_eq!(decision, PolicyDecision::AutoApprove);
    }

    #[test]
    fn test_daily_spend_limit_exceeded() {
        let policy = permissive_policy();
        let actions = vec![Action::Transfer(Transfer {
            deposit: ONE_NEAR / 2,
        })];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("bob.near", &actions, &perm, &policy);
        // Current daily spend is 50 NEAR (at limit), adding 0.5 NEAR puts us over
        let decision = policy.evaluate(&analysis, &perm, 50 * ONE_NEAR);
        assert!(matches!(decision, PolicyDecision::RequireApproval { .. }));
    }

    // -- Per-transaction limit tests --

    #[test]
    fn test_per_tx_limit() {
        let policy = permissive_policy();
        let actions = vec![Action::Transfer(Transfer {
            deposit: 6 * ONE_NEAR,
        })];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("bob.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        // 6 NEAR > per_tx_auto_approve_max (5 NEAR)
        assert!(matches!(decision, PolicyDecision::RequireApproval { .. }));
    }

    // -- Most restrictive wins --

    #[test]
    fn test_mixed_actions_most_restrictive_wins() {
        let policy = permissive_policy();
        // One auto-approvable + one that requires approval
        let actions = vec![
            Action::FunctionCall(FunctionCall {
                method_name: "execute_intents".to_string(),
                args: vec![],
                gas: 30 * TGAS,
                deposit: 0,
            }),
            Action::Transfer(Transfer {
                deposit: 100 * ONE_NEAR,
            }),
        ];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("intents.near", &actions, &perm, &policy);
        let decision = policy.evaluate(&analysis, &perm, 0);
        // Transfer is too large, so the whole tx requires approval
        assert!(matches!(decision, PolicyDecision::RequireApproval { .. }));
    }

    // -- Transaction analysis tests --

    #[test]
    fn test_analysis_total_value() {
        let policy = default_policy();
        let actions = vec![
            Action::Transfer(Transfer {
                deposit: 2 * ONE_NEAR,
            }),
            Action::FunctionCall(FunctionCall {
                method_name: "deposit".to_string(),
                args: vec![],
                gas: 30 * TGAS,
                deposit: ONE_NEAR,
            }),
        ];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("bob.near", &actions, &perm, &policy);
        assert_eq!(analysis.total_value_yocto, 3 * ONE_NEAR);
        assert_eq!(analysis.actions.len(), 2);
    }

    #[test]
    fn test_analysis_risk_levels() {
        let policy = default_policy();
        let actions = vec![
            Action::Transfer(Transfer { deposit: 0 }),
            Action::DeleteAccount(crate::keys::transaction::DeleteAccount {
                beneficiary_id: crate::keys::types::NearAccountId::new("bob.near").unwrap(),
            }),
        ];
        let perm = AccessKeyPermission::FullAccess;
        let analysis = analyze_transaction("alice.near", &actions, &perm, &policy);
        assert_eq!(analysis.actions[0].risk_level, RiskLevel::Low);
        assert_eq!(analysis.actions[1].risk_level, RiskLevel::Critical);
    }

    // -- Chain signature tests --

    #[test]
    fn test_chain_sig_no_rule_requires_approval() {
        let policy = default_policy();
        let chain_sig = ChainSigAnalysis {
            derivation_path: "ethereum-1".to_string(),
            domain: SignatureDomain::Secp256k1,
            target_chain: Some("Ethereum".to_string()),
            payload_size: 256,
            risk_level: RiskLevel::Medium,
        };
        let decision = policy.evaluate_chain_sig(&chain_sig, 0);
        assert!(matches!(decision, PolicyDecision::RequireApproval { .. }));
    }

    #[test]
    fn test_chain_sig_matching_rule_auto_approve() {
        let policy = PolicyConfig {
            chain_sig_rules: vec![ChainSigRule {
                allowed_paths: vec!["ethereum-*".to_string()],
                allowed_domains: vec![SignatureDomain::Secp256k1],
                max_payload_bytes: 1024,
                auto_approve: true,
            }],
            ..default_policy()
        };
        let chain_sig = ChainSigAnalysis {
            derivation_path: "ethereum-1".to_string(),
            domain: SignatureDomain::Secp256k1,
            target_chain: Some("Ethereum".to_string()),
            payload_size: 256,
            risk_level: RiskLevel::Medium,
        };
        let decision = policy.evaluate_chain_sig(&chain_sig, 0);
        assert_eq!(decision, PolicyDecision::AutoApprove);
    }

    // -- Glob matching tests --

    #[test]
    fn test_glob_matches() {
        assert!(glob_matches("ethereum-*", "ethereum-1"));
        assert!(glob_matches("ethereum-*", "ethereum-mainnet"));
        assert!(!glob_matches("ethereum-*", "bitcoin-0"));
        assert!(glob_matches("exact-match", "exact-match"));
        assert!(!glob_matches("exact-match", "other"));
    }

    // -- Infer target chain --

    #[test]
    fn test_infer_target_chain() {
        assert_eq!(
            infer_target_chain("ethereum-1"),
            Some("Ethereum".to_string())
        );
        assert_eq!(
            infer_target_chain("bitcoin/0/0"),
            Some("Bitcoin".to_string())
        );
        assert_eq!(infer_target_chain("unknown-path"), None);
    }
}
