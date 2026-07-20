//! Shared resolution of assistant-mentioned workspace files for native channels.

use ironclaw_attachments::{DEFAULT_ATTACHMENT_BUDGETS, extract_workspace_attachment_paths};
use ironclaw_product_adapters::{
    ProductAdapterError, ProductOutboundAttachment, ProductOutboundPayload, RedactedString,
};
use ironclaw_product_workflow::ProjectFilesystemReader;
use ironclaw_threads::ThreadScope;

pub(crate) async fn resolve_workspace_attachments(
    payload: &ProductOutboundPayload,
    thread_scope: &ThreadScope,
    reader: Option<&dyn ProjectFilesystemReader>,
) -> Result<Vec<ProductOutboundAttachment>, ProductAdapterError> {
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
    let reader = reader.ok_or_else(|| permanent("workspace attachment reader is unavailable"))?;
    let mut attachments = Vec::with_capacity(paths.len());
    let mut total_bytes = 0usize;
    for path in paths {
        let file = reader
            .read_file(thread_scope, &path)
            .await
            .map_err(|_| permanent("assistant workspace file could not be read"))?;
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
        let filename = file
            .filename
            .or_else(|| path.rsplit('/').next().map(ToOwned::to_owned))
            .ok_or_else(|| permanent("assistant workspace file has no filename"))?;
        attachments.push(ProductOutboundAttachment::new(
            path,
            filename,
            file.mime_type,
            file.bytes,
        )?);
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
    use ironclaw_product_workflow::{ProjectFsEntry, ProjectFsError, ProjectFsFile, ProjectFsStat};
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
        ) -> Result<ProjectFsFile, ProjectFsError> {
            Ok(ProjectFsFile {
                path: path.to_string(),
                filename: Some("report.pdf".into()),
                mime_type: "application/pdf".into(),
                size_bytes: self.bytes.len() as u64,
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
            Some(&reader),
        )
        .await
        .expect("workspace file resolves");

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].workspace_path(), "/workspace/report.pdf");
        assert_eq!(attachments[0].filename(), "report.pdf");
        assert_eq!(attachments[0].bytes(), b"pdf");
    }

    #[tokio::test]
    async fn referenced_file_without_reader_fails_closed() {
        let result = resolve_workspace_attachments(
            &final_reply("/workspace/report.pdf"),
            &thread_scope(),
            None,
        )
        .await;
        let error = match result {
            Ok(_) => panic!("must not silently drop referenced file"),
            Err(error) => error,
        };

        assert!(matches!(error, ProductAdapterError::Internal { .. }));
    }
}
