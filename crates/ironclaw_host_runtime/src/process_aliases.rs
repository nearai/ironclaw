use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalHostWorkdirAlias {
    alias: String,
    host_path: PathBuf,
}

impl LocalHostWorkdirAlias {
    /// Create a new alias mapping.
    ///
    /// `host_path` must be an absolute, canonicalized host path from a trusted source
    /// (e.g. from `std::fs::canonicalize`). Non-canonicalized or relative paths may
    /// produce silently invalid mappings.
    pub(crate) fn try_new(
        alias: impl Into<String>,
        host_path: impl Into<PathBuf>,
    ) -> Result<Self, String> {
        let alias = normalize_alias(alias.into())?;
        let host_path = host_path.into();
        if !host_path.is_absolute() {
            return Err("local host workdir alias host_path must be absolute".to_string());
        }
        Ok(Self { alias, host_path })
    }

    fn resolve(&self, workdir: &str) -> Option<PathBuf> {
        if workdir == self.alias {
            return Some(PathBuf::new());
        }
        let relative = workdir.strip_prefix(&format!("{}/", self.alias))?;
        relative_workdir_tail(relative)
    }
}

pub(crate) fn resolve_local_host_workdir(
    workdir: Option<&str>,
    workdir_aliases: &[LocalHostWorkdirAlias],
) -> std::io::Result<PathBuf> {
    let Some(workdir) = workdir else {
        return std::env::current_dir();
    };
    if let Some((alias, relative)) = workdir_aliases
        .iter()
        .filter_map(|alias| alias.resolve(workdir).map(|relative| (alias, relative)))
        .max_by_key(|(alias, _)| alias.alias.len())
    {
        return Ok(alias.host_path.join(relative));
    }
    Ok(PathBuf::from(workdir))
}

pub(crate) fn rewrite_local_host_command_aliases(
    command: &str,
    aliases: &[LocalHostWorkdirAlias],
) -> String {
    // NOTE: This rewriter handles single-quote, double-quote, and escape
    // contexts but does NOT model command-substitution context reset.
    // Inside `$(...)` the shell restarts quoting from scratch, so a command
    // like `printf "$(cat '/workspace/file')"` may misquote the rewritten
    // path. Paths in local-dev-yolo mode are trusted, so this produces a
    // misquoted but not dangerous rewrite. Full $(...) tracking is left as
    // a future enhancement.
    if aliases.is_empty() {
        return command.to_string();
    }
    let mut rewritten = String::with_capacity(command.len());
    let mut index = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;
    while index < command.len() {
        let Some(current) = command[index..].chars().next() else {
            break;
        };
        let current_len = current.len_utf8();
        if !escaped && let Some(alias) = longest_matching_command_alias(command, index, aliases) {
            push_rewritten_alias(&mut rewritten, alias, in_single_quote, in_double_quote);
            index += alias.alias.len();
            continue;
        }
        rewritten.push(current);
        if escaped {
            escaped = false;
        } else if current == '\\' && !in_single_quote {
            escaped = true;
        } else if current == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if current == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        }
        index += current_len;
    }
    rewritten
}

fn normalize_alias(alias: String) -> Result<String, String> {
    let alias = alias.trim_end_matches('/').to_string();
    if alias.is_empty() || alias == "/" {
        return Err("local host workdir alias must name a non-root absolute path".to_string());
    }
    let path = Path::new(&alias);
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err("local host workdir alias must be absolute".to_string());
    }
    if !components.all(|component| matches!(component, Component::Normal(_))) {
        return Err("local host workdir alias must not contain prefix, root, . or ..".to_string());
    }
    Ok(alias)
}

fn relative_workdir_tail(relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    if path.components().any(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        )
    }) {
        return None;
    }
    Some(path.to_path_buf())
}

fn push_rewritten_alias(
    rewritten: &mut String,
    alias: &LocalHostWorkdirAlias,
    in_single_quote: bool,
    in_double_quote: bool,
) {
    let raw = alias.host_path.display().to_string();
    if in_single_quote {
        rewritten.push_str(&escape_for_single_quote_context(&raw));
    } else if in_double_quote {
        rewritten.push_str(&escape_for_double_quote_context(&raw));
    } else {
        rewritten.push_str(&shell_quote_path(&raw));
    }
}

fn longest_matching_command_alias<'a>(
    command: &str,
    index: usize,
    aliases: &'a [LocalHostWorkdirAlias],
) -> Option<&'a LocalHostWorkdirAlias> {
    aliases
        .iter()
        .filter(|alias| {
            command[index..].starts_with(&alias.alias)
                && command_alias_start_boundary(command, index)
                && command_alias_end_boundary(command, index + alias.alias.len())
        })
        .max_by_key(|alias| alias.alias.len())
}

fn command_alias_start_boundary(command: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    command[..index]
        .chars()
        .next_back()
        .is_none_or(|ch| !command_path_char(ch))
}

fn command_alias_end_boundary(command: &str, index: usize) -> bool {
    if index >= command.len() {
        return true;
    }
    command[index..]
        .chars()
        .next()
        .is_some_and(|ch| ch == '/' || !command_path_char(ch))
}

fn command_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-' | '.')
}

fn shell_quote_path(path: &str) -> String {
    format!("'{}'", escape_for_single_quote_context(path))
}

fn escape_for_single_quote_context(path: &str) -> String {
    path.replace('\'', "'\\''")
}

fn escape_for_double_quote_context(path: &str) -> String {
    let mut escaped = String::with_capacity(path.len());
    for ch in path.chars() {
        if matches!(ch, '"' | '\\' | '$' | '`') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alias(alias: &str, host_path: &str) -> LocalHostWorkdirAlias {
        LocalHostWorkdirAlias::try_new(alias, host_path).expect("valid alias")
    }

    #[test]
    fn alias_normalization_requires_non_root_absolute_paths() {
        assert!(LocalHostWorkdirAlias::try_new("workspace", "/tmp/workspace").is_err());
        assert!(LocalHostWorkdirAlias::try_new("/", "/tmp/workspace").is_err());
        assert!(LocalHostWorkdirAlias::try_new("/workspace/..", "/tmp/workspace").is_err());
        assert_eq!(
            LocalHostWorkdirAlias::try_new("/workspace/", "/tmp/workspace")
                .expect("valid alias")
                .alias,
            "/workspace"
        );
    }

    #[test]
    fn workspace_workdir_translation_rejects_parent_escape() {
        let alias = alias("/workspace", "/tmp/workspace");
        assert_eq!(alias.resolve("/workspace/../outside"), None);
    }

    #[test]
    fn command_alias_rewrite_ignores_embedded_path_fragments() {
        let alias = alias("/host", "/Users/alice");

        assert_eq!(
            rewrite_local_host_command_aliases("printf http://host.test/hosted", &[alias]),
            "printf http://host.test/hosted"
        );
    }

    #[test]
    fn command_alias_rewrite_quotes_unquoted_paths_with_spaces() {
        let alias = alias("/workspace", "/tmp/work space");

        assert_eq!(
            rewrite_local_host_command_aliases("ls /workspace/qa", &[alias]),
            "ls '/tmp/work space'/qa"
        );
    }

    #[test]
    fn command_alias_rewrite_escapes_single_quote_context() {
        let alias = alias("/workspace", "/tmp/work'space");

        assert_eq!(
            rewrite_local_host_command_aliases("printf '%s' '/workspace/qa'", &[alias]),
            "printf '%s' '/tmp/work'\\''space/qa'"
        );
    }

    #[test]
    fn command_alias_rewrite_escapes_double_quote_context() {
        let alias = alias("/workspace", "/tmp/work$space`quoted`");

        assert_eq!(
            rewrite_local_host_command_aliases("printf \"%s\" \"/workspace/qa\"", &[alias]),
            "printf \"%s\" \"/tmp/work\\$space\\`quoted\\`/qa\""
        );
    }

    #[test]
    fn command_alias_rewrite_respects_escaped_aliases() {
        let alias = alias("/workspace", "/tmp/workspace");

        assert_eq!(
            rewrite_local_host_command_aliases("printf \\/workspace /workspace", &[alias]),
            "printf \\/workspace '/tmp/workspace'"
        );
    }

    #[test]
    fn command_alias_rewrite_at_start_of_command() {
        let alias = alias("/workspace", "/tmp/workspace");

        assert_eq!(
            rewrite_local_host_command_aliases("/workspace/run.sh", &[alias]),
            "'/tmp/workspace'/run.sh"
        );
    }

    #[test]
    fn command_alias_rewrite_followed_by_semicolon_boundary() {
        let alias = alias("/workspace", "/tmp/workspace");

        assert_eq!(
            rewrite_local_host_command_aliases("cd /workspace; ls", &[alias]),
            "cd '/tmp/workspace'; ls"
        );
    }

    #[test]
    fn command_alias_rewrite_handles_command_substitution_paths() {
        let alias = alias("/workspace", "/tmp/workspace");

        assert_eq!(
            rewrite_local_host_command_aliases("printf \"$(cat /workspace/file)\"", &[alias]),
            "printf \"$(cat /tmp/workspace/file)\""
        );
    }
}
