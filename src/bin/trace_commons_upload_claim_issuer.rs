use ironclaw::trace_upload_claim_issuer::{
    TraceUploadClaimIssuerConfig, configure_tenant_access_grants_from_env,
    serve_trace_upload_claim_issuer,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "trace_commons_upload_claim_issuer=info,ironclaw=info".into()),
        )
        .init();
    let mut config = TraceUploadClaimIssuerConfig::from_env()?;
    configure_tenant_access_grants_from_env(&mut config).await?;
    serve_trace_upload_claim_issuer(config).await
}
