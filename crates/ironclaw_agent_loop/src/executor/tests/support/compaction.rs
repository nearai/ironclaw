use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ironclaw_turns::run_profile::{
    LoopCancelReasonKind, LoopCompactionError, LoopCompactionRequest, LoopCompactionResponse,
    LoopContextCompactionMetadata,
};

use super::MockHost;

#[derive(Clone)]
pub(super) struct MockCompactionSupport {
    prompt_indexes: Arc<Mutex<VecDeque<Vec<LoopContextCompactionMetadata>>>>,
    result: Arc<Mutex<Result<LoopCompactionResponse, LoopCompactionError>>>,
    delay: Arc<Mutex<Option<std::time::Duration>>>,
    cancel_after_success: Arc<Mutex<bool>>,
    cancel_on_start: Arc<Mutex<bool>>,
}

impl MockCompactionSupport {
    pub(super) fn new() -> Self {
        Self {
            prompt_indexes: Arc::new(Mutex::new(VecDeque::new())),
            result: Arc::new(Mutex::new(Err(LoopCompactionError::InputTooLarge))),
            delay: Arc::new(Mutex::new(None)),
            cancel_after_success: Arc::new(Mutex::new(false)),
            cancel_on_start: Arc::new(Mutex::new(false)),
        }
    }

    pub(super) fn set_prompt_index(&self, index: Vec<LoopContextCompactionMetadata>) {
        *self.prompt_indexes.lock().expect("lock") = VecDeque::from([index]);
    }

    pub(super) fn set_prompt_indexes(&self, indexes: Vec<Vec<LoopContextCompactionMetadata>>) {
        *self.prompt_indexes.lock().expect("lock") = indexes.into();
    }

    pub(super) fn next_prompt_index(&self) -> Vec<LoopContextCompactionMetadata> {
        self.prompt_indexes
            .lock()
            .expect("lock")
            .pop_front()
            .unwrap_or_default()
    }

    pub(super) fn set_result(&self, result: Result<LoopCompactionResponse, LoopCompactionError>) {
        *self.result.lock().expect("lock") = result;
    }

    pub(super) fn set_delay(&self, delay: std::time::Duration) {
        *self.delay.lock().expect("lock") = Some(delay);
    }

    pub(super) fn set_cancel_after_success(&self) {
        *self.cancel_after_success.lock().expect("lock") = true;
    }

    pub(super) fn set_cancel_on_start(&self) {
        *self.cancel_on_start.lock().expect("lock") = true;
    }

    pub(super) fn take_cancel_on_start(&self) -> bool {
        std::mem::take(&mut *self.cancel_on_start.lock().expect("lock"))
    }

    pub(super) fn take_cancel_after_success(&self) -> bool {
        std::mem::take(&mut *self.cancel_after_success.lock().expect("lock"))
    }

    pub(super) async fn compact_loop_context(
        &self,
        _request: LoopCompactionRequest,
    ) -> Result<LoopCompactionResponse, LoopCompactionError> {
        let delay = *self.delay.lock().expect("lock");
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        self.result.lock().expect("lock").clone()
    }
}

impl MockHost {
    pub(in crate::executor::tests) fn with_prompt_compaction_index(
        self,
        index: Vec<LoopContextCompactionMetadata>,
    ) -> Self {
        self.compaction.set_prompt_index(index);
        self
    }

    pub(in crate::executor::tests) fn with_prompt_compaction_indexes(
        self,
        indexes: Vec<Vec<LoopContextCompactionMetadata>>,
    ) -> Self {
        self.compaction.set_prompt_indexes(indexes);
        self
    }

    pub(in crate::executor::tests) fn with_compaction_result(
        self,
        result: Result<LoopCompactionResponse, LoopCompactionError>,
    ) -> Self {
        self.compaction.set_result(result);
        self
    }

    pub(in crate::executor::tests) fn with_compaction_delay(
        self,
        delay: std::time::Duration,
    ) -> Self {
        self.compaction.set_delay(delay);
        self
    }

    pub(in crate::executor::tests) fn cancel_after_compaction_success(self) -> Self {
        self.compaction.set_cancel_after_success();
        self
    }

    pub(in crate::executor::tests) fn cancel_on_compaction_start(self) -> Self {
        self.compaction.set_cancel_on_start();
        self
    }

    pub(super) async fn compact_loop_context_for_tests(
        &self,
        request: LoopCompactionRequest,
    ) -> Result<LoopCompactionResponse, LoopCompactionError> {
        if self.compaction.take_cancel_on_start() {
            self.request_cancellation(LoopCancelReasonKind::UserRequested);
        }
        let result = self.compaction.compact_loop_context(request).await;
        if result.is_ok() && self.compaction.take_cancel_after_success() {
            self.request_cancellation(LoopCancelReasonKind::UserRequested);
        }
        result
    }
}
