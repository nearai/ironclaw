pub mod db_probe;
#[doc(hidden)]
pub mod redaction;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::db_probe::{DbProbeConfig, DbProbeTarget};

    #[test]
    fn db_probe_config_selects_libsql_path() {
        let path = PathBuf::from("measurement.db");
        let config = DbProbeConfig::libsql(path.clone(), true);

        assert_eq!(config.target(), &DbProbeTarget::LibSql { path });
        assert!(config.reset_stats());
    }

    #[test]
    fn db_probe_config_selects_postgres_url() {
        let url = "postgresql://localhost/ironclaw";
        let config = DbProbeConfig::postgres(url, false);

        assert_eq!(
            config.target(),
            &DbProbeTarget::Postgres {
                url: url.to_string(),
            }
        );
        assert!(!config.reset_stats());
    }
}
