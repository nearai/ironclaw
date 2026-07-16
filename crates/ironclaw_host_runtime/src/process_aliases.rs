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

/// Rewrite real host paths in command OUTPUT back to their virtual aliases.
///
/// `rewrite_local_host_command_aliases` rewrites `/workspace` to the real host
/// path *before* execution, so any program that echoes a path it was handed
/// prints the host path — e.g. `print("saved /workspace/out.pdf")` actually runs
/// as `print("saved /Users/alice/proj/out.pdf")`. Left untouched, that host path
/// flows back into the model-facing output and the user-visible reply, leaking
/// host layout and defeating `/workspace`-path download detection. This is the
/// inverse pass: it maps the longest-matching host-path prefix in `output` back
/// to its virtual alias so model-facing output only ever speaks in alias terms.
pub(crate) fn rewrite_local_host_output_aliases(
    output: &str,
    aliases: &[LocalHostWorkdirAlias],
) -> String {
    if aliases.is_empty() || output.is_empty() {
        return output.to_string();
    }
    // Most specific (longest host path) first: when `/workspace` nests under the
    // same root as `/host` (host home `/Users/alice`, workspace
    // `/Users/alice/proj`), the deeper mapping must win so `/Users/alice/proj/x`
    // becomes `/workspace/x`, not `/host/proj/x`.
    let mut ordered: Vec<&LocalHostWorkdirAlias> = aliases.iter().collect();
    ordered.sort_by_key(|alias| std::cmp::Reverse(alias.host_path.as_os_str().len()));
    let mut result = output.to_string();
    for alias in ordered {
        let Some(host_path) = alias.host_path.to_str() else {
            continue;
        };
        result = rewrite_host_path_prefix(&result, host_path, &alias.alias);
    }
    result
}

/// Replace every boundary-aligned occurrence of `host_path` in `haystack` with
/// `alias`. A match rewrites only when it starts at a path boundary and is either
/// a whole path token or the prefix of a deeper path (next char is `/`, the end,
/// or a non-path char) — so a sibling like `/Users/alice/proj-backup` is left
/// alone. Boundary logic mirrors the forward command rewriter.
fn rewrite_host_path_prefix(haystack: &str, host_path: &str, alias: &str) -> String {
    if host_path.is_empty() {
        return haystack.to_string();
    }
    let mut result = String::with_capacity(haystack.len());
    let mut search_start = 0;
    while let Some(rel_idx) = haystack[search_start..].find(host_path) {
        let idx = search_start + rel_idx;
        let end = idx + host_path.len();
        result.push_str(&haystack[search_start..idx]);
        if command_alias_start_boundary(haystack, idx) && command_alias_end_boundary(haystack, end)
        {
            result.push_str(alias);
        } else {
            result.push_str(host_path);
        }
        search_start = end;
    }
    result.push_str(&haystack[search_start..]);
    result
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

    #[test]
    fn output_alias_rewrite_maps_host_subpath_back_to_alias() {
        let aliases = [alias("/workspace", "/Users/alice/proj")];

        assert_eq!(
            rewrite_local_host_output_aliases("PDF created at /Users/alice/proj/out.pdf", &aliases),
            "PDF created at /workspace/out.pdf"
        );
    }

    #[test]
    fn output_alias_rewrite_maps_exact_host_path_at_boundaries() {
        let aliases = [alias("/workspace", "/Users/alice/proj")];

        // end-of-string, whitespace, and quote boundaries all rewrite the bare
        // host path (no trailing subpath).
        assert_eq!(
            rewrite_local_host_output_aliases("cwd is /Users/alice/proj", &aliases),
            "cwd is /workspace"
        );
        assert_eq!(
            rewrite_local_host_output_aliases("cwd: /Users/alice/proj\nok", &aliases),
            "cwd: /workspace\nok"
        );
        assert_eq!(
            rewrite_local_host_output_aliases("\"/Users/alice/proj\"", &aliases),
            "\"/workspace\""
        );
    }

    #[test]
    fn output_alias_rewrite_leaves_sibling_paths_untouched() {
        let aliases = [alias("/workspace", "/Users/alice/proj")];

        // `proj-backup` is a different directory that merely shares the prefix.
        assert_eq!(
            rewrite_local_host_output_aliases("/Users/alice/proj-backup/x", &aliases),
            "/Users/alice/proj-backup/x"
        );
    }

    #[test]
    fn output_alias_rewrite_prefers_longest_host_path() {
        // Host home and workspace share a root; the deeper workspace mapping wins
        // for paths inside it, and the home mapping covers the rest.
        let aliases = [
            alias("/host", "/Users/alice"),
            alias("/workspace", "/Users/alice/proj"),
        ];

        assert_eq!(
            rewrite_local_host_output_aliases("/Users/alice/proj/out.pdf", &aliases),
            "/workspace/out.pdf"
        );
        assert_eq!(
            rewrite_local_host_output_aliases("/Users/alice/notes.txt", &aliases),
            "/host/notes.txt"
        );
    }

    #[test]
    fn output_alias_rewrite_handles_multiple_occurrences() {
        let aliases = [alias("/workspace", "/Users/alice/proj")];

        assert_eq!(
            rewrite_local_host_output_aliases(
                "a /Users/alice/proj/x and /Users/alice/proj/y",
                &aliases
            ),
            "a /workspace/x and /workspace/y"
        );
    }

    #[test]
    fn output_alias_rewrite_noop_without_aliases() {
        assert_eq!(
            rewrite_local_host_output_aliases("/Users/alice/proj/out.pdf", &[]),
            "/Users/alice/proj/out.pdf"
        );
    }
}
