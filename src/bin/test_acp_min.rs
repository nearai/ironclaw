//! Minimal ACP server test — no agent, just the protocol layer.
//! Usage: cargo run --release --bin test_acp_min

use std::rc::Rc;
use agent_client_protocol::{
    Agent, AgentSideConnection, AgentCapabilities, AuthenticateRequest, AuthenticateResponse,
    ContentBlock, Error, Implementation, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse,
    ProtocolVersion, SessionId, StopReason,
};

struct MinAgent;

#[async_trait::async_trait(?Send)]
impl Agent for MinAgent {
    async fn initialize(&self, _args: InitializeRequest) -> Result<InitializeResponse, Error> {
        eprintln!("[MIN] initialize called");
        Ok(InitializeResponse::new(ProtocolVersion::LATEST)
            .agent_capabilities(AgentCapabilities::new())
            .agent_info(Implementation::new("min-agent", "0.1.0")))
    }

    async fn authenticate(&self, _args: AuthenticateRequest) -> Result<AuthenticateResponse, Error> {
        Ok(AuthenticateResponse::new())
    }

    async fn new_session(&self, _args: NewSessionRequest) -> Result<NewSessionResponse, Error> {
        eprintln!("[MIN] new_session called");
        Ok(NewSessionResponse::new(SessionId::new("test-session".to_string())))
    }

    async fn prompt(&self, args: PromptRequest) -> Result<PromptResponse, Error> {
        eprintln!("[MIN] prompt called");
        let content: String = args.prompt.iter()
            .filter_map(|b| match b { ContentBlock::Text(t) => Some(t.text.clone()), _ => None })
            .collect::<Vec<_>>().join("\n");
        eprintln!("[MIN] prompt content: {content}");

        // Send a text delta notification
        let mut stdout = tokio::io::stdout();
        use tokio::io::AsyncWriteExt;
        let delta = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/session/update",
            "params": {
                "update": {
                    "type": "text_delta",
                    "delta": format!("You said: {content}")
                }
            }
        });
        let mut line = serde_json::to_string(&delta).unwrap_or_default();
        line.push('\n');
        stdout.write_all(line.as_bytes()).await.ok();
        stdout.flush().await.ok();

        Ok(PromptResponse::new(StopReason::EndTurn))
    }

    async fn cancel(&self, _args: agent_client_protocol::CancelNotification) -> Result<(), Error> {
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    eprintln!("[MIN] Starting minimal ACP server on current_thread runtime");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let local_set = tokio::task::LocalSet::new();
    let result = local_set.run_until(async {
        eprintln!("[MIN] LocalSet started, creating AgentSideConnection");
        let (_conn, io_task) = AgentSideConnection::new(
            MinAgent,
            tokio_util::compat::TokioAsyncWriteCompatExt::compat_write(stdout),
            tokio_util::compat::TokioAsyncReadCompatExt::compat(stdin),
            |fut| {
                tokio::task::spawn_local(fut);
            },
        );
        eprintln!("[MIN] AgentSideConnection created, waiting for I/O");
        io_task.await.map_err(|e| anyhow::anyhow!("ACP I/O: {e}"))
    }).await;
    eprintln!("[MIN] ACP I/O finished: {result:?}");
    result?;
    Ok(())
}