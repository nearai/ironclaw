#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    gws_bridge::run().await
}
