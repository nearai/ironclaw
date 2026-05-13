use std::path::PathBuf;
use std::sync::Arc;

use ironclaw_secrets::SecretMaterial;

use crate::RebornCompositionProfile;

pub struct RebornBuildInput {
    pub(crate) profile: RebornCompositionProfile,
    pub(crate) owner_id: String,
    pub(crate) storage: RebornStorageInput,
    pub(crate) required_runtime_backends: Vec<ironclaw_host_api::RuntimeKind>,
    pub(crate) require_runtime_http_egress: bool,
    pub(crate) require_wasm_credentials: bool,
}

pub(crate) enum RebornStorageInput {
    Disabled,
    LocalDev {
        root: PathBuf,
    },
    #[cfg(feature = "libsql")]
    Libsql {
        db: Arc<libsql::Database>,
        path_or_url: String,
        auth_token: Option<SecretMaterial>,
        secret_master_key: SecretMaterial,
    },
    #[cfg(feature = "postgres")]
    Postgres {
        pool: deadpool_postgres::Pool,
        url: SecretMaterial,
        secret_master_key: SecretMaterial,
    },
}

impl RebornBuildInput {
    pub fn disabled(owner_id: impl Into<String>) -> Self {
        Self::new(
            RebornCompositionProfile::Disabled,
            owner_id,
            RebornStorageInput::Disabled,
        )
    }

    pub fn local_dev(owner_id: impl Into<String>, root: PathBuf) -> Self {
        Self::new(
            RebornCompositionProfile::LocalDev,
            owner_id,
            RebornStorageInput::LocalDev { root },
        )
    }

    #[cfg(feature = "libsql")]
    pub fn libsql(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        db: Arc<libsql::Database>,
        path_or_url: impl Into<String>,
        auth_token: Option<SecretMaterial>,
        secret_master_key: SecretMaterial,
    ) -> Self {
        Self::new(
            profile,
            owner_id,
            RebornStorageInput::Libsql {
                db,
                path_or_url: path_or_url.into(),
                auth_token,
                secret_master_key,
            },
        )
    }

    #[cfg(feature = "postgres")]
    pub fn postgres(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        pool: deadpool_postgres::Pool,
        url: SecretMaterial,
        secret_master_key: SecretMaterial,
    ) -> Self {
        Self::new(
            profile,
            owner_id,
            RebornStorageInput::Postgres {
                pool,
                url,
                secret_master_key,
            },
        )
    }

    pub fn with_required_runtime_backends(
        mut self,
        backends: impl IntoIterator<Item = ironclaw_host_api::RuntimeKind>,
    ) -> Self {
        self.required_runtime_backends = backends.into_iter().collect();
        self
    }

    pub fn require_runtime_http_egress(mut self) -> Self {
        self.require_runtime_http_egress = true;
        self
    }

    pub fn require_wasm_credentials(mut self) -> Self {
        self.require_wasm_credentials = true;
        self
    }

    fn new(
        profile: RebornCompositionProfile,
        owner_id: impl Into<String>,
        storage: RebornStorageInput,
    ) -> Self {
        Self {
            profile,
            owner_id: owner_id.into(),
            storage,
            required_runtime_backends: Vec::new(),
            require_runtime_http_egress: false,
            require_wasm_credentials: false,
        }
    }
}
