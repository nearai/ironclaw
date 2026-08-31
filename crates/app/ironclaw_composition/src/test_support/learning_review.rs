//! Learning review test support over the production sink and candidate store.

use std::sync::Arc;

use ironclaw_filesystem::CompositeRootFilesystem;
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_memory::LearningCandidateStore;
use ironclaw_product_contracts::operator_llm::LearningSettings;
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::TurnEventSink;

/// The production learning-review components with only inference injected at
/// the provider-neutral port. The integration harness uses the sink in the
/// real completed-turn event fan-out and reads the same candidate store.
pub struct LearningReviewTestSupport {
    pub sink: Arc<dyn TurnEventSink>,
    pub candidate_store: Arc<dyn LearningCandidateStore>,
    pub tasks: Arc<ironclaw_loop_host::learning_review::LearningReviewTasks>,
    pub controller: Arc<ironclaw_loop_host::learning_review::LearningRuntimeControllerImpl>,
}

/// Build the same controller, filesystem candidate store, task supervisor, and
/// completed-turn sink that production composition wires. Tests inject a
/// provider-neutral inference port because the ordinary harness's model
/// override intentionally bypasses the production reload/provider handle.
pub fn learning_review_turn_event_sink_for_test(
    thread_service: Arc<dyn SessionThreadService>,
    filesystem: Arc<CompositeRootFilesystem>,
    storage_scope: ResourceScope,
    inference: Arc<dyn ironclaw_loop_host::learning_review::LearningInferencePort>,
    settings: LearningSettings,
) -> LearningReviewTestSupport {
    let controller =
        Arc::new(ironclaw_loop_host::learning_review::LearningRuntimeControllerImpl::new(settings));
    let scoped_filesystem = crate::wrap_scoped(filesystem);
    let candidate_store: Arc<dyn LearningCandidateStore> = Arc::new(
        ironclaw_loop_host::learning_review::FilesystemLearningCandidateStore::new(
            scoped_filesystem,
            storage_scope,
        ),
    );
    let tasks = Arc::new(ironclaw_loop_host::learning_review::LearningReviewTasks::new());
    let sink: Arc<dyn TurnEventSink> = Arc::new(
        ironclaw_loop_host::learning_review::LearningReviewTurnEventSink::new(
            thread_service,
            inference,
            Arc::clone(&candidate_store),
            Arc::clone(&tasks),
            Arc::clone(&controller),
        ),
    );
    LearningReviewTestSupport {
        sink,
        candidate_store,
        tasks,
        controller,
    }
}
