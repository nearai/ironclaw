use super::*;

use ironclaw_common::hashing::sha256_hex;
use ironclaw_filesystem::{CasUpdateError, Entry, RecordKind, cas_update};
use ironclaw_host_api::path::ScopedPath;
use sha2::{Digest, Sha256};

impl<F: RootFilesystem + ?Sized> FilesystemSuggestionsStore<F> {
    pub(super) async fn update<T, A, Fut>(
        &self,
        scope: &ResourceScope,
        operation: &'static str,
        apply: A,
    ) -> Result<T, SuggestionsStoreError>
    where
        A: FnMut(Option<SuggestionDocument>) -> Fut,
        Fut: std::future::Future<
                Output = Result<CasApply<SuggestionDocument, T>, SuggestionsStoreError>,
            >,
    {
        let path = document_path(scope)?;
        cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            decode_document,
            document_entry,
            apply,
        )
        .await
        .map_err(|error| cas_error(operation, error))
    }
}

pub(super) fn document_path(scope: &ResourceScope) -> Result<ScopedPath, SuggestionsStoreError> {
    let mut digest = Sha256::new();
    update_digest_part(&mut digest, scope.tenant_id.to_string().as_bytes());
    update_digest_part(&mut digest, scope.user_id.to_string().as_bytes());
    let context_key = sha256_hex(&digest.finalize());
    ScopedPath::new(format!("{SUGGESTION_DOCUMENT_ROOT}/{context_key}/doc.json")).map_err(|error| {
        SuggestionsStoreError::Filesystem {
            operation: "construct suggestion document path",
            reason: error.to_string(),
        }
    })
}

fn update_digest_part(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

pub(super) fn decode_document(bytes: &[u8]) -> Result<SuggestionDocument, SuggestionsStoreError> {
    serde_json::from_slice(bytes).map_err(|error| SuggestionsStoreError::Serialization {
        reason: error.to_string(),
    })
}

pub(super) fn document_entry(
    document: &SuggestionDocument,
) -> Result<Entry, SuggestionsStoreError> {
    let payload =
        serde_json::to_value(document).map_err(|error| SuggestionsStoreError::Serialization {
            reason: error.to_string(),
        })?;
    let kind = RecordKind::new(SUGGESTION_DOCUMENT_RECORD_KIND).map_err(|error| {
        SuggestionsStoreError::Serialization {
            reason: error.to_string(),
        }
    })?;
    Entry::record(kind, &payload).map_err(|error| SuggestionsStoreError::Serialization {
        reason: error.to_string(),
    })
}

pub(super) fn filesystem_error(
    operation: &'static str,
    error: impl std::fmt::Display,
) -> SuggestionsStoreError {
    SuggestionsStoreError::Filesystem {
        operation,
        reason: error.to_string(),
    }
}

pub(super) fn cas_error(
    operation: &'static str,
    error: CasUpdateError<SuggestionsStoreError>,
) -> SuggestionsStoreError {
    match error {
        CasUpdateError::Apply(error) => error,
        other => filesystem_error(operation, other),
    }
}
