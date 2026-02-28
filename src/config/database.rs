use std::path::PathBuf;

use secrecy::{ExposeSecret, SecretString};

use crate::bootstrap::ironclaw_base_dir;
use crate::config::helpers::{optional_env, parse_optional_env};
use crate::error::ConfigError;

/// Which database backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DatabaseBackend {
    /// PostgreSQL via deadpool-postgres (default).
    #[default]
    Postgres,
    /// libSQL/Turso embedded database.
    LibSql,
}

impl std::fmt::Display for DatabaseBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Postgres => write!(f, "postgres"),
            Self::LibSql => write!(f, "libsql"),
        }
    }
}

impl std::str::FromStr for DatabaseBackend {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "postgres" | "postgresql" | "pg" => Ok(Self::Postgres),
            "libsql" | "turso" | "sqlite" => Ok(Self::LibSql),
            _ => Err(format!(
                "invalid database backend '{}', expected 'postgres' or 'libsql'",
                s
            )),
        }
    }
}

/// PostgreSQL SSL/TLS mode, matching libpq semantics for the common cases.
///
/// Default is `Prefer`: attempt TLS, fall back to plaintext.  This is the
/// safest non-breaking default — local Postgres without TLS keeps working
/// while managed providers (Neon, Supabase, RDS) automatically get TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SslMode {
    /// Never use TLS (equivalent to libpq `sslmode=disable`).
    Disable,
    /// Try TLS first; fall back to plaintext on failure (default).
    #[default]
    Prefer,
    /// Require TLS; fail if the server does not support it.
    Require,
}

impl std::fmt::Display for SslMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disable => write!(f, "disable"),
            Self::Prefer => write!(f, "prefer"),
            Self::Require => write!(f, "require"),
        }
    }
}

impl std::str::FromStr for SslMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "disable" => Ok(Self::Disable),
            "prefer" => Ok(Self::Prefer),
            "require" => Ok(Self::Require),
            _ => Err(format!(
                "invalid DATABASE_SSLMODE '{}', expected 'disable', 'prefer', or 'require'",
                s
            )),
        }
    }
}

/// Database configuration.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Which backend to use (default: Postgres).
    pub backend: DatabaseBackend,

    // -- PostgreSQL fields --
    pub url: SecretString,
    pub pool_size: usize,
    /// TLS mode for PostgreSQL connections (default: Prefer).
    pub ssl_mode: SslMode,

    // -- libSQL fields --
    /// Path to local libSQL database file (default: ~/.ironclaw/ironclaw.db).
    pub libsql_path: Option<PathBuf>,
    /// Turso cloud URL for remote sync (optional).
    pub libsql_url: Option<String>,
    /// Turso auth token (required when libsql_url is set).
    pub libsql_auth_token: Option<SecretString>,
}

impl DatabaseConfig {
    pub(crate) fn resolve() -> Result<Self, ConfigError> {
        let backend: DatabaseBackend = if let Some(b) = optional_env("DATABASE_BACKEND")? {
            b.parse().map_err(|e| ConfigError::InvalidValue {
                key: "DATABASE_BACKEND".to_string(),
                message: e,
            })?
        } else {
            DatabaseBackend::default()
        };

        // PostgreSQL URL is required only when using the postgres backend.
        // For libsql backend, default to an empty placeholder.
        // DATABASE_URL is loaded from ~/.ironclaw/.env via dotenvy early in startup.
        let url = optional_env("DATABASE_URL")?
            .or_else(|| {
                if backend == DatabaseBackend::LibSql {
                    Some("unused://libsql".to_string())
                } else {
                    None
                }
            })
            .ok_or_else(|| ConfigError::MissingRequired {
                key: "DATABASE_URL".to_string(),
                hint: "Run 'ironclaw onboard' or set DATABASE_URL environment variable".to_string(),
            })?;

        let pool_size = parse_optional_env("DATABASE_POOL_SIZE", 10)?;

        let ssl_mode: SslMode = if let Some(s) = optional_env("DATABASE_SSLMODE")? {
            s.parse().map_err(|e| ConfigError::InvalidValue {
                key: "DATABASE_SSLMODE".to_string(),
                message: e,
            })?
        } else {
            SslMode::default()
        };

        let libsql_path = optional_env("LIBSQL_PATH")?.map(PathBuf::from).or_else(|| {
            if backend == DatabaseBackend::LibSql {
                Some(default_libsql_path())
            } else {
                None
            }
        });

        let libsql_url = optional_env("LIBSQL_URL")?;
        let libsql_auth_token = optional_env("LIBSQL_AUTH_TOKEN")?.map(SecretString::from);

        if libsql_url.is_some() && libsql_auth_token.is_none() {
            return Err(ConfigError::MissingRequired {
                key: "LIBSQL_AUTH_TOKEN".to_string(),
                hint: "LIBSQL_AUTH_TOKEN is required when LIBSQL_URL is set".to_string(),
            });
        }

        Ok(Self {
            backend,
            url: SecretString::from(url),
            pool_size,
            ssl_mode,
            libsql_path,
            libsql_url,
            libsql_auth_token,
        })
    }

    /// Get the database URL (exposes the secret).
    pub fn url(&self) -> &str {
        self.url.expose_secret()
    }
}

impl SslMode {
    /// Read from `DATABASE_SSLMODE` env var, defaulting to `Prefer`.
    ///
    /// Silently falls back to `Prefer` on missing or unparseable values.
    /// Used by lightweight CLI tools (status, doctor) that don't run the
    /// full config pipeline.
    pub fn from_env() -> Self {
        std::env::var("DATABASE_SSLMODE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default()
    }
}

/// Default libSQL database path (~/.ironclaw/ironclaw.db).
pub fn default_libsql_path() -> PathBuf {
    ironclaw_base_dir().join("ironclaw.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssl_mode_default_is_prefer() {
        assert_eq!(SslMode::default(), SslMode::Prefer);
    }

    #[test]
    fn ssl_mode_parse_roundtrip() {
        for mode in [SslMode::Disable, SslMode::Prefer, SslMode::Require] {
            let s = mode.to_string();
            let parsed: SslMode = s.parse().expect("should parse");
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn ssl_mode_parse_case_insensitive() {
        assert_eq!("DISABLE".parse::<SslMode>().unwrap(), SslMode::Disable);
        assert_eq!("Prefer".parse::<SslMode>().unwrap(), SslMode::Prefer);
        assert_eq!("REQUIRE".parse::<SslMode>().unwrap(), SslMode::Require);
    }

    #[test]
    fn ssl_mode_parse_invalid() {
        assert!("invalid".parse::<SslMode>().is_err());
    }
}
