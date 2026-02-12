//! Permission enforcement for skill tool calls.
//!
//! Enforces parameter-level constraints declared in `skill.toml` `[permissions]`
//! sections. Builds on the coarse-grained attenuation (which removes tools from
//! the LLM tool list) by adding fine-grained pattern matching on tool parameters.
//!
//! # Enforcement semantics
//!
//! - If **no** active skill declares patterns for a tool, all calls pass through.
//! - If **any** active Verified/Local skill declares patterns, the call must match
//!   at least one pattern from at least one skill (union semantics).
//! - Community skills' patterns are silently ignored (defense in depth).
//! - Local skills **without** patterns for a tool leave that tool unrestricted.
//! - HTTP tools are handled by `http_scoping.rs` and are not covered here.

use std::collections::HashMap;
use std::fmt;

use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::skills::{
    FilePathPattern, LoadedSkill, MemoryTargetPattern, ShellPattern, SkillTrust, ToolPattern,
};

// ---------------------------------------------------------------------------
// Serializable permission DTOs (cross the HTTP boundary to workers)
// ---------------------------------------------------------------------------

/// A serialized tool permission that can be sent over HTTP to workers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedToolPermission {
    pub tool_name: String,
    pub trust: SkillTrust,
    pub patterns: Vec<SerializedPattern>,
}

/// A serialized pattern variant with explicit tags for unambiguous JSON round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SerializedPattern {
    Shell { command: String },
    FilePath { path: String },
    MemoryTarget { target: String },
}

impl SerializedToolPermission {
    /// Extract serializable permissions from active skills.
    ///
    /// Community skills are excluded (defense in depth).
    /// Each skill's tool permissions are flattened into a list of per-tool entries.
    pub fn from_active_skills(skills: &[LoadedSkill]) -> Vec<Self> {
        let mut result = Vec::new();

        for skill in skills {
            // Defense in depth: exclude community skill patterns
            if skill.trust == SkillTrust::Community {
                continue;
            }

            for (tool_name, decl) in &skill.manifest.permissions {
                // HTTP is handled by http_scoping.rs
                if tool_name == "http" {
                    continue;
                }

                let patterns: Vec<SerializedPattern> = decl
                    .allowed_patterns
                    .iter()
                    .map(|p| match p {
                        ToolPattern::Shell(ShellPattern { command }) => SerializedPattern::Shell {
                            command: command.clone(),
                        },
                        ToolPattern::FilePath(FilePathPattern { path }) => {
                            SerializedPattern::FilePath { path: path.clone() }
                        }
                        ToolPattern::MemoryTarget(MemoryTargetPattern { target }) => {
                            SerializedPattern::MemoryTarget {
                                target: target.clone(),
                            }
                        }
                    })
                    .collect();

                result.push(SerializedToolPermission {
                    tool_name: tool_name.clone(),
                    trust: skill.trust,
                    patterns,
                });
            }
        }

        result
    }
}

/// Maximum compiled regex size (64 KiB) to prevent ReDoS.
const MAX_REGEX_SIZE: usize = 1 << 16;

/// Allowed characters in glob patterns (alphanumeric + common path/filename chars + wildcard).
fn is_valid_pattern_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '*' | ' ')
}

/// Convert a glob pattern to a regex string.
///
/// - `*` matches anything except `/`
/// - `**` matches anything including `/`
/// - All regex-special characters are escaped.
/// - Pattern characters are restricted to a safe set.
fn glob_to_regex(glob: &str) -> Result<String, PatternError> {
    // Validate characters
    for c in glob.chars() {
        if !is_valid_pattern_char(c) {
            return Err(PatternError::InvalidCharacter {
                pattern: glob.to_string(),
                character: c,
            });
        }
    }

    let mut regex = String::with_capacity(glob.len() * 2 + 2);
    regex.push('^');

    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            if i < chars.len() && chars[i] == '/' {
                // **/ matches zero or more directory segments
                regex.push_str("(.*/)?");
                i += 1;
            } else {
                // ** at end or before non-slash matches anything including /
                regex.push_str(".*");
            }
        } else if chars[i] == '*' {
            // * matches anything except /
            regex.push_str("[^/]*");
            i += 1;
        } else {
            // Escape regex-special characters
            let c = chars[i];
            if ".+^${}()|[]\\".contains(c) {
                regex.push('\\');
            }
            regex.push(c);
            i += 1;
        }
    }

    regex.push('$');
    Ok(regex)
}

/// A compiled pattern ready for matching.
struct CompiledPattern {
    regex: regex::Regex,
}

impl CompiledPattern {
    fn new(glob: &str) -> Result<Self, PatternError> {
        let regex_str = glob_to_regex(glob)?;
        let regex = RegexBuilder::new(&regex_str)
            .size_limit(MAX_REGEX_SIZE)
            .build()
            .map_err(|e| PatternError::CompilationFailed {
                pattern: glob.to_string(),
                reason: e.to_string(),
            })?;
        Ok(Self { regex })
    }

    fn matches(&self, value: &str) -> bool {
        self.regex.is_match(value)
    }
}

/// Per-tool compiled patterns from a single skill.
struct ToolPatterns {
    shell: Vec<CompiledPattern>,
    file_path: Vec<CompiledPattern>,
    memory_target: Vec<CompiledPattern>,
}

/// Errors from pattern compilation.
#[derive(Debug, Clone)]
pub enum PatternError {
    InvalidCharacter { pattern: String, character: char },
    CompilationFailed { pattern: String, reason: String },
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCharacter { pattern, character } => write!(
                f,
                "Invalid character '{}' in pattern '{}'",
                character, pattern
            ),
            Self::CompilationFailed { pattern, reason } => {
                write!(f, "Failed to compile pattern '{}': {}", pattern, reason)
            }
        }
    }
}

impl std::error::Error for PatternError {}

/// Errors from permission enforcement.
#[derive(Debug, Clone)]
pub enum PermissionError {
    /// Tool call denied because no active skill's patterns matched.
    Denied {
        tool_name: String,
        parameter: String,
        value: String,
        reason: String,
    },
}

impl fmt::Display for PermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied {
                tool_name,
                parameter,
                value,
                reason,
            } => {
                write!(
                    f,
                    "Permission denied for tool '{}': {} '{}' {}",
                    tool_name, parameter, value, reason
                )
            }
        }
    }
}

impl std::error::Error for PermissionError {}

/// Tracks whether a tool has patterns or is unrestricted.
enum ToolRestriction {
    /// At least one Local skill has no patterns for this tool, making it unrestricted.
    Unrestricted,
    /// Patterns exist and must be checked.
    Patterns(Vec<ToolPatterns>),
}

/// Aggregated permission enforcer built from active skills.
///
/// Constructed once per agentic loop iteration. Validates tool parameters
/// against declared patterns using union semantics across skills.
pub struct SkillPermissionEnforcer {
    /// Per-tool restrictions: tool_name -> restriction.
    tools: HashMap<String, ToolRestriction>,
    /// Whether any enforcement is active.
    has_enforcement: bool,
}

impl SkillPermissionEnforcer {
    /// Reconstruct an enforcer from serialized permission data (worker side).
    ///
    /// Recompiles glob patterns into regex. Invalid patterns are logged and skipped.
    /// Same trust rules as `from_active_skills()`: Community ignored, Local without
    /// patterns = unrestricted, union semantics.
    pub fn from_serialized(permissions: &[SerializedToolPermission]) -> Self {
        let mut tools: HashMap<String, ToolRestriction> = HashMap::new();

        for perm in permissions {
            // Defense in depth: ignore community skill patterns
            if perm.trust == SkillTrust::Community {
                continue;
            }

            let tool_name = &perm.tool_name;

            if perm.patterns.is_empty() {
                if perm.trust == SkillTrust::Local {
                    tools.insert(tool_name.clone(), ToolRestriction::Unrestricted);
                }
                // Verified with no patterns: no enforcement (backward compat).
                // NOTE: Changing this to deny-all for empty patterns was considered
                // but deferred -- it would require a migration path for existing
                // Verified skills that declare tools without patterns.
                continue;
            }

            // If already unrestricted due to a Local skill, skip
            if matches!(tools.get(tool_name), Some(ToolRestriction::Unrestricted)) {
                continue;
            }

            // Compile patterns
            let mut compiled = ToolPatterns {
                shell: Vec::new(),
                file_path: Vec::new(),
                memory_target: Vec::new(),
            };

            for pattern in &perm.patterns {
                match pattern {
                    SerializedPattern::Shell { command } => match CompiledPattern::new(command) {
                        Ok(cp) => compiled.shell.push(cp),
                        Err(e) => {
                            tracing::warn!(
                                tool_name = tool_name.as_str(),
                                "Bad serialized shell pattern: {}",
                                e
                            );
                        }
                    },
                    SerializedPattern::FilePath { path } => match CompiledPattern::new(path) {
                        Ok(cp) => compiled.file_path.push(cp),
                        Err(e) => {
                            tracing::warn!(
                                tool_name = tool_name.as_str(),
                                "Bad serialized file path pattern: {}",
                                e
                            );
                        }
                    },
                    SerializedPattern::MemoryTarget { target } => {
                        match CompiledPattern::new(target) {
                            Ok(cp) => compiled.memory_target.push(cp),
                            Err(e) => {
                                tracing::warn!(
                                    tool_name = tool_name.as_str(),
                                    "Bad serialized memory target pattern: {}",
                                    e
                                );
                            }
                        }
                    }
                }
            }

            match tools.get_mut(tool_name) {
                Some(ToolRestriction::Patterns(existing)) => {
                    existing.push(compiled);
                }
                _ => {
                    tools.insert(tool_name.clone(), ToolRestriction::Patterns(vec![compiled]));
                }
            }
        }

        let has_enforcement = tools
            .values()
            .any(|r| matches!(r, ToolRestriction::Patterns(_)));

        Self {
            tools,
            has_enforcement,
        }
    }

    /// Build an enforcer from active skills.
    ///
    /// Community skills' patterns are silently ignored.
    /// Local skills without patterns for a tool leave it unrestricted.
    ///
    /// Internally serializes to `SerializedToolPermission` and delegates to
    /// `from_serialized()` so both construction paths share the same logic.
    pub fn from_active_skills(skills: &[LoadedSkill]) -> Self {
        let serialized = SerializedToolPermission::from_active_skills(skills);
        Self::from_serialized(&serialized)
    }

    /// Whether any enforcement is active.
    pub fn has_enforcement(&self) -> bool {
        self.has_enforcement
    }

    /// Validate a tool call against active skill permissions.
    ///
    /// Returns `Ok(())` if the call is allowed, or `Err(PermissionError)` if denied.
    pub fn validate_tool_call(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
    ) -> Result<(), PermissionError> {
        // No enforcement active -- passthrough
        if !self.has_enforcement {
            return Ok(());
        }

        let restriction = match self.tools.get(tool_name) {
            None => return Ok(()), // No patterns declared for this tool
            Some(ToolRestriction::Unrestricted) => return Ok(()),
            Some(ToolRestriction::Patterns(patterns)) => patterns,
        };

        match tool_name {
            "shell" => {
                let command = params.get("command").and_then(|v| v.as_str()).unwrap_or("");
                self.check_patterns(restriction, tool_name, "command", command, |tp| &tp.shell)
            }
            "read_file" | "write_file" | "list_dir" | "apply_patch" => {
                let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
                self.check_patterns(restriction, tool_name, "path", path, |tp| &tp.file_path)
            }
            "memory_write" => {
                let target = params.get("target").and_then(|v| v.as_str()).unwrap_or("");
                self.check_patterns(restriction, tool_name, "target", target, |tp| {
                    &tp.memory_target
                })
            }
            // NOTE: memory_read and memory_search are intentionally unrestricted.
            // Read-only memory access is safe by design (attenuation already gates
            // tool availability). Write enforcement is the priority since writes
            // to identity files (SOUL.md, AGENTS.md) could alter agent behavior.
            _ => Ok(()), // Unknown tool with patterns -- passthrough
        }
    }

    /// Check if a value matches any pattern across all skills (union semantics).
    fn check_patterns(
        &self,
        patterns: &[ToolPatterns],
        tool_name: &str,
        param_name: &str,
        value: &str,
        extractor: impl Fn(&ToolPatterns) -> &Vec<CompiledPattern>,
    ) -> Result<(), PermissionError> {
        // Union: if ANY skill's patterns permit it, allow
        for tp in patterns {
            let compiled = extractor(tp);
            if compiled.is_empty() {
                continue;
            }
            for cp in compiled {
                if cp.matches(value) {
                    return Ok(());
                }
            }
        }

        // Check if all skills have empty pattern lists for this type
        // (e.g., shell patterns declared but only file_path patterns provided)
        let any_relevant_patterns = patterns.iter().any(|tp| !extractor(tp).is_empty());
        if !any_relevant_patterns {
            return Ok(());
        }

        Err(PermissionError::Denied {
            tool_name: tool_name.to_string(),
            parameter: param_name.to_string(),
            value: truncate_value(value, 100),
            reason: "does not match any active skill's allowed patterns".to_string(),
        })
    }
}

impl std::fmt::Debug for SkillPermissionEnforcer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillPermissionEnforcer")
            .field("has_enforcement", &self.has_enforcement)
            .field("tool_count", &self.tools.len())
            .finish()
    }
}

/// Truncate a value for inclusion in error messages.
/// Finds a safe UTF-8 boundary to avoid panicking on multi-byte characters.
fn truncate_value(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Walk backwards from max to find a char boundary
        let mut boundary = max;
        while boundary > 0 && !s.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}...", &s[..boundary])
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::skills::{
        ActivationCriteria, IntegrityInfo, LoadedSkill, SkillManifest, SkillMeta, SkillSource,
        SkillTrust, ToolPermissionDeclaration,
    };

    /// Helper to build a LoadedSkill with permissions.
    fn make_skill(
        name: &str,
        trust: SkillTrust,
        permissions: HashMap<String, ToolPermissionDeclaration>,
    ) -> LoadedSkill {
        LoadedSkill {
            manifest: SkillManifest {
                skill: SkillMeta {
                    name: name.to_string(),
                    version: "1.0.0".to_string(),
                    description: String::new(),
                    author: String::new(),
                    tags: vec![],
                },
                activation: ActivationCriteria::default(),
                permissions,
                integrity: IntegrityInfo::default(),
                http: None,
            },
            prompt_content: "test".to_string(),
            trust,
            source: SkillSource::Local(PathBuf::from("/tmp/test")),
            content_hash: "sha256:000".to_string(),
            scan_warnings: vec![],
            compiled_patterns: vec![],
        }
    }

    fn shell_patterns(patterns: &[&str]) -> ToolPermissionDeclaration {
        ToolPermissionDeclaration {
            reason: "test".to_string(),
            allowed_patterns: patterns
                .iter()
                .map(|p| {
                    ToolPattern::Shell(ShellPattern {
                        command: p.to_string(),
                    })
                })
                .collect(),
        }
    }

    fn file_patterns(patterns: &[&str]) -> ToolPermissionDeclaration {
        ToolPermissionDeclaration {
            reason: "test".to_string(),
            allowed_patterns: patterns
                .iter()
                .map(|p| {
                    ToolPattern::FilePath(FilePathPattern {
                        path: p.to_string(),
                    })
                })
                .collect(),
        }
    }

    fn memory_patterns(patterns: &[&str]) -> ToolPermissionDeclaration {
        ToolPermissionDeclaration {
            reason: "test".to_string(),
            allowed_patterns: patterns
                .iter()
                .map(|p| {
                    ToolPattern::MemoryTarget(MemoryTargetPattern {
                        target: p.to_string(),
                    })
                })
                .collect(),
        }
    }

    fn no_patterns() -> ToolPermissionDeclaration {
        ToolPermissionDeclaration {
            reason: "test".to_string(),
            allowed_patterns: vec![],
        }
    }

    fn params(kv: &[(&str, &str)]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in kv {
            map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
        }
        serde_json::Value::Object(map)
    }

    // -----------------------------------------------------------------------
    // glob_to_regex unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_glob_to_regex_star_matches_no_slash() {
        let re_str = glob_to_regex("cargo *").unwrap();
        let re = regex::Regex::new(&re_str).unwrap();
        assert!(re.is_match("cargo build"));
        assert!(re.is_match("cargo test"));
        assert!(!re.is_match("cargo build --release/extra"));
    }

    #[test]
    fn test_glob_to_regex_double_star() {
        let re_str = glob_to_regex("src/**/*.rs").unwrap();
        let re = regex::Regex::new(&re_str).unwrap();
        assert!(re.is_match("src/main.rs"));
        assert!(re.is_match("src/agent/mod.rs"));
        assert!(re.is_match("src/deep/nested/file.rs"));
        assert!(!re.is_match("other/main.rs"));
    }

    #[test]
    fn test_glob_to_regex_exact_match() {
        let re_str = glob_to_regex("cargo fmt").unwrap();
        let re = regex::Regex::new(&re_str).unwrap();
        assert!(re.is_match("cargo fmt"));
        assert!(!re.is_match("cargo fmt --check"));
        assert!(!re.is_match("malicious && cargo fmt"));
    }

    #[test]
    fn test_glob_to_regex_invalid_char() {
        let result = glob_to_regex("cargo; rm -rf /");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // No skills / no patterns passthrough
    // -----------------------------------------------------------------------

    #[test]
    fn test_no_skills_passthrough() {
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[]);
        assert!(!enforcer.has_enforcement());
        let result = enforcer.validate_tool_call("shell", &params(&[("command", "rm -rf /")]));
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_patterns_passthrough() {
        let skill = make_skill(
            "basic",
            SkillTrust::Verified,
            [("shell".to_string(), no_patterns())].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);
        assert!(!enforcer.has_enforcement());
        let result = enforcer.validate_tool_call("shell", &params(&[("command", "anything")]));
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // Shell pattern tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_shell_pattern_match() {
        let skill = make_skill(
            "builder",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["cargo *"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);
        assert!(enforcer.has_enforcement());

        // Allowed
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo build")]))
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo test")]))
                .is_ok()
        );
    }

    #[test]
    fn test_shell_pattern_deny() {
        let skill = make_skill(
            "builder",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["cargo *"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        // Denied
        let result = enforcer.validate_tool_call("shell", &params(&[("command", "rm -rf /")]));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Permission denied"));
        assert!(err.to_string().contains("shell"));
    }

    #[test]
    fn test_shell_exact_match() {
        let skill = make_skill(
            "formatter",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["cargo fmt"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo fmt")]))
                .is_ok()
        );
        // Extra args not matched by exact pattern
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo fmt --check")]))
                .is_err()
        );
    }

    // -----------------------------------------------------------------------
    // File path tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_file_path_single_star() {
        let skill = make_skill(
            "editor",
            SkillTrust::Verified,
            [("write_file".to_string(), file_patterns(&["src/*.rs"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        assert!(
            enforcer
                .validate_tool_call("write_file", &params(&[("path", "src/main.rs")]))
                .is_ok()
        );
        // Single * doesn't match nested paths
        assert!(
            enforcer
                .validate_tool_call("write_file", &params(&[("path", "src/agent/mod.rs")]))
                .is_err()
        );
    }

    #[test]
    fn test_file_path_double_star() {
        let skill = make_skill(
            "editor",
            SkillTrust::Verified,
            [("write_file".to_string(), file_patterns(&["src/**/*.rs"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        assert!(
            enforcer
                .validate_tool_call("write_file", &params(&[("path", "src/main.rs")]))
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("write_file", &params(&[("path", "src/agent/mod.rs")]))
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call(
                    "write_file",
                    &params(&[("path", "src/deep/nested/file.rs")])
                )
                .is_ok()
        );
        // Outside src/
        assert!(
            enforcer
                .validate_tool_call("write_file", &params(&[("path", "tests/test.rs")]))
                .is_err()
        );
    }

    #[test]
    fn test_file_path_applies_to_all_file_tools() {
        let mut perms = HashMap::new();
        perms.insert("read_file".to_string(), file_patterns(&["src/**/*.rs"]));
        perms.insert("write_file".to_string(), file_patterns(&["src/**/*.rs"]));
        perms.insert("list_dir".to_string(), file_patterns(&["src/**"]));
        perms.insert("apply_patch".to_string(), file_patterns(&["src/**/*.rs"]));

        let skill = make_skill("editor", SkillTrust::Verified, perms);
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        for tool in &["read_file", "write_file", "apply_patch"] {
            assert!(
                enforcer
                    .validate_tool_call(tool, &params(&[("path", "src/main.rs")]))
                    .is_ok()
            );
            assert!(
                enforcer
                    .validate_tool_call(tool, &params(&[("path", "/etc/passwd")]))
                    .is_err()
            );
        }
    }

    // -----------------------------------------------------------------------
    // Memory target tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_memory_target_pattern() {
        let skill = make_skill(
            "logger",
            SkillTrust::Verified,
            [("memory_write".to_string(), memory_patterns(&["daily/*"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        assert!(
            enforcer
                .validate_tool_call(
                    "memory_write",
                    &params(&[("target", "daily/2024-01-15.md")])
                )
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("memory_write", &params(&[("target", "SOUL.md")]))
                .is_err()
        );
    }

    // -----------------------------------------------------------------------
    // Union semantics across skills
    // -----------------------------------------------------------------------

    #[test]
    fn test_union_across_skills() {
        let skill1 = make_skill(
            "builder",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["cargo *"]))].into(),
        );
        let skill2 = make_skill(
            "linter",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["vale *"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill1, skill2]);

        // Both patterns available via union
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo build")]))
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "vale readme.md")]))
                .is_ok()
        );
        // Neither matches
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "rm -rf /")]))
                .is_err()
        );
    }

    // -----------------------------------------------------------------------
    // Trust tier behavior
    // -----------------------------------------------------------------------

    #[test]
    fn test_community_patterns_ignored() {
        let skill = make_skill(
            "sketchy",
            SkillTrust::Community,
            [("shell".to_string(), shell_patterns(&["rm *"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        // Community patterns ignored -- no enforcement active
        assert!(!enforcer.has_enforcement());
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "anything")]))
                .is_ok()
        );
    }

    #[test]
    fn test_verified_patterns_enforced() {
        let skill = make_skill(
            "trusted-builder",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["cargo *"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        assert!(enforcer.has_enforcement());
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo build")]))
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "npm install")]))
                .is_err()
        );
    }

    #[test]
    fn test_local_without_patterns_unrestricted() {
        let skill = make_skill(
            "my-tool",
            SkillTrust::Local,
            [("shell".to_string(), no_patterns())].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        // Local without patterns = unrestricted
        assert!(!enforcer.has_enforcement());
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "anything")]))
                .is_ok()
        );
    }

    #[test]
    fn test_local_with_patterns_enforced() {
        let skill = make_skill(
            "careful-tool",
            SkillTrust::Local,
            [("shell".to_string(), shell_patterns(&["cargo *"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        assert!(enforcer.has_enforcement());
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo build")]))
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "npm install")]))
                .is_err()
        );
    }

    // -----------------------------------------------------------------------
    // Mixed trust scenarios
    // -----------------------------------------------------------------------

    #[test]
    fn test_mixed_local_unrestricted_overrides() {
        // A Local skill without patterns makes the tool unrestricted,
        // even if a Verified skill has patterns.
        let verified = make_skill(
            "linter",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["vale *"]))].into(),
        );
        let local = make_skill(
            "my-helper",
            SkillTrust::Local,
            [("shell".to_string(), no_patterns())].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[verified, local]);

        // Local unrestricted overrides verified's constraints
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "anything")]))
                .is_ok()
        );
    }

    #[test]
    fn test_community_plus_verified_enforcement() {
        // Community patterns ignored, only verified patterns enforced
        let community = make_skill(
            "sketchy",
            SkillTrust::Community,
            [("shell".to_string(), shell_patterns(&["rm *"]))].into(),
        );
        let verified = make_skill(
            "builder",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["cargo *"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[community, verified]);

        // Only cargo allowed (from verified), rm not allowed (community ignored)
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo build")]))
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "rm -rf /")]))
                .is_err()
        );
    }

    // -----------------------------------------------------------------------
    // Error message quality
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_message_contains_details() {
        let skill = make_skill(
            "builder",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["cargo *"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        let err = enforcer
            .validate_tool_call("shell", &params(&[("command", "npm install evil-package")]))
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("shell"), "Error should mention tool name");
        assert!(msg.contains("command"), "Error should mention parameter");
        assert!(
            msg.contains("npm install evil-package"),
            "Error should include the value"
        );
        assert!(
            msg.contains("does not match"),
            "Error should explain the denial"
        );
    }

    #[test]
    fn test_long_value_truncated_in_error() {
        let skill = make_skill(
            "builder",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["cargo *"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        let long_cmd = "x".repeat(200);
        let err = enforcer
            .validate_tool_call("shell", &params(&[("command", &long_cmd)]))
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("..."), "Long values should be truncated");
    }

    #[test]
    fn test_multibyte_utf8_truncation_no_panic() {
        let skill = make_skill(
            "builder",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["cargo *"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        // CJK characters are 3 bytes each -- ensure truncation doesn't panic
        let multibyte_cmd = "\u{4e16}\u{754c}".repeat(50); // 100 CJK chars = 300 bytes
        let err = enforcer
            .validate_tool_call("shell", &params(&[("command", &multibyte_cmd)]))
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("..."),
            "Long multibyte values should be truncated"
        );
    }

    // -----------------------------------------------------------------------
    // HTTP tool skipped (handled by http_scoping)
    // -----------------------------------------------------------------------

    #[test]
    fn test_http_tool_skipped() {
        let skill = make_skill(
            "api-caller",
            SkillTrust::Verified,
            [("http".to_string(), shell_patterns(&["anything"]))].into(),
        );
        let enforcer = SkillPermissionEnforcer::from_active_skills(&[skill]);

        // HTTP tool should not be enforced by this module
        assert!(!enforcer.has_enforcement());
    }

    // -----------------------------------------------------------------------
    // Phase 3: Serialization / from_serialized tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let perm = SerializedToolPermission {
            tool_name: "shell".to_string(),
            trust: SkillTrust::Verified,
            patterns: vec![
                SerializedPattern::Shell {
                    command: "cargo *".to_string(),
                },
                SerializedPattern::FilePath {
                    path: "src/**/*.rs".to_string(),
                },
                SerializedPattern::MemoryTarget {
                    target: "daily/*".to_string(),
                },
            ],
        };

        let json = serde_json::to_string(&perm).unwrap();
        let roundtripped: SerializedToolPermission = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtripped.tool_name, "shell");
        assert_eq!(roundtripped.patterns.len(), 3);
        assert!(matches!(
            &roundtripped.patterns[0],
            SerializedPattern::Shell { command } if command == "cargo *"
        ));
        assert!(matches!(
            &roundtripped.patterns[1],
            SerializedPattern::FilePath { path } if path == "src/**/*.rs"
        ));
        assert!(matches!(
            &roundtripped.patterns[2],
            SerializedPattern::MemoryTarget { target } if target == "daily/*"
        ));
    }

    #[test]
    fn test_from_serialized_shell_enforcement() {
        let perms = vec![SerializedToolPermission {
            tool_name: "shell".to_string(),
            trust: SkillTrust::Verified,
            patterns: vec![SerializedPattern::Shell {
                command: "cargo *".to_string(),
            }],
        }];
        let enforcer = SkillPermissionEnforcer::from_serialized(&perms);
        assert!(enforcer.has_enforcement());

        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo build")]))
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "rm -rf /")]))
                .is_err()
        );
    }

    #[test]
    fn test_from_serialized_file_path_enforcement() {
        let perms = vec![SerializedToolPermission {
            tool_name: "write_file".to_string(),
            trust: SkillTrust::Verified,
            patterns: vec![SerializedPattern::FilePath {
                path: "src/**/*.rs".to_string(),
            }],
        }];
        let enforcer = SkillPermissionEnforcer::from_serialized(&perms);
        assert!(enforcer.has_enforcement());

        assert!(
            enforcer
                .validate_tool_call("write_file", &params(&[("path", "src/main.rs")]))
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("write_file", &params(&[("path", "/etc/passwd")]))
                .is_err()
        );
    }

    #[test]
    fn test_from_serialized_memory_target_enforcement() {
        let perms = vec![SerializedToolPermission {
            tool_name: "memory_write".to_string(),
            trust: SkillTrust::Verified,
            patterns: vec![SerializedPattern::MemoryTarget {
                target: "daily/*".to_string(),
            }],
        }];
        let enforcer = SkillPermissionEnforcer::from_serialized(&perms);
        assert!(enforcer.has_enforcement());

        assert!(
            enforcer
                .validate_tool_call(
                    "memory_write",
                    &params(&[("target", "daily/2024-01-15.md")])
                )
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("memory_write", &params(&[("target", "SOUL.md")]))
                .is_err()
        );
    }

    #[test]
    fn test_from_serialized_community_ignored() {
        let perms = vec![SerializedToolPermission {
            tool_name: "shell".to_string(),
            trust: SkillTrust::Community,
            patterns: vec![SerializedPattern::Shell {
                command: "rm *".to_string(),
            }],
        }];
        let enforcer = SkillPermissionEnforcer::from_serialized(&perms);
        // Community patterns ignored -- no enforcement active
        assert!(!enforcer.has_enforcement());
    }

    #[test]
    fn test_from_serialized_local_no_patterns_unrestricted() {
        let perms = vec![SerializedToolPermission {
            tool_name: "shell".to_string(),
            trust: SkillTrust::Local,
            patterns: vec![],
        }];
        let enforcer = SkillPermissionEnforcer::from_serialized(&perms);
        assert!(!enforcer.has_enforcement());
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "anything")]))
                .is_ok()
        );
    }

    #[test]
    fn test_from_serialized_empty_no_enforcement() {
        let enforcer = SkillPermissionEnforcer::from_serialized(&[]);
        assert!(!enforcer.has_enforcement());
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "anything")]))
                .is_ok()
        );
    }

    #[test]
    fn test_from_serialized_union_semantics() {
        let perms = vec![
            SerializedToolPermission {
                tool_name: "shell".to_string(),
                trust: SkillTrust::Verified,
                patterns: vec![SerializedPattern::Shell {
                    command: "cargo *".to_string(),
                }],
            },
            SerializedToolPermission {
                tool_name: "shell".to_string(),
                trust: SkillTrust::Verified,
                patterns: vec![SerializedPattern::Shell {
                    command: "vale *".to_string(),
                }],
            },
        ];
        let enforcer = SkillPermissionEnforcer::from_serialized(&perms);
        assert!(enforcer.has_enforcement());

        // Both patterns available via union
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo build")]))
                .is_ok()
        );
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "vale readme.md")]))
                .is_ok()
        );
        // Neither matches
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "rm -rf /")]))
                .is_err()
        );
    }

    #[test]
    fn test_from_serialized_bad_pattern_skipped() {
        // Pattern with invalid chars should be skipped, not fatal
        let perms = vec![SerializedToolPermission {
            tool_name: "shell".to_string(),
            trust: SkillTrust::Verified,
            patterns: vec![
                SerializedPattern::Shell {
                    command: "cargo; rm -rf /".to_string(), // invalid: semicolon
                },
                SerializedPattern::Shell {
                    command: "cargo *".to_string(), // valid
                },
            ],
        }];
        let enforcer = SkillPermissionEnforcer::from_serialized(&perms);
        assert!(enforcer.has_enforcement());

        // Valid pattern still works
        assert!(
            enforcer
                .validate_tool_call("shell", &params(&[("command", "cargo build")]))
                .is_ok()
        );
    }

    #[test]
    fn test_from_serialized_matches_from_active_skills() {
        // Build the same enforcer both ways and verify equivalent behavior
        let skill = make_skill(
            "builder",
            SkillTrust::Verified,
            [("shell".to_string(), shell_patterns(&["cargo *"]))].into(),
        );

        let from_skills = SkillPermissionEnforcer::from_active_skills(std::slice::from_ref(&skill));
        let serialized = SerializedToolPermission::from_active_skills(std::slice::from_ref(&skill));
        let from_serial = SkillPermissionEnforcer::from_serialized(&serialized);

        // Both should have enforcement
        assert_eq!(from_skills.has_enforcement(), from_serial.has_enforcement());

        // Both should produce same results
        let test_cases = [
            ("shell", "command", "cargo build", true),
            ("shell", "command", "rm -rf /", false),
            ("shell", "command", "cargo test", true),
        ];

        for (tool, param, value, expected_ok) in &test_cases {
            let params_val = params(&[(param, value)]);
            let r1 = from_skills.validate_tool_call(tool, &params_val).is_ok();
            let r2 = from_serial.validate_tool_call(tool, &params_val).is_ok();
            assert_eq!(
                r1, *expected_ok,
                "from_skills mismatch for {} {}={}",
                tool, param, value
            );
            assert_eq!(
                r2, *expected_ok,
                "from_serialized mismatch for {} {}={}",
                tool, param, value
            );
        }
    }
}
