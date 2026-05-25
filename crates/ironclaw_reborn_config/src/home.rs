use std::{
    env, error,
    ffi::OsString,
    fmt, io,
    path::{Component, Path, PathBuf},
};

/// Environment variable that selects the standalone Reborn state root.
pub const REBORN_HOME_ENV: &str = "IRONCLAW_REBORN_HOME";

const V1_BASE_DIR_ENV: &str = "IRONCLAW_BASE_DIR";

/// Source used to resolve [`RebornHome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornHomeSource {
    Env,
    Default,
}

impl RebornHomeSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Env => REBORN_HOME_ENV,
            Self::Default => "default",
        }
    }
}

/// Resolved, validated state root for the standalone Reborn binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornHome {
    path: PathBuf,
    source: RebornHomeSource,
}

impl RebornHome {
    pub fn resolve_from_env() -> Result<Self, RebornConfigError> {
        Self::resolve_from_env_parts_with_v1_base(
            env::var_os(REBORN_HOME_ENV),
            env::var_os("HOME"),
            env::var_os("USERPROFILE"),
            env::var_os(V1_BASE_DIR_ENV),
        )
    }

    pub fn resolve_from_env_parts(
        reborn_home: Option<OsString>,
        home: Option<OsString>,
        userprofile: Option<OsString>,
    ) -> Result<Self, RebornConfigError> {
        Self::resolve_from_env_parts_with_v1_base(reborn_home, home, userprofile, None)
    }

    fn resolve_from_env_parts_with_v1_base(
        reborn_home: Option<OsString>,
        home: Option<OsString>,
        userprofile: Option<OsString>,
        v1_base_dir: Option<OsString>,
    ) -> Result<Self, RebornConfigError> {
        if let Some(raw_home) = reborn_home {
            validate_non_empty(&raw_home, REBORN_HOME_ENV)?;
            let path = PathBuf::from(raw_home);
            validate_absolute(&path, REBORN_HOME_ENV)?;
            validate_no_parent_components(&path, REBORN_HOME_ENV)?;
            validate_not_root(&path, REBORN_HOME_ENV)?;
            validate_not_v1_state_root(
                &path,
                home.as_ref(),
                userprofile.as_ref(),
                v1_base_dir.as_ref(),
            )?;
            return Ok(Self {
                path,
                source: RebornHomeSource::Env,
            });
        }

        let mut first_error = None;
        for (raw_home, label) in [(home, "HOME"), (userprofile, "USERPROFILE")] {
            let Some(raw_home) = raw_home else {
                continue;
            };
            match default_home_from_candidate(raw_home, label) {
                Ok(path) => {
                    return Ok(Self {
                        path,
                        source: RebornHomeSource::Default,
                    });
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }

        Err(first_error.unwrap_or(RebornConfigError::MissingHome))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn into_path(self) -> PathBuf {
        self.path
    }

    pub fn source(&self) -> RebornHomeSource {
        self.source
    }

    pub fn source_label(&self) -> &'static str {
        self.source.label()
    }

    /// Absolute path of the operator-edited boot config TOML.
    ///
    /// `$IRONCLAW_REBORN_HOME/config.toml`. The file is **optional**:
    /// `RebornConfigFile::load` returns `Ok(None)` when it doesn't
    /// exist, and the runtime falls back to compiled-in defaults.
    pub fn config_file_path(&self) -> PathBuf {
        self.path.join("config.toml")
    }

    /// Absolute path of the user-overlay LLM provider catalog.
    ///
    /// `$IRONCLAW_REBORN_HOME/providers.json`. Same JSON shape as v1's
    /// `~/.ironclaw/providers.json` so the same editor tooling and
    /// operator muscle memory apply. The file is **optional**: when
    /// it's missing, the runtime uses only the compiled-in built-in
    /// provider definitions. Loading happens in the composition root
    /// via `ironclaw_llm::ProviderRegistry`; this accessor just hands
    /// out the path.
    pub fn providers_file_path(&self) -> PathBuf {
        self.path.join("providers.json")
    }
}

fn validate_non_empty(value: &OsString, name: &'static str) -> Result<(), RebornConfigError> {
    if value.as_os_str().is_empty() {
        return Err(RebornConfigError::EmptyPath { name });
    }
    Ok(())
}

fn default_home_from_candidate(
    raw_home: OsString,
    label: &'static str,
) -> Result<PathBuf, RebornConfigError> {
    validate_non_empty(&raw_home, label)?;
    let path = PathBuf::from(raw_home);
    validate_absolute(&path, label)?;
    validate_no_parent_components(&path, label)?;
    validate_not_root(&path, label)?;
    Ok(path.join(".ironclaw").join("reborn"))
}

fn validate_absolute(path: &Path, name: &'static str) -> Result<(), RebornConfigError> {
    if !path.is_absolute() {
        return Err(RebornConfigError::RelativePath {
            name,
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn validate_no_parent_components(path: &Path, name: &'static str) -> Result<(), RebornConfigError> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(RebornConfigError::ParentPath {
            name,
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn validate_not_v1_state_root(
    path: &Path,
    home: Option<&OsString>,
    userprofile: Option<&OsString>,
    v1_base_dir: Option<&OsString>,
) -> Result<(), RebornConfigError> {
    let home_candidate = home.and_then(default_v1_state_root_from_home);
    let userprofile_candidate = userprofile.and_then(default_v1_state_root_from_home);
    let explicit_base_candidate = v1_base_dir.and_then(v1_state_root_from_base_dir);

    for candidate in [
        home_candidate.as_deref(),
        userprofile_candidate.as_deref(),
        explicit_base_candidate.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if paths_overlap(path, candidate) {
            return Err(RebornConfigError::V1StateRoot {
                name: REBORN_HOME_ENV,
                path: path.to_path_buf(),
            });
        }
    }

    Ok(())
}

fn default_v1_state_root_from_home(raw_home: &OsString) -> Option<PathBuf> {
    validated_absolute_candidate(raw_home).map(|path| path.join(".ironclaw"))
}

fn v1_state_root_from_base_dir(raw_base_dir: &OsString) -> Option<PathBuf> {
    if raw_base_dir.as_os_str().is_empty() {
        return None;
    }
    let path = PathBuf::from(raw_base_dir);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return None;
    }
    if path.is_absolute() {
        return Some(path);
    }
    env::current_dir().ok().map(|cwd| cwd.join(path))
}

fn validated_absolute_candidate(raw_path: &OsString) -> Option<PathBuf> {
    if raw_path.as_os_str().is_empty() {
        return None;
    }
    let path = PathBuf::from(raw_path);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return None;
    }
    Some(path)
}

fn paths_overlap(path: &Path, candidate: &Path) -> bool {
    if path == candidate {
        return true;
    }
    if existing_canonical_pair(path, candidate).is_some_and(|(path, candidate)| path == candidate) {
        return true;
    }
    match (
        normalize_existing_prefix(path),
        normalize_existing_prefix(candidate),
    ) {
        (Some(path), Some(candidate)) => path == candidate,
        _ => false,
    }
}

fn existing_canonical_pair(path: &Path, candidate: &Path) -> Option<(PathBuf, PathBuf)> {
    match (path.canonicalize(), candidate.canonicalize()) {
        (Ok(path), Ok(candidate)) => Some((path, candidate)),
        (Err(error), _) | (_, Err(error)) if missing_path_error(&error) => None,
        _ => None,
    }
}

fn normalize_existing_prefix(path: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors() {
        if let Ok(canonical_ancestor) = ancestor.canonicalize() {
            let suffix = path.strip_prefix(ancestor).ok()?;
            return Some(canonical_ancestor.join(suffix));
        }
    }
    None
}

fn missing_path_error(error: &io::Error) -> bool {
    matches!(error.kind(), io::ErrorKind::NotFound)
}

fn validate_not_root(path: &Path, name: &'static str) -> Result<(), RebornConfigError> {
    if path.parent().is_none() {
        return Err(RebornConfigError::RootPath {
            name,
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// Error returned when standalone Reborn boot configuration is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornConfigError {
    EmptyPath { name: &'static str },
    RelativePath { name: &'static str, path: PathBuf },
    ParentPath { name: &'static str, path: PathBuf },
    RootPath { name: &'static str, path: PathBuf },
    V1StateRoot { name: &'static str, path: PathBuf },
    MissingHome,
    InvalidProfile { name: &'static str, value: String },
}

impl fmt::Display for RebornConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath { name } => write!(formatter, "{name} must not be empty"),
            Self::RelativePath { name, .. } => write!(formatter, "{name} must be an absolute path"),
            Self::ParentPath { name, .. } => {
                write!(
                    formatter,
                    "{name} must not contain parent directory components"
                )
            }
            Self::RootPath { name, .. } => write!(formatter, "{name} must not be filesystem root"),
            Self::V1StateRoot { name, .. } => {
                write!(
                    formatter,
                    "{name} must not point at the v1 IronClaw state root"
                )
            }
            Self::MissingHome => write!(
                formatter,
                "HOME or USERPROFILE must be set when {REBORN_HOME_ENV} is unset"
            ),
            Self::InvalidProfile { name, value } => write!(
                formatter,
                "{name} must be one of local-dev, local-dev-yolo, production, migration-dry-run; got {value:?}"
            ),
        }
    }
}

impl error::Error for RebornConfigError {}
