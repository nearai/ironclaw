use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, Filter, Page, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{path::ScopedPath, resource::ResourceScope};
use ironclaw_memory::{
    LearningCandidateInsert, LearningCandidateStore, LearningCandidateStoreError,
    LearningReviewRecord, LearningScope, MAX_LEARNING_UNRESOLVED_PROPOSALS,
};
use ironclaw_turns::TurnRunId;

const CANDIDATE_DIRECTORY: &str = "/tenant-shared/learning-candidates";

/// Durable candidate store. A run owns one immutable record, so an exclusive
/// CAS write makes repeated completion events idempotent without a mutex.
pub struct FilesystemLearningCandidateStore<F: RootFilesystem + ?Sized> {
    filesystem: Arc<ScopedFilesystem<F>>,
    storage_scope: ResourceScope,
}

impl<F: RootFilesystem + ?Sized> FilesystemLearningCandidateStore<F> {
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, storage_scope: ResourceScope) -> Self {
        Self {
            filesystem,
            storage_scope,
        }
    }

    fn directory(scope: &LearningScope) -> Result<ScopedPath, LearningCandidateStoreError> {
        let project = match scope.project_id() {
            Some(project) => format!("project-{}", project.as_str()),
            None => "project-none".to_string(),
        };
        ScopedPath::new(format!(
            "{CANDIDATE_DIRECTORY}/{}/{}/{}/{}",
            scope.tenant_id().as_str(),
            scope.user_id().as_str(),
            scope.agent_id().as_str(),
            project,
        ))
        .map_err(|error| {
            tracing::debug!(%error, "learning candidate path construction failed");
            LearningCandidateStoreError::InvalidData
        })
    }

    fn path(
        scope: &LearningScope,
        run_id: TurnRunId,
    ) -> Result<ScopedPath, LearningCandidateStoreError> {
        let directory = Self::directory(scope)?;
        ScopedPath::new(format!("{}/{}.json", directory.as_str(), run_id)).map_err(|error| {
            tracing::debug!(%error, ?run_id, "learning candidate path construction failed");
            LearningCandidateStoreError::InvalidData
        })
    }
}

#[async_trait]
impl<F: RootFilesystem + ?Sized> LearningCandidateStore for FilesystemLearningCandidateStore<F> {
    async fn insert_if_absent(
        &self,
        record: &LearningReviewRecord,
    ) -> Result<LearningCandidateInsert, LearningCandidateStoreError> {
        if record.idempotency_key.as_str() != format!("learning-review:{}", record.run_id) {
            return Err(LearningCandidateStoreError::InvalidData);
        }
        record
            .review
            .validate()
            .map_err(|error| {
                tracing::debug!(%error, run_id = ?record.run_id, "learning candidate validation failed");
                LearningCandidateStoreError::InvalidData
            })?;
        let bytes = serde_json::to_vec(record).map_err(|error| {
            tracing::debug!(%error, run_id = ?record.run_id, "learning candidate serialization failed");
            LearningCandidateStoreError::InvalidData
        })?;
        let path = Self::path(&record.scope, record.run_id)?;
        match self
            .filesystem
            .put(
                &self.storage_scope,
                &path,
                Entry::bytes(bytes),
                CasExpectation::Absent,
            )
            .await
        {
            Ok(_) => Ok(LearningCandidateInsert::Created),
            Err(FilesystemError::VersionMismatch { .. }) => {
                Ok(LearningCandidateInsert::AlreadyExists)
            }
            Err(error) => {
                tracing::debug!(%error, run_id = ?record.run_id, "learning candidate write failed");
                Err(LearningCandidateStoreError::Unavailable)
            }
        }
    }

    async fn get(
        &self,
        scope: &LearningScope,
        run_id: TurnRunId,
    ) -> Result<Option<LearningReviewRecord>, LearningCandidateStoreError> {
        let path = Self::path(scope, run_id)?;
        let Some(row) = self
            .filesystem
            .get(&self.storage_scope, &path)
            .await
            .map_err(|error| {
                tracing::debug!(%error, ?run_id, "learning candidate read failed");
                LearningCandidateStoreError::Unavailable
            })?
        else {
            return Ok(None);
        };
        let record: LearningReviewRecord =
            serde_json::from_slice(&row.entry.body).map_err(|error| {
                tracing::debug!(%error, ?run_id, "learning candidate deserialization failed");
                LearningCandidateStoreError::InvalidData
            })?;
        if &record.scope != scope
            || record.run_id != run_id
            || record.idempotency_key.as_str() != format!("learning-review:{run_id}")
        {
            return Err(LearningCandidateStoreError::InvalidData);
        }
        record.review.validate().map_err(|error| {
            tracing::debug!(%error, ?run_id, "learning candidate validation failed");
            LearningCandidateStoreError::InvalidData
        })?;
        Ok(Some(record))
    }

    async fn list_unresolved(
        &self,
        scope: &LearningScope,
    ) -> Result<Vec<LearningReviewRecord>, LearningCandidateStoreError> {
        let prefix = Self::directory(scope)?;
        let rows = match self
            .filesystem
            .query(
                &self.storage_scope,
                &prefix,
                &Filter::All,
                Page::first(MAX_LEARNING_UNRESOLVED_PROPOSALS),
            )
            .await
        {
            Ok(rows) => rows,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => {
                tracing::debug!(%error, "learning candidate query failed");
                return Err(LearningCandidateStoreError::Unavailable);
            }
        };
        rows.into_iter()
            .map(|row| {
                let record: LearningReviewRecord = serde_json::from_slice(&row.entry.body)
                    .map_err(|error| {
                        tracing::debug!(%error, "learning candidate deserialization failed");
                        LearningCandidateStoreError::InvalidData
                    })?;
                if &record.scope != scope
                    || record.idempotency_key.as_str()
                        != format!("learning-review:{}", record.run_id)
                {
                    return Err(LearningCandidateStoreError::InvalidData);
                }
                record
                    .review
                    .validate()
                    .map_err(|error| {
                        tracing::debug!(%error, run_id = ?record.run_id, "learning candidate validation failed");
                        LearningCandidateStoreError::InvalidData
                    })?;
                Ok(record)
            })
            .collect()
    }
}
