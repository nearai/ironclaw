# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/nearai/ironclaw/compare/ironclaw_skills-v0.3.0...ironclaw_skills-v0.4.0) - 2026-07-24

### Added

- *(filesystem)* CAS-guarded delete_if_version on RootFilesystem ([#5749](https://github.com/nearai/ironclaw/pull/5749))
- *(reborn)* skill extraction & self-evolution with activation controls ([#5061](https://github.com/nearai/ironclaw/pull/5061))

### Fixed

- *(deps)* replace unmaintained serde_yml with serde_norway ([#5475](https://github.com/nearai/ironclaw/pull/5475))
- *(ci)* gate skills io/Read import to unix (fixes Clippy Windows ripple from #5325) ([#5351](https://github.com/nearai/ironclaw/pull/5351))
- *(ci)* green up main + cargo/non-cargo network resilience ([#5325](https://github.com/nearai/ironclaw/pull/5325))
- *(turns)* exempt certified skill content from prompt content denylist ([#5169](https://github.com/nearai/ironclaw/pull/5169)) ([#5258](https://github.com/nearai/ironclaw/pull/5258))
- *(skills,host_runtime,gsuite)* close reborn-closure tail failures ([#5108](https://github.com/nearai/ironclaw/pull/5108))

### Other

- *(skills)* establish routing slice 0 baseline ([#6595](https://github.com/nearai/ironclaw/pull/6595))
- *(stores)* FaultInjecting backend decorator; drive store fault tests through the real store ([#6400](https://github.com/nearai/ironclaw/pull/6400))
- W2 endorsed Reborn crate folds ([#5874](https://github.com/nearai/ironclaw/pull/5874))
- Add Reborn crate layer allowlist gate ([#5852](https://github.com/nearai/ironclaw/pull/5852))
- upgrade Rust version to 1.96 ([#5405](https://github.com/nearai/ironclaw/pull/5405))
- *(deps)* bump the everything-else group across 1 directory with 47 updates ([#5271](https://github.com/nearai/ironclaw/pull/5271))
- [codex] Add user-scoped skills settings UI ([#4527](https://github.com/nearai/ironclaw/pull/4527))
- *(agents)* reconcile crate AGENTS.md maps with current Reborn code ([#4302](https://github.com/nearai/ironclaw/pull/4302))
- [codex] Normalize synthesized skill install names
- [codex] fix reborn skill install replay ([#4385](https://github.com/nearai/ironclaw/pull/4385))
- [codex] Add config for regex skill activation ([#4144](https://github.com/nearai/ironclaw/pull/4144))
- Accept named plain Markdown skill installs ([#4138](https://github.com/nearai/ironclaw/pull/4138))
- Wire Reborn CLI skills list ([#4095](https://github.com/nearai/ironclaw/pull/4095))
- Wire Reborn extension lifecycle registry ([#4066](https://github.com/nearai/ironclaw/pull/4066))
- [codex] Realign Reborn lifecycle UX contracts ([#4012](https://github.com/nearai/ironclaw/pull/4012))
- [codex] Add URL installs for Reborn skills ([#4062](https://github.com/nearai/ironclaw/pull/4062))
- Add debug tracing for Reborn capability dispatch ([#3986](https://github.com/nearai/ironclaw/pull/3986))
- [codex] Add Reborn skill management tools ([#3935](https://github.com/nearai/ironclaw/pull/3935))
- [codex] Add Reborn skill activation selector ([#3861](https://github.com/nearai/ironclaw/pull/3861))
- *(crates)* seal internal modules across service crates
- *(reborn)* add crate agent maps ([#3308](https://github.com/nearai/ironclaw/pull/3308))

## [0.3.0](https://github.com/nearai/ironclaw/compare/ironclaw_skills-v0.2.0...ironclaw_skills-v0.3.0) - 2026-04-29

### Added

- *(credentials)* path-based credential matching for per-endpoint auth ([#2168](https://github.com/nearai/ironclaw/pull/2168))

### Other

- Merge pull request #3002 from nearai/main

## [0.2.0](https://github.com/nearai/ironclaw/compare/ironclaw_skills-v0.1.0...ironclaw_skills-v0.2.0) - 2026-04-21

### Added

- *(gateway)* add attachment flows, v2 skill install coverage, and e2e stabilization ([#2385](https://github.com/nearai/ironclaw/pull/2385))
- *(skills)* activation feedback pipeline + install idempotence ([#2530](https://github.com/nearai/ironclaw/pull/2530))
- *(skills)* setup-marker lifecycle, chain-loading, and live GitHub workflow test ([#2268](https://github.com/nearai/ironclaw/pull/2268))
- discover tool source in working directory during install ([#2396](https://github.com/nearai/ironclaw/pull/2396))

### Other

- Fix Slack relay OAuth callback state lookup ([#2512](https://github.com/nearai/ironclaw/pull/2512))
