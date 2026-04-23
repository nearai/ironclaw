//! Repository/workspace label resolution for the status bar.
//!
//! Produces a short label shown as the leading `⌂ {label}` segment of the
//! status bar: the current git branch when inside a repository, otherwise a
//! shortened display of the working directory.

use std::path::{Path, PathBuf};

const HOME_TILDE: &str = "~";

/// Compute the status-bar label for `path`.
///
/// Resolution order:
/// 1. If `path` (or any ancestor) contains a `.git` directory, return the
///    current branch name (or the short SHA when detached).
/// 2. Otherwise fall back to the final path component, with `$HOME`
///    collapsed to `~`.
/// 3. If `path` is empty, return an empty string.
pub fn compute_repo_label(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }

    let p = PathBuf::from(path);
    if let Some(branch) = find_git_branch(&p) {
        return branch;
    }

    cwd_label(&p)
}

fn find_git_branch(start: &Path) -> Option<String> {
    let mut current = Some(start);
    while let Some(dir) = current {
        let git_path = dir.join(".git");
        if git_path.is_dir() {
            return read_head_branch(&git_path);
        }
        // Worktree: .git is a file containing `gitdir: ...`
        if git_path.is_file()
            && let Ok(contents) = std::fs::read_to_string(&git_path)
            && let Some(gitdir) = contents.strip_prefix("gitdir: ")
        {
            let gitdir = PathBuf::from(gitdir.trim());
            return read_head_branch(&gitdir);
        }
        current = dir.parent();
    }
    None
}

fn read_head_branch(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(rest) = head.strip_prefix("ref: ") {
        let branch = rest.rsplit('/').next().unwrap_or(rest);
        // Keep the full ref tail after refs/heads/ so slashes in branch names
        // (e.g. `feat/tui-status-bar`) survive.
        let stripped = rest.strip_prefix("refs/heads/").unwrap_or(branch);
        return Some(stripped.to_string());
    }
    // Detached head: show a short SHA.
    if head.len() >= 7 {
        return Some(head[..7].to_string());
    }
    None
}

fn cwd_label(path: &Path) -> String {
    let display = path.display().to_string();
    let with_tilde = if let Some(home) = std::env::var_os("HOME") {
        let home_str = home.to_string_lossy().to_string();
        if !home_str.is_empty() && display.starts_with(&home_str) {
            format!("{HOME_TILDE}{}", &display[home_str.len()..])
        } else {
            display
        }
    } else {
        display
    };

    // Prefer the last meaningful component so the badge stays short.
    if let Some(last) = Path::new(&with_tilde).file_name() {
        return last.to_string_lossy().to_string();
    }
    with_tilde
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn empty_path_yields_empty_label() {
        assert_eq!(compute_repo_label(""), "");
    }

    #[test]
    fn non_git_dir_falls_back_to_basename() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("my-project");
        fs::create_dir_all(&dir).unwrap();
        let label = compute_repo_label(&dir.display().to_string());
        assert_eq!(label, "my-project");
    }

    #[test]
    fn git_repo_returns_branch_from_head_file() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let git = repo.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/feat/my-feature\n").unwrap();

        let label = compute_repo_label(&repo.display().to_string());
        assert_eq!(label, "feat/my-feature");
    }

    #[test]
    fn detached_head_returns_short_sha() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let git = repo.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(
            git.join("HEAD"),
            "abcdef1234567890abcdef1234567890abcdef12\n",
        )
        .unwrap();

        let label = compute_repo_label(&repo.display().to_string());
        assert_eq!(label, "abcdef1");
    }

    #[test]
    fn worktree_gitdir_file_is_followed() {
        let tmp = tempdir().unwrap();
        let main_git = tmp.path().join("main").join(".git");
        let wt_dir = main_git.join("worktrees").join("feature");
        fs::create_dir_all(&wt_dir).unwrap();
        fs::write(wt_dir.join("HEAD"), "ref: refs/heads/feature-branch\n").unwrap();

        let worktree = tmp.path().join("feature");
        fs::create_dir_all(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", wt_dir.display()),
        )
        .unwrap();

        let label = compute_repo_label(&worktree.display().to_string());
        assert_eq!(label, "feature-branch");
    }

    #[test]
    fn walks_up_to_find_git_dir() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let nested = repo.join("crates").join("sub");
        fs::create_dir_all(&nested).unwrap();
        let git = repo.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let label = compute_repo_label(&nested.display().to_string());
        assert_eq!(label, "main");
    }
}
