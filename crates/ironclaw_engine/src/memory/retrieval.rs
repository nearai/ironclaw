//! Context retrieval engine.
//!
//! Builds context for thread steps by retrieving relevant memory docs
//! from the project. Uses keyword matching against doc title + content,
//! with priority scoring by doc type (Lessons and Specs rank higher
//! than Summaries for context injection).

use std::sync::Arc;

use crate::traits::store::Store;
use crate::types::error::EngineError;
use crate::types::memory::{DocType, MemoryDoc};
use crate::types::project::ProjectId;

/// Retrieves relevant memory docs for a thread's context.
pub struct RetrievalEngine {
    store: Arc<dyn Store>,
}

impl RetrievalEngine {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    /// Retrieve relevant memory docs for the given query within a project.
    ///
    /// Loads all docs for the project, scores them by keyword relevance and
    /// doc-type priority, and returns the top `max_docs` results.
    pub async fn retrieve_context(
        &self,
        project_id: ProjectId,
        query: &str,
        max_docs: usize,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        if max_docs == 0 {
            return Ok(Vec::new());
        }

        let all_docs = self.store.list_memory_docs(project_id).await?;
        if all_docs.is_empty() {
            return Ok(Vec::new());
        }

        let keywords = extract_keywords(query);
        if keywords.is_empty() {
            // No meaningful keywords — return by doc-type priority alone
            let mut scored: Vec<(f64, MemoryDoc)> = all_docs
                .into_iter()
                .map(|doc| (doc_type_weight(doc.doc_type), doc))
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            scored.truncate(max_docs);
            return Ok(scored.into_iter().map(|(_, doc)| doc).collect());
        }

        let mut scored: Vec<(f64, MemoryDoc)> = all_docs
            .into_iter()
            .map(|doc| {
                let keyword_score = keyword_match_score(&doc, &keywords);
                let type_weight = doc_type_weight(doc.doc_type);
                // Combined score: keyword relevance (0.0-1.0) + type priority bonus
                let score = keyword_score + type_weight;
                (score, doc)
            })
            .filter(|(score, _)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(max_docs);
        Ok(scored.into_iter().map(|(_, doc)| doc).collect())
    }
}

/// Extract lowercase keywords from a query, filtering out stop words.
fn extract_keywords(query: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "about", "it",
        "its", "this", "that", "these", "those", "i", "you", "he", "she", "we", "they", "what",
        "which", "who", "how", "when", "where", "why", "and", "or", "but", "not", "no", "if",
        "then", "so", "up", "out", "just",
    ];

    query
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 2 && !STOP_WORDS.contains(&w.as_str()))
        .collect()
}

/// Score how well a doc matches the given keywords (0.0 to 1.0).
fn keyword_match_score(doc: &MemoryDoc, keywords: &[String]) -> f64 {
    if keywords.is_empty() {
        return 0.0;
    }

    let title_lower = doc.title.to_lowercase();
    let content_lower = doc.content.to_lowercase();

    let mut matched = 0usize;
    for kw in keywords {
        // Title matches are worth more
        if title_lower.contains(kw.as_str()) {
            matched += 2;
        } else if content_lower.contains(kw.as_str()) {
            matched += 1;
        }
    }

    // Normalize: max possible score is keywords.len() * 2 (all in title)
    let max_score = keywords.len() * 2;
    matched as f64 / max_score as f64
}

/// Priority weight by doc type. Higher = more useful for context injection.
fn doc_type_weight(doc_type: DocType) -> f64 {
    match doc_type {
        DocType::Spec => 0.5,     // Missing capability info is highest priority
        DocType::Skill => 0.45,   // Skills with activation metadata and code snippets
        DocType::Lesson => 0.4,   // Lessons prevent repeating mistakes
        DocType::Playbook => 0.3, // Reusable procedures
        DocType::Issue => 0.2,    // Known problems
        DocType::Summary => 0.1,  // Background context
        DocType::Note => 0.05,    // Scratch notes, lowest priority
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::capability::{CapabilityLease, LeaseId};
    use crate::types::event::ThreadEvent;
    use crate::types::memory::DocId;
    use crate::types::project::{Project, ProjectId};
    use crate::types::step::Step;
    use crate::types::thread::{Thread, ThreadId, ThreadState};

    /// Mock Store that returns a fixed set of memory docs.
    struct DocStore {
        docs: tokio::sync::Mutex<Vec<MemoryDoc>>,
    }

    impl DocStore {
        fn new(docs: Vec<MemoryDoc>) -> Arc<Self> {
            Arc::new(Self {
                docs: tokio::sync::Mutex::new(docs),
            })
        }
    }

    #[async_trait::async_trait]
    impl crate::traits::store::Store for DocStore {
        async fn save_thread(&self, _: &Thread) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_thread(&self, _: ThreadId) -> Result<Option<Thread>, EngineError> {
            Ok(None)
        }
        async fn list_threads(&self, _: ProjectId) -> Result<Vec<Thread>, EngineError> {
            Ok(vec![])
        }
        async fn update_thread_state(
            &self,
            _: ThreadId,
            _: ThreadState,
        ) -> Result<(), EngineError> {
            Ok(())
        }
        async fn save_step(&self, _: &Step) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_steps(&self, _: ThreadId) -> Result<Vec<Step>, EngineError> {
            Ok(vec![])
        }
        async fn append_events(&self, _: &[ThreadEvent]) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_events(&self, _: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
            Ok(vec![])
        }
        async fn save_project(&self, _: &Project) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_project(&self, _: ProjectId) -> Result<Option<Project>, EngineError> {
            Ok(None)
        }
        async fn save_memory_doc(&self, _: &MemoryDoc) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_memory_doc(&self, _: DocId) -> Result<Option<MemoryDoc>, EngineError> {
            Ok(None)
        }
        async fn list_memory_docs(
            &self,
            project_id: ProjectId,
        ) -> Result<Vec<MemoryDoc>, EngineError> {
            let docs = self.docs.lock().await;
            Ok(docs
                .iter()
                .filter(|d| d.project_id == project_id)
                .cloned()
                .collect())
        }
        async fn save_lease(&self, _: &CapabilityLease) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_active_leases(
            &self,
            _: ThreadId,
        ) -> Result<Vec<CapabilityLease>, EngineError> {
            Ok(vec![])
        }
        async fn revoke_lease(&self, _: LeaseId, _: &str) -> Result<(), EngineError> {
            Ok(())
        }
        async fn save_mission(
            &self,
            _: &crate::types::mission::Mission,
        ) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_mission(
            &self,
            _: crate::types::mission::MissionId,
        ) -> Result<Option<crate::types::mission::Mission>, EngineError> {
            Ok(None)
        }
        async fn list_missions(
            &self,
            _: ProjectId,
        ) -> Result<Vec<crate::types::mission::Mission>, EngineError> {
            Ok(vec![])
        }
        async fn update_mission_status(
            &self,
            _: crate::types::mission::MissionId,
            _: crate::types::mission::MissionStatus,
        ) -> Result<(), EngineError> {
            Ok(())
        }
    }

    #[test]
    fn extract_keywords_filters_stop_words() {
        let kws = extract_keywords("what is the latest news about Iran war");
        assert!(kws.contains(&"latest".to_string()));
        assert!(kws.contains(&"news".to_string()));
        assert!(kws.contains(&"iran".to_string()));
        assert!(kws.contains(&"war".to_string()));
        assert!(!kws.contains(&"the".to_string()));
        assert!(!kws.contains(&"is".to_string()));
    }

    #[test]
    fn extract_keywords_handles_special_chars() {
        let kws = extract_keywords("web_search web-fetch tool");
        assert!(kws.contains(&"web_search".to_string()));
        assert!(kws.contains(&"web-fetch".to_string()));
        assert!(kws.contains(&"tool".to_string()));
    }

    #[test]
    fn keyword_match_title_beats_content() {
        use crate::types::project::ProjectId;

        let doc = MemoryDoc::new(
            ProjectId::new(),
            DocType::Lesson,
            "Lesson about web_search errors",
            "The tool was not found during execution.",
        );

        let keywords = vec!["web_search".to_string()];
        let score = keyword_match_score(&doc, &keywords);
        // Title match = 2/2 = 1.0
        assert!((score - 1.0).abs() < f64::EPSILON);

        let keywords2 = vec!["execution".to_string()];
        let score2 = keyword_match_score(&doc, &keywords2);
        // Content-only match = 1/2 = 0.5
        assert!((score2 - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn doc_type_weight_ordering() {
        assert!(doc_type_weight(DocType::Spec) > doc_type_weight(DocType::Lesson));
        assert!(doc_type_weight(DocType::Lesson) > doc_type_weight(DocType::Playbook));
        assert!(doc_type_weight(DocType::Playbook) > doc_type_weight(DocType::Issue));
        assert!(doc_type_weight(DocType::Issue) > doc_type_weight(DocType::Summary));
        assert!(doc_type_weight(DocType::Summary) > doc_type_weight(DocType::Note));
    }

    #[tokio::test]
    async fn retrieve_returns_relevant_docs_by_keyword() {
        let project = ProjectId::new();
        let store = DocStore::new(vec![
            MemoryDoc::new(
                project,
                DocType::Lesson,
                "web_search tool alias",
                "Use web-search not web_search",
            ),
            MemoryDoc::new(
                project,
                DocType::Summary,
                "weather query",
                "Fetched weather data",
            ),
            MemoryDoc::new(
                project,
                DocType::Issue,
                "API timeout",
                "External API timed out",
            ),
        ]);
        let engine = RetrievalEngine::new(store);

        let docs = engine
            .retrieve_context(project, "web_search error", 5)
            .await
            .unwrap();
        assert!(!docs.is_empty());
        // The lesson about web_search should rank first (keyword + type weight)
        assert_eq!(docs[0].doc_type, DocType::Lesson);
        assert!(docs[0].title.contains("web_search"));
    }

    #[tokio::test]
    async fn retrieve_respects_project_scoping() {
        let project_a = ProjectId::new();
        let project_b = ProjectId::new();
        let store = DocStore::new(vec![
            MemoryDoc::new(
                project_a,
                DocType::Lesson,
                "Lesson for project A",
                "Some lesson",
            ),
            MemoryDoc::new(
                project_b,
                DocType::Lesson,
                "Lesson for project B",
                "Other lesson",
            ),
        ]);
        let engine = RetrievalEngine::new(store);

        let docs_a = engine
            .retrieve_context(project_a, "lesson", 5)
            .await
            .unwrap();
        assert_eq!(docs_a.len(), 1);
        assert!(docs_a[0].title.contains("project A"));

        let docs_b = engine
            .retrieve_context(project_b, "lesson", 5)
            .await
            .unwrap();
        assert_eq!(docs_b.len(), 1);
        assert!(docs_b[0].title.contains("project B"));
    }

    #[tokio::test]
    async fn retrieve_respects_max_docs_limit() {
        let project = ProjectId::new();
        let store = DocStore::new(vec![
            MemoryDoc::new(project, DocType::Lesson, "Lesson 1", "Content 1"),
            MemoryDoc::new(project, DocType::Lesson, "Lesson 2", "Content 2"),
            MemoryDoc::new(project, DocType::Lesson, "Lesson 3", "Content 3"),
        ]);
        let engine = RetrievalEngine::new(store);

        let docs = engine.retrieve_context(project, "lesson", 2).await.unwrap();
        assert_eq!(docs.len(), 2);
    }

    #[tokio::test]
    async fn retrieve_empty_store_returns_empty() {
        let project = ProjectId::new();
        let store = DocStore::new(vec![]);
        let engine = RetrievalEngine::new(store);

        let docs = engine
            .retrieve_context(project, "anything", 5)
            .await
            .unwrap();
        assert!(docs.is_empty());
    }

    #[tokio::test]
    async fn retrieve_spec_ranks_above_summary() {
        let project = ProjectId::new();
        let store = DocStore::new(vec![
            MemoryDoc::new(
                project,
                DocType::Summary,
                "Summary of search",
                "searched the web",
            ),
            MemoryDoc::new(
                project,
                DocType::Spec,
                "Missing search tool",
                "ALIAS: web_search -> web-search",
            ),
        ]);
        let engine = RetrievalEngine::new(store);

        let docs = engine.retrieve_context(project, "search", 5).await.unwrap();
        assert_eq!(docs.len(), 2);
        // Spec should rank first due to higher type weight
        assert_eq!(docs[0].doc_type, DocType::Spec);
    }
}
