use tokio::sync::Mutex;

use ironclaw_host_api::ThreadId;
use ironclaw_threads::{ContextWindow, ThreadScope};

/// One-shot cache shared by prompt and model ports within a single host-built
/// model request.
#[derive(Default)]
pub struct ThreadContextWindowCache {
    cached: Mutex<Option<CachedContextWindow>>,
}

struct CachedContextWindow {
    scope: ThreadScope,
    thread_id: ThreadId,
    max_messages: usize,
    context: ContextWindow,
}

impl ThreadContextWindowCache {
    pub(crate) async fn store(
        &self,
        scope: ThreadScope,
        max_messages: usize,
        context: ContextWindow,
    ) {
        let mut cached = self.cached.lock().await;
        *cached = Some(CachedContextWindow {
            scope,
            thread_id: context.thread_id.clone(),
            max_messages,
            context,
        });
    }

    pub(crate) async fn take_matching(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        max_messages: usize,
    ) -> Option<ContextWindow> {
        let mut cached = self.cached.lock().await;
        if cached.as_ref().is_some_and(|entry| {
            entry.scope == *scope
                && entry.thread_id == *thread_id
                && entry.max_messages == max_messages
        }) {
            return cached.take().map(|entry| entry.context);
        }
        None
    }
}
