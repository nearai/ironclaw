use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, RootFilesystem};
use ironclaw_reborn_openai_compat::FilesystemOpenAiCompatRefStore;

pub(crate) fn in_memory_openai_compat_ref_store() -> Arc<FilesystemOpenAiCompatRefStore> {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
    Arc::new(FilesystemOpenAiCompatRefStore::new(filesystem))
}
