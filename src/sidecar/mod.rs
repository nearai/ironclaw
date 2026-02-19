//! Generic Docker sidecar management for external services.
//!
//! Provides lifecycle management for Docker containers that run as long-lived
//! sidecars (e.g., Browserless for browser automation, databases for testing).
//!
//! Unlike the sandbox module which creates ephemeral containers per-command,
//! sidecars are persistent services that:
//! - Start on first request (lazy initialization)
//! - Remain running for the lifetime of the application
//! - Provide health check polling
//! - Clean up on application shutdown
//!
//! # Architecture
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────────────────────┐
//! │                           SidecarManager                                   │
//! │                                                                            │
//! │   ensure_ready()                                                          │
//! │         │                                                                  │
//! │         ▼                                                                  │
//! │   ┌──────────────┐     ┌──────────────┐     ┌──────────────────────────┐  │
//! │   │ Check Status │────▶│ Pull Image   │────▶│ Create & Start Container │  │
//! │   │ (health)     │     │ (if needed)  │     │                          │  │
//! │   └──────────────┘     └──────────────┘     └──────────────────────────┘  │
//! │                              │                     │                       │
//! │                              ▼                     ▼                       │
//! │                       ┌──────────────┐     ┌──────────────────────────┐   │
//! │                       │ Poll Health  │◀────│ Wait for Ready Signal    │   │
//! │                       │ (HTTP/TCP)   │     │                          │   │
//! │                       └──────────────┘     └──────────────────────────┘   │
//! │                              │                                            │
//! │                              ▼                                            │
//! │                       ┌──────────────┐                                   │
//! │                       │ Ready Signal │                                   │
//! │                       └──────────────┘                                   │
//! └───────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use ironclaw::sidecar::{SidecarManager, SidecarConfig, HealthCheck};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = SidecarConfig {
//!     name: "browserless".to_string(),
//!     image: "ghcr.io/browserless/chromium:latest".to_string(),
//!     ports: vec![("3000".to_string(), "3000".to_string())],
//!     env: vec![("TOKEN".to_string(), "secret".to_string())],
//!     health_check: HealthCheck::Http {
//!         path: "/".to_string(),
//!         port: 3000,
//!     },
//!     ..Default::default()
//! };
//!
//! let manager = Arc::new(SidecarManager::new(config));
//!
//! // Lazy start on first request
//! manager.ensure_ready().await?;
//!
//! // Container is now running and healthy
//! let endpoint = manager.endpoint();
//! if let Some(ep) = endpoint {
//!     println!("Sidecar available at: {}", ep);
//! }
//!
//! // Clean shutdown
//! manager.shutdown().await;
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod error;
pub mod manager;

pub use config::{HealthCheck, SidecarConfig, SidecarEndpoint};
pub use error::{Result, SidecarError};
pub use manager::SidecarManager;
