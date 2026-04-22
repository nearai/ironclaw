//! Strategy stage — generate ranked rebalancing proposals from
//! classified positions and a set of strategy docs.
//!
//! Strategy docs are user-authored Markdown files with a YAML
//! frontmatter block declaring constraints. The deterministic
//! constraint filter lives here; the LLM does the final ranking
//! back in the skill playbook (it sees the candidate set the filter
//! emitted, plus each strategy's prose body).

use crate::types::{ClassifiedPosition, ProjectConfig, Proposal};

mod filter;
mod parser;

pub fn propose(
    positions: &[ClassifiedPosition],
    strategy_sources: &[String],
    config: &ProjectConfig,
) -> Result<Vec<Proposal>, String> {
    let mut docs = Vec::with_capacity(strategy_sources.len());
    for src in strategy_sources {
        docs.push(parser::parse(src)?);
    }

    let mut proposals = Vec::new();
    for doc in &docs {
        let mut from_doc = filter::candidates(doc, positions, config);
        proposals.append(&mut from_doc);
    }
    Ok(proposals)
}
