use std::sync::Arc;

use ironclaw_filesystem::CompositeRootFilesystem;
use ironclaw_notifications::{
    NOTIFICATION_INBOX_MAX_RECORDS, NotificationInboxStore, NotificationInboxStorePort,
};

/// Builds the product notification Inbox over the composition-owned filesystem.
pub(crate) fn build_notification_inbox(
    filesystem: Arc<CompositeRootFilesystem>,
) -> Arc<dyn NotificationInboxStorePort> {
    #[allow(clippy::disallowed_methods)]
    let store = NotificationInboxStore::new(
        crate::wrap_scoped(filesystem),
        NOTIFICATION_INBOX_MAX_RECORDS,
    );
    Arc::new(store)
}
