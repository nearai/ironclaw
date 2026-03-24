use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::OnceLock;

use rcgen::generate_simple_self_signed;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

/// Minimal local HTTPS server for tests that need a self-signed certificate.
pub struct SelfSignedHttpsServer {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl SelfSignedHttpsServer {
    pub async fn start(body: impl Into<String>) -> Self {
        let body = Arc::<str>::from(body.into());
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind self-signed HTTPS test listener"); // safety: test-only setup helper
        let addr = listener
            .local_addr()
            .expect("read local addr for self-signed HTTPS test listener"); // safety: test-only setup helper
        let acceptor = TlsAcceptor::from(Arc::new(server_config()));
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accept_result = listener.accept() => {
                        let (stream, _) = match accept_result {
                            Ok(pair) => pair,
                            Err(_) => continue,
                        };
                        let acceptor = acceptor.clone();
                        let body = Arc::clone(&body);
                        tokio::spawn(async move {
                            let mut tls_stream = match acceptor.accept(stream).await {
                                Ok(stream) => stream,
                                Err(_) => return,
                            };

                            let mut request_buf = [0_u8; 2048];
                            let _ = tls_stream.read(&mut request_buf).await;

                            let response = format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                                body.len(),
                                body,
                            );
                            let _ = tls_stream.write_all(response.as_bytes()).await;
                            let _ = tls_stream.shutdown().await;
                        });
                    }
                }
            }
        });

        Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
        }
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{}", self.addr.port(), path)
    }
}

impl Drop for SelfSignedHttpsServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

fn server_config() -> ServerConfig {
    static CRYPTO_PROVIDER: OnceLock<()> = OnceLock::new();
    CRYPTO_PROVIDER.get_or_init(|| {
        tokio_rustls::rustls::crypto::ring::default_provider()
            .install_default()
            .expect("install rustls crypto provider for test HTTPS server"); // safety: test-only setup helper
    });

    let rcgen::CertifiedKey { cert, key_pair } =
        generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("generate self-signed cert for test HTTPS server"); // safety: test-only setup helper

    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));

    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], key_der)
        .expect("build rustls server config for test HTTPS server") // safety: test-only setup helper
}
