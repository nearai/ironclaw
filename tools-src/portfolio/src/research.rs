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

// Paid research plans run in the WASM tool layer and use f64 only for
// non-settlement ranking, budgeting, and policy hints. Settlement remains with
// the payment rails; this tolerance avoids machine-epsilon threshold mistakes.
const USD_TOLERANCE: f64 = 1e-6;

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
    #[serde(default = "default_min_trust_score")]
    pub min_trust_score: f64,
    #[serde(default = "default_max_seo_risk_score")]
    pub max_seo_risk_score: f64,
    #[serde(default)]
    pub max_source_age_days: Option<u32>,
    #[serde(default = "default_spending_mode")]
    pub spending_mode: String,
    #[serde(default = "default_near_funding_asset")]
    pub near_funding_asset: String,
    #[serde(default)]
    pub preferred_payment_protocols: Vec<String>,
    #[serde(default = "default_article_price_usd")]
    pub default_article_price_usd: f64,
    #[serde(default)]
    pub agent_wallet: Option<AgentWalletPolicyInput>,
    #[serde(default)]
    pub required_tags: Vec<String>,
    #[serde(default)]
    pub blocked_publishers: Vec<String>,
    #[serde(default)]
    pub sources: Vec<PaidResearchSourceInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
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
    pub seo_risk_score: Option<f64>,
    #[serde(default)]
    pub age_days: Option<u32>,
    #[serde(default)]
    pub payment: Option<PaidResearchPaymentInput>,
    #[serde(default)]
    pub payment_options: Vec<PaidResearchPaymentInput>,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentWalletPolicyInput {
    #[serde(default = "default_agent_wallet_provider")]
    pub provider: String,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default = "default_agent_wallet_network")]
    pub network: String,
    #[serde(default)]
    pub balance_usd: Option<f64>,
    #[serde(default = "default_article_price_usd")]
    pub default_article_price_usd: f64,
    #[serde(default = "default_per_article_cap_usd")]
    pub per_article_cap_usd: f64,
    #[serde(default = "default_daily_wallet_cap_usd")]
    pub daily_cap_usd: f64,
    #[serde(default = "default_max_wallet_balance_usd")]
    pub max_wallet_balance_usd: f64,
    #[serde(default)]
    pub audit_urls: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DripstackBrowseInput {
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub selected_publication_slug: Option<String>,
    #[serde(default)]
    pub selected_post_slug: Option<String>,
    #[serde(default)]
    pub max_results: Option<usize>,
    #[serde(default = "default_article_price_usd")]
    pub default_price_usd: f64,
    #[serde(default)]
    pub publications: Vec<DripstackPublicationInput>,
    #[serde(default)]
    pub posts: Vec<DripstackPostInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DripstackPublicationInput {
    pub slug: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    #[serde(alias = "siteUrl")]
    pub site_url: Option<String>,
    #[serde(default)]
    #[serde(alias = "lastSyncedAt")]
    pub last_synced_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DripstackPostInput {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    #[serde(alias = "publishedAt")]
    pub published_at: Option<String>,
    #[serde(default)]
    pub price_usd: Option<f64>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_policy: Option<AgentWalletPolicy>,
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
    seo_risk_score: f64,
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
    pub seo_risk_score: f64,
    pub selection_score: f64,
    pub evidence_weight: f64,
    pub payment: PlannedResearchPayment,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub payment_options: Vec<PlannedResearchPayment>,
    pub attribution: ResearchAttribution,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct AgentWalletPolicy {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    pub network: String,
    pub balance_usd: f64,
    pub default_article_price_usd: f64,
    pub max_articles_at_default_price: usize,
    pub per_article_cap_usd: f64,
    pub daily_cap_usd: f64,
    pub max_wallet_balance_usd: f64,
    pub safe_to_autopay: bool,
    pub audit_urls: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DripstackBrowsePlan {
    pub schema_version: &'static str,
    pub checkpoint: String,
    pub prompt: String,
    pub matched_publications: Vec<DripstackPublicationMatch>,
    pub post_candidates: Vec<DripstackPostCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_article: Option<DripstackPostCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_source_candidate: Option<PaidResearchSourceInput>,
    pub guardrails: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DripstackPublicationMatch {
    pub rank: usize,
    pub slug: String,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site_url: Option<String>,
    pub relevance: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DripstackPostCandidate {
    pub rank: usize,
    pub publication_slug: String,
    pub slug: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    pub price_usd: f64,
    pub endpoint: String,
}

pub fn plan(input: PaidResearchPlanInput) -> Result<PaidResearchPlan, String> {
    if input.query.trim().is_empty() {
        return Err("query is required for plan_paid_research".to_string());
    }

    let budget_usd = clean_nonnegative(input.budget_usd);
    let max_sources = input.max_sources.max(1);
    let min_relevance = clamp01(input.min_relevance);
    let min_trust_score = clamp01(input.min_trust_score);
    let max_seo_risk_score = clamp01(input.max_seo_risk_score);
    let blocked = normalized_set(&input.blocked_publishers);
    let required_tags = normalized_set(&input.required_tags);
    let preferred_payment_protocols =
        preferred_payment_protocols(&input.preferred_payment_protocols);
    let spending_mode = input.spending_mode.clone();
    let default_article_price_usd = clean_nonnegative(input.default_article_price_usd);
    let agent_wallet = input.agent_wallet.clone();

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
        if scored.trust_score < min_trust_score {
            rejected_sources.push(rejected(&scored, "below-min-trust"));
            continue;
        }
        if scored.seo_risk_score > max_seo_risk_score {
            rejected_sources.push(rejected(&scored, "seo-risk-too-high"));
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
        if candidate.price_usd > remaining_budget + USD_TOLERANCE {
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
            let payment_options =
                planned_payment_options(&scored, &spending_mode, &preferred_payment_protocols);
            let payment = payment_options
                .first()
                .cloned()
                .unwrap_or_else(|| planned_payment_default(&scored, &spending_mode));
            let rail_entry = rail_totals
                .entry(payment.protocol.clone())
                .or_insert((0, 0.0));
            rail_entry.0 += 1;
            rail_entry.1 += payment.amount_usd;

            if payment.amount_usd > 0.0 && matches!(payment.protocol.as_str(), "mpp" | "x402") {
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
                seo_risk_score: round4(scored.seo_risk_score),
                selection_score: round4(scored.selection_score),
                evidence_weight,
                payment,
                payment_options,
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
    let max_selected_price_usd = selected_sources
        .iter()
        .map(|source| source.price_usd)
        .fold(0.0, f64::max);
    let wallet_policy = agent_wallet_policy(
        agent_wallet.as_ref(),
        max_selected_price_usd,
        default_article_price_usd,
    );
    let policy_gates = policy_gates(
        &spending_mode,
        budget_usd,
        allocated_usd,
        paid_selected,
        wallet_policy.as_ref(),
        max_selected_price_usd,
    );
    let ready_for_paid_fetch = !selected_sources.is_empty()
        && allocated_usd <= budget_usd + USD_TOLERANCE
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
        spending_mode,
        budget_usd: round4(budget_usd),
        allocated_usd: round4(allocated_usd),
        unspent_usd: round4((budget_usd - allocated_usd).max(0.0)),
        selected_sources,
        rejected_sources,
        payment_rails,
        near_funding_routes,
        policy_gates,
        wallet_policy,
        ready_for_paid_fetch,
        ready_for_trade_research,
        warnings,
    })
}

pub fn plan_dripstack_browse(input: DripstackBrowseInput) -> Result<DripstackBrowsePlan, String> {
    let guardrails = vec![
        "Guided browse only: topic, publication, article, then explicit purchase approval."
            .to_string(),
        "Catalog and post-title routes are free; article body remains gated until 402 payment succeeds."
            .to_string(),
        "Never auto-buy: a selected article becomes a paid-source candidate, not fetched content."
            .to_string(),
        "Paid article text requires a receipt before it can be summarized, quoted, or used in a trade thesis."
            .to_string(),
    ];
    let mut warnings = Vec::new();
    let max_results = input.max_results.unwrap_or(5).clamp(1, 10);
    let default_price_usd = clean_nonnegative(input.default_price_usd).max(0.01);

    let topic = input
        .topic
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let selected_publication_slug = input
        .selected_publication_slug
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let selected_post_slug = input
        .selected_post_slug
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if topic.is_none() && selected_publication_slug.is_none() {
        return Ok(DripstackBrowsePlan {
            schema_version: "dripstack-browse-plan/1",
            checkpoint: "topic".to_string(),
            prompt: "Choose a topic first: finance, crypto, AI, tech, business, geopolitics, or culture.".to_string(),
            matched_publications: Vec::new(),
            post_candidates: Vec::new(),
            selected_article: None,
            paid_source_candidate: None,
            guardrails,
            warnings,
        });
    }

    let matched_publications = if let Some(topic) = topic {
        let mut scored: Vec<(f64, &DripstackPublicationInput)> = input
            .publications
            .iter()
            .map(|publication| (publication_relevance(topic, publication), publication))
            .filter(|(score, _)| *score > 0.0)
            .collect();
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.1.slug.cmp(&b.1.slug))
        });
        scored
            .into_iter()
            .take(max_results)
            .enumerate()
            .map(
                |(idx, (relevance, publication))| DripstackPublicationMatch {
                    rank: idx + 1,
                    slug: publication.slug.clone(),
                    title: publication
                        .title
                        .clone()
                        .unwrap_or_else(|| publication.slug.clone()),
                    description: publication.description.clone().unwrap_or_default(),
                    site_url: publication.site_url.clone(),
                    relevance: round4(relevance),
                },
            )
            .collect()
    } else {
        Vec::new()
    };

    if selected_publication_slug.is_none() {
        if input.publications.is_empty() {
            warnings.push(
                "No publication catalog was supplied. Fetch DripStack GET /api/v1/publications before this step."
                    .to_string(),
            );
        }
        return Ok(DripstackBrowsePlan {
            schema_version: "dripstack-browse-plan/1",
            checkpoint: "publication".to_string(),
            prompt: "Pick one matched publication before listing articles.".to_string(),
            matched_publications,
            post_candidates: Vec::new(),
            selected_article: None,
            paid_source_candidate: None,
            guardrails,
            warnings,
        });
    }

    let publication_slug = selected_publication_slug.unwrap();
    let publication = input
        .publications
        .iter()
        .find(|publication| publication.slug == publication_slug);
    if publication.is_none() && !input.publications.is_empty() {
        warnings.push(format!(
            "Selected publication '{publication_slug}' was not found in the supplied catalog."
        ));
    }
    let publication_title = publication
        .and_then(|publication| publication.title.clone())
        .unwrap_or_else(|| publication_slug.to_string());

    let post_candidates: Vec<DripstackPostCandidate> = input
        .posts
        .iter()
        .take(max_results)
        .enumerate()
        .map(|(idx, post)| post_candidate(idx + 1, publication_slug, post, default_price_usd))
        .collect();

    if selected_post_slug.is_none() {
        if input.posts.is_empty() {
            warnings.push(format!(
                "No post summaries were supplied. Fetch DripStack GET /api/v1/publications/{publication_slug} before choosing an article."
            ));
        }
        return Ok(DripstackBrowsePlan {
            schema_version: "dripstack-browse-plan/1",
            checkpoint: "article".to_string(),
            prompt: format!(
                "Pick one article from {publication_title}; no article body is fetched yet."
            ),
            matched_publications,
            post_candidates,
            selected_article: None,
            paid_source_candidate: None,
            guardrails,
            warnings,
        });
    }

    let selected_post_slug = selected_post_slug.unwrap();
    let selected_post = input
        .posts
        .iter()
        .find(|post| post.slug == selected_post_slug)
        .ok_or_else(|| {
            format!(
                "Selected post '{selected_post_slug}' was not found in the supplied post summaries."
            )
        })?;
    let selected_article = post_candidate(1, publication_slug, selected_post, default_price_usd);
    let title = selected_article.title.clone();
    let author = selected_article
        .author
        .clone()
        .unwrap_or_else(|| publication_title.clone());
    let paid_source_candidate = PaidResearchSourceInput {
        id: format!("dripstack:{publication_slug}:{selected_post_slug}"),
        title,
        author,
        publisher: Some(publication_title),
        url: selected_article.endpoint.clone(),
        summary: selected_article.subtitle.clone(),
        tags: vec![
            "dripstack".to_string(),
            "paid-content".to_string(),
            "financial-research".to_string(),
        ],
        price_usd: Some(selected_article.price_usd),
        relevance: Some(0.75),
        trust_score: Some(0.65),
        freshness_score: selected_article.published_at.as_ref().map(|_| 0.75),
        seo_risk_score: Some(0.35),
        age_days: None,
        payment: None,
        payment_options: vec![
            PaidResearchPaymentInput {
                protocol: "mpp".to_string(),
                network: Some("tempo".to_string()),
                asset: Some("USDC".to_string()),
                recipient: None,
                endpoint: Some(selected_article.endpoint.clone()),
            },
            PaidResearchPaymentInput {
                protocol: "x402".to_string(),
                network: Some("base".to_string()),
                asset: Some("USDC".to_string()),
                recipient: None,
                endpoint: Some(selected_article.endpoint.clone()),
            },
        ],
    };

    Ok(DripstackBrowsePlan {
        schema_version: "dripstack-browse-plan/1",
        checkpoint: "purchase-confirmation".to_string(),
        prompt: format!(
            "Confirm before paying for this article. The planned price is ${:.4}; article body remains locked until the payment-aware client returns a receipt.",
            selected_article.price_usd
        ),
        matched_publications,
        post_candidates,
        selected_article: Some(selected_article),
        paid_source_candidate: Some(paid_source_candidate),
        guardrails,
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
    let seo_risk_score = source.seo_risk_score.map(clamp01).unwrap_or_else(|| {
        if trust_score < 0.45 && relevance > 0.8 {
            0.75
        } else {
            0.25
        }
    });
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
        seo_risk_score,
    }
}

fn planned_payment_options(
    scored: &ScoredSource,
    spending_mode: &str,
    preferred_protocols: &[String],
) -> Vec<PlannedResearchPayment> {
    let mut raw_options = if scored.source.payment_options.is_empty() {
        scored
            .source
            .payment
            .clone()
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        scored.source.payment_options.clone()
    };
    if raw_options.is_empty() {
        raw_options.push(PaidResearchPaymentInput {
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
    }

    let mut options: Vec<PlannedResearchPayment> = raw_options
        .into_iter()
        .map(|raw| planned_payment_from_raw(scored, raw, spending_mode))
        .collect();
    options.sort_by(|a, b| {
        payment_rank(&a.protocol, preferred_protocols)
            .cmp(&payment_rank(&b.protocol, preferred_protocols))
            .then_with(|| a.network.cmp(&b.network))
    });
    options
}

fn planned_payment_default(scored: &ScoredSource, spending_mode: &str) -> PlannedResearchPayment {
    planned_payment_from_raw(
        scored,
        PaidResearchPaymentInput {
            protocol: if scored.price_usd > 0.0 {
                "manual".to_string()
            } else {
                "free".to_string()
            },
            network: None,
            asset: None,
            recipient: None,
            endpoint: None,
        },
        spending_mode,
    )
}

fn planned_payment_from_raw(
    scored: &ScoredSource,
    raw: PaidResearchPaymentInput,
    spending_mode: &str,
) -> PlannedResearchPayment {
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
    wallet_policy: Option<&AgentWalletPolicy>,
    max_selected_price_usd: f64,
) -> Vec<PaidResearchPolicyGate> {
    let mut gates = vec![
        PaidResearchPolicyGate {
            name: "budget-cap".to_string(),
            status: if allocated_usd <= budget_usd + USD_TOLERANCE {
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
    if let Some(policy) = wallet_policy {
        gates.push(PaidResearchPolicyGate {
            name: "agent-wallet-funded".to_string(),
            status: if policy.balance_usd <= 0.0
                || policy.balance_usd > policy.max_wallet_balance_usd
            {
                "warn"
            } else {
                "pass"
            }
            .to_string(),
            detail: format!(
                "{} wallet balance is ${:.2}; cap is ${:.2}.",
                policy.provider, policy.balance_usd, policy.max_wallet_balance_usd
            ),
        });
        gates.push(PaidResearchPolicyGate {
            name: "per-article-cap".to_string(),
            status: if max_selected_price_usd <= policy.per_article_cap_usd + USD_TOLERANCE {
                "pass"
            } else {
                "fail"
            }
            .to_string(),
            detail: format!(
                "Highest selected source is ${:.4}; wallet per-article cap is ${:.4}.",
                max_selected_price_usd, policy.per_article_cap_usd
            ),
        });
    } else if paid_selected > 0 {
        gates.push(PaidResearchPolicyGate {
            name: "agent-wallet-configured".to_string(),
            status: "warn".to_string(),
            detail:
                "No agent wallet policy was supplied; paid fetches must stay in explicit approval mode."
                    .to_string(),
        });
    }

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

fn agent_wallet_policy(
    input: Option<&AgentWalletPolicyInput>,
    max_selected_price_usd: f64,
    fallback_article_price_usd: f64,
) -> Option<AgentWalletPolicy> {
    let input = input?;
    let balance_usd = clean_nonnegative(input.balance_usd.unwrap_or(0.0));
    let default_article_price_usd = clean_nonnegative(input.default_article_price_usd)
        .max(clean_nonnegative(fallback_article_price_usd))
        .max(0.0001);
    let per_article_cap_usd = clean_nonnegative(input.per_article_cap_usd);
    let daily_cap_usd = clean_nonnegative(input.daily_cap_usd);
    let max_wallet_balance_usd = clean_nonnegative(input.max_wallet_balance_usd);
    let mut warnings = Vec::new();
    if balance_usd == 0.0 {
        warnings.push("Agent wallet has no declared USDC balance.".to_string());
    }
    if balance_usd > max_wallet_balance_usd {
        warnings.push("Agent wallet is over the configured autonomous balance cap.".to_string());
    }
    if max_selected_price_usd > per_article_cap_usd + USD_TOLERANCE {
        warnings.push("At least one selected source exceeds the per-article cap.".to_string());
    }
    let audit_urls = if input.audit_urls.is_empty() {
        vec![
            "https://mppscan.com/".to_string(),
            "https://www.x402scan.com/".to_string(),
        ]
    } else {
        input.audit_urls.clone()
    };
    Some(AgentWalletPolicy {
        provider: input.provider.clone(),
        address: input.address.clone(),
        network: input.network.clone(),
        balance_usd: round4(balance_usd),
        default_article_price_usd: round4(default_article_price_usd),
        max_articles_at_default_price: (balance_usd / default_article_price_usd).floor() as usize,
        per_article_cap_usd: round4(per_article_cap_usd),
        daily_cap_usd: round4(daily_cap_usd),
        max_wallet_balance_usd: round4(max_wallet_balance_usd),
        safe_to_autopay: balance_usd > 0.0
            && balance_usd <= max_wallet_balance_usd + USD_TOLERANCE
            && max_selected_price_usd <= per_article_cap_usd + USD_TOLERANCE,
        audit_urls,
        warnings,
    })
}

fn preferred_payment_protocols(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = raw
        .iter()
        .map(|protocol| normalize_protocol(protocol, 1.0))
        .filter(|protocol| !protocol.is_empty())
        .collect();
    if out.is_empty() {
        out = vec![
            "near-intents".to_string(),
            "mpp".to_string(),
            "x402".to_string(),
            "subscription".to_string(),
            "manual".to_string(),
            "free".to_string(),
        ];
    }
    out
}

fn payment_rank(protocol: &str, preferred_protocols: &[String]) -> usize {
    preferred_protocols
        .iter()
        .position(|candidate| candidate == protocol)
        .unwrap_or(usize::MAX)
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

fn publication_relevance(topic: &str, publication: &DripstackPublicationInput) -> f64 {
    lexical_relevance(
        topic,
        &format!(
            "{} {} {}",
            publication.title.as_deref().unwrap_or_default(),
            publication.description.as_deref().unwrap_or_default(),
            publication.site_url.as_deref().unwrap_or_default()
        ),
    )
}

fn post_candidate(
    rank: usize,
    publication_slug: &str,
    post: &DripstackPostInput,
    default_price_usd: f64,
) -> DripstackPostCandidate {
    DripstackPostCandidate {
        rank,
        publication_slug: publication_slug.to_string(),
        slug: post.slug.clone(),
        title: post.title.clone(),
        subtitle: post.subtitle.clone(),
        author: post.author.clone(),
        published_at: post.published_at.clone(),
        price_usd: round4(clean_nonnegative(
            post.price_usd.unwrap_or(default_price_usd),
        )),
        endpoint: format!(
            "https://dripstack.xyz/api/v1/publications/{}/{}",
            publication_slug, post.slug
        ),
    }
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
        .filter(|token| token.len() > 1 && !is_stopword(token))
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
        "the" | "and" | "for" | "with" | "into" | "from" | "that" | "this" | "what" | "when"
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

fn default_min_trust_score() -> f64 {
    0.45
}

fn default_max_seo_risk_score() -> f64 {
    0.65
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

fn default_agent_wallet_provider() -> String {
    "AgentCash".to_string()
}

fn default_agent_wallet_network() -> String {
    "base".to_string()
}

fn default_article_price_usd() -> f64 {
    0.01
}

fn default_per_article_cap_usd() -> f64 {
    0.05
}

fn default_daily_wallet_cap_usd() -> f64 {
    1.0
}

fn default_max_wallet_balance_usd() -> f64 {
    5.0
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
            seo_risk_score: Some(0.2),
            age_days: Some(1),
            payment: Some(PaidResearchPaymentInput {
                protocol: protocol.to_string(),
                network: None,
                asset: Some("USDC".to_string()),
                recipient: Some("0xmerchant".to_string()),
                endpoint: Some(format!("https://example.com/{id}/paid")),
            }),
            payment_options: Vec::new(),
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
            min_trust_score: 0.4,
            max_seo_risk_score: 0.65,
            max_source_age_days: Some(7),
            spending_mode: "quote".to_string(),
            near_funding_asset: "USDC.near".to_string(),
            preferred_payment_protocols: vec!["mpp".to_string(), "x402".to_string()],
            default_article_price_usd: 0.01,
            agent_wallet: None,
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

    #[test]
    fn funding_routes_only_include_supported_crypto_native_rails() {
        let plan = plan(PaidResearchPlanInput {
            query: "NEAR price crypto research".to_string(),
            pair: Some("NEAR/USDC".to_string()),
            budget_usd: 0.09,
            max_sources: 4,
            min_relevance: 0.1,
            min_trust_score: 0.4,
            max_seo_risk_score: 0.65,
            max_source_age_days: Some(7),
            spending_mode: "quote".to_string(),
            near_funding_asset: "USDC.near".to_string(),
            preferred_payment_protocols: vec![],
            default_article_price_usd: 0.01,
            agent_wallet: None,
            required_tags: vec![],
            blocked_publishers: vec![],
            sources: vec![
                source("manual-source", 0.02, "manual", 0.95),
                source("subscription-source", 0.02, "subscription", 0.9),
                source("mpp-source", 0.02, "mpp", 0.85),
                source("x402-source", 0.02, "x402", 0.8),
            ],
        })
        .expect("plan");

        let route_protocols: Vec<&str> = plan
            .near_funding_routes
            .iter()
            .map(|route| route.target_protocol.as_str())
            .collect();

        assert_eq!(route_protocols, vec!["mpp", "x402"]);
    }

    #[test]
    fn tokenize_preserves_two_letter_tickers_and_trading_terms() {
        let tokens = tokenize("NEAR price for OP and AR crypto token trading");

        assert!(tokens.contains("near"));
        assert!(tokens.contains("price"));
        assert!(tokens.contains("op"));
        assert!(tokens.contains("ar"));
        assert!(tokens.contains("crypto"));
        assert!(tokens.contains("token"));
        assert!(tokens.contains("trading"));
        assert!(!tokens.contains("a"));
        assert!(!tokens.contains("for"));
    }
}
