//! Shared resolution of assistant-mentioned workspace files for native channels.

use ironclaw_attachments::{
    DEFAULT_ATTACHMENT_BUDGETS, WorkspaceFile, extract_workspace_attachment_paths,
};
use ironclaw_product_adapters::{ProductAdapterError, ProductOutboundPayload, RedactedString};
use ironclaw_product_workflow::ProjectFilesystemReader;
use ironclaw_threads::ThreadScope;

pub(crate) async fn resolve_workspace_attachments(
    payload: &ProductOutboundPayload,
    thread_scope: &ThreadScope,
    reader: &dyn ProjectFilesystemReader,
) -> Result<Vec<WorkspaceFile>, ProductAdapterError> {
    let ProductOutboundPayload::FinalReply(view) = payload else {
        return Ok(Vec::new());
    };
    let paths = extract_workspace_attachment_paths(&view.text);
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    if paths.len() > DEFAULT_ATTACHMENT_BUDGETS.max_count {
        return Err(permanent(
            "assistant reply references too many workspace files",
        ));
    }
    let mut attachments = Vec::with_capacity(paths.len());
    let mut total_bytes = 0usize;
    for path in paths {
        let file = reader
            .read_file(thread_scope, &path)
            .await
            .map_err(|error| {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_delivery",
                    %error,
                    "failed to read assistant-referenced workspace file"
                );
                permanent("assistant workspace file could not be read")
            })?;
        if file.bytes.len() > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes {
            return Err(permanent(
                "assistant workspace file exceeds the channel size limit",
            ));
        }
        total_bytes = total_bytes.saturating_add(file.bytes.len());
        if total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes {
            return Err(permanent(
                "assistant workspace files exceed the channel batch limit",
            ));
        }
        attachments.push(file);
    }
    Ok(attachments)
}

fn permanent(reason: &'static str) -> ProductAdapterError {
    ProductAdapterError::Internal {
        detail: RedactedString::new(reason),
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_host_api::{AgentId, TenantId, UserId};
    use ironclaw_product_adapters::FinalReplyView;
    use ironclaw_product_workflow::{ProjectFsEntry, ProjectFsError, ProjectFsStat};
    use ironclaw_turns::TurnRunId;

    use super::*;

    struct FileReader {
        bytes: Vec<u8>,
    }

    #[async_trait]
    impl ProjectFilesystemReader for FileReader {
        async fn list_dir(
            &self,
            _thread_scope: &ThreadScope,
            _path: &str,
        ) -> Result<Vec<ProjectFsEntry>, ProjectFsError> {
            Err(ProjectFsError::Denied)
        }

        async fn read_file(
            &self,
            _thread_scope: &ThreadScope,
            path: &str,
        ) -> Result<WorkspaceFile, ProjectFsError> {
            Ok(WorkspaceFile {
                path: ironclaw_host_api::ScopedPath::new(path).expect("valid test path"),
                filename: Some("report.pdf".into()),
                mime_type: "application/pdf".into(),
                bytes: self.bytes.clone(),
            })
        }

        async fn stat(
            &self,
            _thread_scope: &ThreadScope,
            _path: &str,
        ) -> Result<ProjectFsStat, ProjectFsError> {
            Err(ProjectFsError::Denied)
        }
    }

    fn thread_scope() -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            agent_id: AgentId::new("agent").expect("agent"),
            project_id: None,
            owner_user_id: Some(UserId::new("user").expect("user")),
            mission_id: None,
        }
    }

    fn final_reply(text: &str) -> ProductOutboundPayload {
        ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: text.into(),
            generated_at: Utc::now(),
        })
    }

    #[tokio::test]
    async fn final_reply_workspace_path_resolves_transient_bytes() {
        let reader = FileReader {
            bytes: b"pdf".to_vec(),
        };
        let attachments = resolve_workspace_attachments(
            &final_reply("Done: [report](/workspace/report.pdf)"),
            &thread_scope(),
            &reader,
        )
        .await
        .expect("workspace file resolves");

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].path.as_str(), "/workspace/report.pdf");
        assert_eq!(attachments[0].filename.as_deref(), Some("report.pdf"));
        assert_eq!(attachments[0].bytes, b"pdf");
    }

    #[tokio::test]
    async fn workspace_attachment_count_budget_is_enforced() {
        let text = (0..=DEFAULT_ATTACHMENT_BUDGETS.max_count)
            .map(|index| format!("/workspace/file-{index}.txt"))
            .collect::<Vec<_>>()
            .join(" ");
        let result = resolve_workspace_attachments(
            &final_reply(&text),
            &thread_scope(),
            &FileReader { bytes: vec![] },
        )
        .await;

        assert!(matches!(result, Err(ProductAdapterError::Internal { .. })));
    }

    #[tokio::test]
    async fn workspace_attachment_file_budget_accepts_equal_and_rejects_over() {
        let equal = resolve_workspace_attachments(
            &final_reply("/workspace/equal.bin"),
            &thread_scope(),
            &FileReader {
                bytes: vec![0; DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes],
            },
        )
        .await;
        assert!(equal.is_ok());

        let over = resolve_workspace_attachments(
            &final_reply("/workspace/over.bin"),
            &thread_scope(),
            &FileReader {
                bytes: vec![0; DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes + 1],
            },
        )
        .await;
        assert!(matches!(over, Err(ProductAdapterError::Internal { .. })));
    }

    #[tokio::test]
    async fn workspace_attachment_total_budget_accepts_equal_and_rejects_over() {
        let reader = FileReader {
            bytes: vec![0; DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes],
        };
        let equal = resolve_workspace_attachments(
            &final_reply("/workspace/one.bin /workspace/two.bin"),
            &thread_scope(),
            &reader,
        )
        .await;
        assert!(equal.is_ok());

        let over = resolve_workspace_attachments(
            &final_reply("/workspace/one.bin /workspace/two.bin /workspace/three.bin"),
            &thread_scope(),
            &reader,
        )
        .await;
        assert!(matches!(over, Err(ProductAdapterError::Internal { .. })));
    }
}
