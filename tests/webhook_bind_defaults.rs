use std::net::SocketAddr;

use ironclaw::config::default_webhook_bind_addr;

#[test]
fn default_webhook_bind_addr_matches_platform() {
    #[cfg(windows)]
    {
        assert_eq!(
            default_webhook_bind_addr(8080),
            SocketAddr::from(([127, 0, 0, 1], 8080))
        );
    }

    #[cfg(not(windows))]
    {
        assert_eq!(
            default_webhook_bind_addr(8080),
            SocketAddr::from(([0, 0, 0, 0], 8080))
        );
    }
}
