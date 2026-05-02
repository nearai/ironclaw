//! Paid research planning for the Intents Trading Agent.
//!
//! This module models the "DripStack" pattern for IronClaw: an agent can
//! discover premium research sources, budget a query, attribute which sources
//! would be used in an answer, and prepare payment rails before any trading
//! intent is built. It intentionally does not fetch paywalled content or sign
//! payments. The output is a deterministic plan the skill can persist, show in
//! the widget, and hand to a wallet/payment client after user approval.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct PaidResearchPlanInput {
    pub query: String,
    #[serde(default)]
    pub pair: Option<String>,
    #[serde(default = "default_budget_usd")]
    pub budget_usd: f64,
    #[serde(default = "default_max_sources")]
    pub max_sources: usize,
    #[serde(default = "default_min_relevance")]
    pub min_relevance: f64,
    #[serde(default)]
    pub max_source_age_days: Option<u32>,
    #[serde(default = "default_spending_mode")]
    pub spending_mode: String,
    #[serde(default = "default_near_funding_asset")]
    pub near_funding_asset: String,
    #[serde(default)]
    pub required_tags: Vec<String>,
    #[serde(default)]
    pub blocked_publishers: Vec<String>,
    #[serde(default)]
    pub sources: Vec<PaidResearchSourceInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PaidResearchSourceInput {
    pub id: String,
    pub title: String,
    pub author: String,
    #[serde(default)]
    pub publisher: Option<String>,
    pub url: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub price_usd: Option<f64>,
    #[serde(default)]
    pub relevance: Option<f64>,
    #[serde(default)]
    pub trust_score: Option<f64>,
    #[serde(default)]
    pub freshness_score: Option<f64>,
    #[serde(default)]
    pub age_days: Option<u32>,
    #[serde(default)]
    pub payment: Option<PaidResearchPaymentInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PaidResearchPaymentInput {
    #[serde(default = "default_payment_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub recipient: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PaidResearchPlan {
    pub schema_version: &'static str,
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair: Option<String>,
    pub spending_mode: String,
    pub budget_usd: f64,
    pub allocated_usd: f64,
    pub unspent_usd: f64,
    pub selected_sources: Vec<SelectedPaidResearchSource>,
    pub rejected_sources: Vec<RejectedPaidResearchSource>,
    pub payment_rails: Vec<PaidResearchRailSummary>,
    pub near_funding_routes: Vec<NearFundingRoute>,
    pub policy_gates: Vec<PaidResearchPolicyGate>,
    pub ready_for_paid_fetch: bool,
    pub ready_for_trade_research: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct ScoredSource {
    source: PaidResearchSourceInput,
    relevance: f64,
    trust_score: f64,
    freshness_score: f64,
    tag_score: f64,
    selection_score: f64,
    price_usd: f64,
}

#[derive(Debug, Serialize)]
pub struct SelectedPaidResearchSource {
    pub id: String,
    pub title: String,
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    pub url: String,
    pub tags: Vec<String>,
    pub price_usd: f64,
    pub relevance: f64,
    pub trust_score: f64,
    pub freshness_score: f64,
    pub selection_score: f64,
    pub evidence_weight: f64,
    pub payment: PlannedResearchPayment,
    pub attribution: ResearchAttribution,
}

#[derive(Debug, Serialize)]
pub struct PlannedResearchPayment {
    pub protocol: String,
    pub network: String,
    pub asset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub amount_usd: f64,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ResearchAttribution {
    pub credit_id: String,
    pub payable: bool,
    pub receipt_required: bool,
    pub answer_usage: String,
}

#[derive(Debug, Serialize)]
pub struct RejectedPaidResearchSource {
    pub id: String,
    pub title: String,
    pub reason: String,
    pub price_usd: f64,
    pub relevance: f64,
    pub selection_score: f64,
}

#[derive(Debug, Serialize)]
pub struct PaidResearchRailSummary {
    pub protocol: String,
    pub count: usize,
    pub allocated_usd: f64,
}

#[derive(Debug, Serialize)]
pub struct NearFundingRoute {
    pub target_protocol: String,
    pub target_network: String,
    pub target_asset: String,
    pub amount_usd: f64,
    pub via: String,
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct PaidResearchPolicyGate {
    pub name: String,
    pub status: String,
    pub detail: String,
}

pub fn plan(input: PaidResearchPlanInput) -> Result<PaidResearchPlan, String> {
    if input.query.trim().is_empty() {
        return Err("query is required for plan_paid_research".to_string());
    }

    let budget_usd = clean_nonnegative(input.budget_usd);
    let max_sources = input.max_sources.max(1);
    let min_relevance = clamp01(input.min_relevance);
    let blocked = normalized_set(&input.blocked_publishers);
    let required_tags = normalized_set(&input.required_tags);

    let mut warnings = Vec::new();
    if input.sources.is_empty() {
        warnings.push("No candidate research sources were provided.".to_string());
    }
    if budget_usd == 0.0 {
        warnings.push("Budget is zero; only free sources can be selected.".to_string());
    }

    let mut rejected_sources = Vec::new();
    let mut candidates = Vec::new();
    for source in input.sources {
        let scored = score_source(source, &input.query, &required_tags, budget_usd);
        if blocked.contains(&normalize(&scored.source.author))
            || scored
                .source
                .publisher
                .as_ref()
                .map(|publisher| blocked.contains(&normalize(publisher)))
                .unwrap_or(false)
        {
            rejected_sources.push(rejected(&scored, "blocked-publisher"));
            continue;
        }
        if !required_tags.is_empty() && !source_has_required_tags(&scored.source, &required_tags) {
            rejected_sources.push(rejected(&scored, "missing-required-tags"));
            continue;
        }
        if let (Some(max_age), Some(age)) = (input.max_source_age_days, scored.source.age_days) {
            if age > max_age {
                rejected_sources.push(rejected(&scored, "stale-source"));
                continue;
            }
        }
        if scored.relevance < min_relevance {
            rejected_sources.push(rejected(&scored, "below-min-relevance"));
            continue;
        }
        candidates.push(scored);
    }

    candidates.sort_by(|a, b| {
        b.selection_score
            .partial_cmp(&a.selection_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                a.price_usd
                    .partial_cmp(&b.price_usd)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| a.source.id.cmp(&b.source.id))
    });

    let mut selected_scored = Vec::new();
    let mut remaining_budget = budget_usd;
    for candidate in candidates {
        if selected_scored.len() >= max_sources {
            rejected_sources.push(rejected(&candidate, "max-sources-reached"));
            continue;
        }
        if candidate.price_usd > remaining_budget + f64::EPSILON {
            rejected_sources.push(rejected(&candidate, "over-budget"));
            continue;
        }
        remaining_budget = (remaining_budget - candidate.price_usd).max(0.0);
        selected_scored.push(candidate);
    }

    let allocated_usd: f64 = selected_scored.iter().map(|s| s.price_usd).sum();
    let score_sum: f64 = selected_scored
        .iter()
        .map(|s| s.selection_score.max(0.0))
        .sum();
    let mut rail_totals: BTreeMap<String, (usize, f64)> = BTreeMap::new();
    let mut funding_totals: BTreeMap<(String, String, String), f64> = BTreeMap::new();

    let selected_sources: Vec<SelectedPaidResearchSource> = selected_scored
        .into_iter()
        .map(|scored| {
            let payment = planned_payment(&scored, &input.spending_mode);
            let rail_entry = rail_totals
                .entry(payment.protocol.clone())
                .or_insert((0, 0.0));
            rail_entry.0 += 1;
            rail_entry.1 += payment.amount_usd;

            if payment.amount_usd > 0.0 && payment.protocol != "near-intents" {
                let key = (
                    payment.protocol.clone(),
                    payment.network.clone(),
                    payment.asset.clone(),
                );
                *funding_totals.entry(key).or_insert(0.0) += payment.amount_usd;
            }

            let evidence_weight = if score_sum > 0.0 {
                round4(scored.selection_score.max(0.0) / score_sum)
            } else {
                0.0
            };
            let payable = payment.amount_usd > 0.0;
            SelectedPaidResearchSource {
                id: scored.source.id.clone(),
                title: scored.source.title.clone(),
                author: scored.source.author.clone(),
                publisher: scored.source.publisher.clone(),
                url: scored.source.url.clone(),
                tags: scored.source.tags.clone(),
                price_usd: round4(scored.price_usd),
                relevance: round4(scored.relevance),
                trust_score: round4(scored.trust_score),
                freshness_score: round4(scored.freshness_score),
                selection_score: round4(scored.selection_score),
                evidence_weight,
                payment,
                attribution: ResearchAttribution {
                    credit_id: format!("research-credit/{}", scored.source.id),
                    payable,
                    receipt_required: payable,
                    answer_usage: if payable {
                        "cite only after payment receipt, then credit this source in the answer"
                            .to_string()
                    } else {
                        "free/public source can be cited with URL attribution".to_string()
                    },
                },
            }
        })
        .collect();

    let payment_rails = rail_totals
        .into_iter()
        .map(
            |(protocol, (count, allocated_usd))| PaidResearchRailSummary {
                protocol,
                count,
                allocated_usd: round4(allocated_usd),
            },
        )
        .collect();

    let near_funding_routes = funding_totals
        .into_iter()
        .map(
            |((target_protocol, target_network, target_asset), amount_usd)| NearFundingRoute {
                target_protocol: target_protocol.clone(),
                target_network,
                target_asset,
                amount_usd: round4(amount_usd),
                via: "near-intents".to_string(),
                note: format!(
                    "Use a NEAR Intents funding route from {} into the {} rail wallet before the paid fetch; this does not itself authorize the content payment.",
                    input.near_funding_asset, target_protocol
                ),
            },
        )
        .collect();

    if selected_sources.is_empty() && !rejected_sources.is_empty() {
        warnings.push(
            "All candidate paid research sources were rejected by budget or gates.".to_string(),
        );
    }

    let paid_selected = selected_sources
        .iter()
        .filter(|source| source.payment.amount_usd > 0.0)
        .count();
    let policy_gates = policy_gates(
        &input.spending_mode,
        budget_usd,
        allocated_usd,
        paid_selected,
    );
    let ready_for_paid_fetch = !selected_sources.is_empty()
        && allocated_usd <= budget_usd + f64::EPSILON
        && policy_gates.iter().all(|gate| gate.status != "fail");
    let ready_for_trade_research = ready_for_paid_fetch
        && selected_sources
            .iter()
            .map(|source| source.evidence_weight)
            .sum::<f64>()
            > 0.0;

    Ok(PaidResearchPlan {
        schema_version: "paid-research-plan/1",
        query: input.query,
        pair: input.pair,
        spending_mode: input.spending_mode,
        budget_usd: round4(budget_usd),
        allocated_usd: round4(allocated_usd),
        unspent_usd: round4((budget_usd - allocated_usd).max(0.0)),
        selected_sources,
        rejected_sources,
        payment_rails,
        near_funding_routes,
        policy_gates,
        ready_for_paid_fetch,
        ready_for_trade_research,
        warnings,
    })
}

fn score_source(
    source: PaidResearchSourceInput,
    query: &str,
    required_tags: &BTreeSet<String>,
    budget_usd: f64,
) -> ScoredSource {
    let text = format!(
        "{} {} {} {}",
        source.title,
        source.author,
        source.summary.as_deref().unwrap_or_default(),
        source.tags.join(" ")
    );
    let lexical = lexical_relevance(query, &text);
    let relevance = source
        .relevance
        .map(clamp01)
        .unwrap_or(lexical)
        .max(lexical * 0.9);
    let trust_score = source.trust_score.map(clamp01).unwrap_or(0.6);
    let freshness_score = source
        .freshness_score
        .map(clamp01)
        .unwrap_or_else(|| age_freshness(source.age_days));
    let tag_score = tag_score(&source, query, required_tags);
    let price_usd = clean_nonnegative(source.price_usd.unwrap_or(0.0));
    let cost_penalty = if budget_usd > 0.0 {
        (price_usd / budget_usd).min(1.0) * 0.08
    } else if price_usd > 0.0 {
        0.25
    } else {
        0.0
    };
    let selection_score =
        (relevance * 0.50) + (trust_score * 0.22) + (freshness_score * 0.18) + (tag_score * 0.10)
            - cost_penalty;

    ScoredSource {
        source,
        relevance,
        trust_score,
        freshness_score,
        tag_score,
        selection_score: selection_score.max(0.0),
        price_usd,
    }
}

fn planned_payment(scored: &ScoredSource, spending_mode: &str) -> PlannedResearchPayment {
    let raw = scored
        .source
        .payment
        .clone()
        .unwrap_or(PaidResearchPaymentInput {
            protocol: if scored.price_usd > 0.0 {
                "manual".to_string()
            } else {
                "free".to_string()
            },
            network: None,
            asset: None,
            recipient: None,
            endpoint: None,
        });
    let protocol = normalize_protocol(&raw.protocol, scored.price_usd);
    let is_free = scored.price_usd == 0.0 || protocol == "free";
    let network = raw
        .network
        .unwrap_or_else(|| default_network_for_protocol(&protocol));
    let asset = raw.asset.unwrap_or_else(|| {
        if is_free {
            "none".to_string()
        } else {
            "USDC".to_string()
        }
    });
    let status = if is_free {
        "free".to_string()
    } else if spending_mode == "autopay" {
        "requires-policy-wallet".to_string()
    } else {
        "requires-user-approval".to_string()
    };

    PlannedResearchPayment {
        protocol,
        network,
        asset,
        recipient: raw.recipient,
        endpoint: raw.endpoint,
        amount_usd: round4(scored.price_usd),
        status,
    }
}

fn policy_gates(
    spending_mode: &str,
    budget_usd: f64,
    allocated_usd: f64,
    paid_selected: usize,
) -> Vec<PaidResearchPolicyGate> {
    let mut gates = vec![
        PaidResearchPolicyGate {
            name: "budget-cap".to_string(),
            status: if allocated_usd <= budget_usd + f64::EPSILON {
                "pass"
            } else {
                "fail"
            }
            .to_string(),
            detail: format!(
                "Allocated ${:.4} of the ${:.4} paid research budget.",
                allocated_usd, budget_usd
            ),
        },
        PaidResearchPolicyGate {
            name: "receipt-before-use".to_string(),
            status: "pass".to_string(),
            detail:
                "Paywalled source text must not be used in an answer until a payment receipt exists."
                    .to_string(),
        },
        PaidResearchPolicyGate {
            name: "citation-attribution".to_string(),
            status: "pass".to_string(),
            detail:
                "Every answer using paid research must include source IDs so writers can be credited."
                    .to_string(),
        },
        PaidResearchPolicyGate {
            name: "trade-gate".to_string(),
            status: "pass".to_string(),
            detail:
                "Paid research can influence the thesis, but backtest and risk gates still control NEAR intent construction."
                    .to_string(),
        },
    ];

    let approval_status = match spending_mode {
        "plan-only" | "quote" => "pass",
        "autopay" if paid_selected == 0 => "pass",
        "autopay" => "warn",
        _ => "warn",
    };
    gates.push(PaidResearchPolicyGate {
        name: "payment-authorization".to_string(),
        status: approval_status.to_string(),
        detail: if spending_mode == "autopay" {
            "Autopay requires a policy wallet with explicit source, amount, and cadence caps."
                .to_string()
        } else {
            "Live source payments remain user-approved; this plan only prepares payable attribution."
                .to_string()
        },
    });

    gates
}

fn rejected(scored: &ScoredSource, reason: &str) -> RejectedPaidResearchSource {
    RejectedPaidResearchSource {
        id: scored.source.id.clone(),
        title: scored.source.title.clone(),
        reason: reason.to_string(),
        price_usd: round4(scored.price_usd),
        relevance: round4(scored.relevance),
        selection_score: round4(scored.selection_score),
    }
}

fn lexical_relevance(query: &str, text: &str) -> f64 {
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return 0.0;
    }
    let text_tokens = tokenize(text);
    if text_tokens.is_empty() {
        return 0.0;
    }
    let matches = query_tokens
        .iter()
        .filter(|token| text_tokens.contains(*token))
        .count();
    (matches as f64 / query_tokens.len() as f64).min(1.0)
}

fn tag_score(
    source: &PaidResearchSourceInput,
    query: &str,
    required_tags: &BTreeSet<String>,
) -> f64 {
    let source_tags = normalized_set(&source.tags);
    if source_tags.is_empty() {
        return 0.0;
    }
    if !required_tags.is_empty() {
        let hits = required_tags
            .iter()
            .filter(|tag| source_tags.contains(*tag))
            .count();
        return hits as f64 / required_tags.len() as f64;
    }
    let query_tokens = tokenize(query);
    let hits = query_tokens
        .iter()
        .filter(|token| source_tags.contains(*token))
        .count();
    (hits as f64 / query_tokens.len().max(1) as f64).min(1.0)
}

fn source_has_required_tags(
    source: &PaidResearchSourceInput,
    required_tags: &BTreeSet<String>,
) -> bool {
    let source_tags = normalized_set(&source.tags);
    required_tags.iter().all(|tag| source_tags.contains(tag))
}

fn age_freshness(age_days: Option<u32>) -> f64 {
    match age_days {
        Some(0..=1) => 1.0,
        Some(2..=7) => 0.85,
        Some(8..=30) => 0.65,
        Some(31..=90) => 0.45,
        Some(_) => 0.25,
        None => 0.55,
    }
}

fn tokenize(value: &str) -> BTreeSet<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(normalize)
        .filter(|token| token.len() > 2 && !is_stopword(token))
        .collect()
}

fn normalized_set(values: &[String]) -> BTreeSet<String> {
    values
        .iter()
        .map(|value| normalize(value))
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_protocol(protocol: &str, price_usd: f64) -> String {
    let protocol = normalize(protocol);
    if price_usd == 0.0 {
        return "free".to_string();
    }
    match protocol.as_str() {
        "near" | "near_intents" | "near-intents" => "near-intents".to_string(),
        "mpp" | "tempo" => "mpp".to_string(),
        "x402" | "base-x402" => "x402".to_string(),
        "subscription" => "subscription".to_string(),
        "manual" => "manual".to_string(),
        "free" => "free".to_string(),
        _ => "manual".to_string(),
    }
}

fn default_network_for_protocol(protocol: &str) -> String {
    match protocol {
        "near-intents" => "near".to_string(),
        "mpp" => "tempo".to_string(),
        "x402" => "base".to_string(),
        "subscription" => "offchain".to_string(),
        "free" => "none".to_string(),
        _ => "manual".to_string(),
    }
}

fn is_stopword(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "and"
            | "for"
            | "with"
            | "into"
            | "from"
            | "that"
            | "this"
            | "what"
            | "when"
            | "near"
            | "price"
            | "trade"
            | "trading"
            | "token"
            | "crypto"
    )
}

fn clean_nonnegative(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

fn clamp01(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn default_budget_usd() -> f64 {
    5.0
}

fn default_max_sources() -> usize {
    4
}

fn default_min_relevance() -> f64 {
    0.35
}

fn default_spending_mode() -> String {
    "plan-only".to_string()
}

fn default_near_funding_asset() -> String {
    "USDC.near".to_string()
}

fn default_payment_protocol() -> String {
    "manual".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: &str, price_usd: f64, protocol: &str, relevance: f64) -> PaidResearchSourceInput {
        PaidResearchSourceInput {
            id: id.to_string(),
            title: format!("{id} NEAR Intents research"),
            author: "Writer".to_string(),
            publisher: Some("Research Desk".to_string()),
            url: format!("https://example.com/{id}"),
            summary: Some("NEAR BTC liquidity, solver quality, and intent route risk.".to_string()),
            tags: vec!["near-intents".to_string(), "btc".to_string()],
            price_usd: Some(price_usd),
            relevance: Some(relevance),
            trust_score: Some(0.8),
            freshness_score: Some(0.9),
            age_days: Some(1),
            payment: Some(PaidResearchPaymentInput {
                protocol: protocol.to_string(),
                network: None,
                asset: Some("USDC".to_string()),
                recipient: Some("0xmerchant".to_string()),
                endpoint: Some(format!("https://example.com/{id}/paid")),
            }),
        }
    }

    #[test]
    fn paid_research_plan_respects_budget_and_routes_external_rails() {
        let plan = plan(PaidResearchPlanInput {
            query: "NEAR BTC solver route risk".to_string(),
            pair: Some("NEAR/BTC".to_string()),
            budget_usd: 0.05,
            max_sources: 3,
            min_relevance: 0.2,
            max_source_age_days: Some(7),
            spending_mode: "quote".to_string(),
            near_funding_asset: "USDC.near".to_string(),
            required_tags: vec!["near-intents".to_string()],
            blocked_publishers: vec![],
            sources: vec![
                source("mpp-source", 0.02, "mpp", 0.9),
                source("x402-source", 0.02, "x402", 0.85),
                source("too-expensive", 0.10, "x402", 0.95),
            ],
        })
        .expect("plan");

        assert_eq!(plan.schema_version, "paid-research-plan/1");
        assert_eq!(plan.selected_sources.len(), 2);
        assert!(plan.allocated_usd <= 0.05);
        assert_eq!(plan.near_funding_routes.len(), 2);
        assert!(plan
            .rejected_sources
            .iter()
            .any(|source| source.reason == "over-budget"));
    }
}
