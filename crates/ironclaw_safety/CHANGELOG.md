# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.3](https://github.com/nearai/ironclaw/compare/ironclaw_safety-v0.2.2...ironclaw_safety-v0.2.3) - 2026-06-15

### Fixed

- *(reborn)* repair oversized provider tool arguments ([#4805](https://github.com/nearai/ironclaw/pull/4805))
- *(reborn)* strengthen host runtime publication gates ([#3444](https://github.com/nearai/ironclaw/pull/3444))

### Other

- PR 18.5a: type-seal trusted trigger ingress ([#4406](https://github.com/nearai/ironclaw/pull/4406))
- [codex] Preserve provider reasoning summaries ([#4230](https://github.com/nearai/ironclaw/pull/4230))
- Refactor host runtime HTTP egress pipeline ([#4214](https://github.com/nearai/ironclaw/pull/4214))
- Add Reborn context compaction phase one ([#4110](https://github.com/nearai/ironclaw/pull/4110))
- Allow multiline provider tool arguments ([#3999](https://github.com/nearai/ironclaw/pull/3999))
- *(safety)* share pure sensitive path checks
- *(reborn)* add crate agent maps ([#3308](https://github.com/nearai/ironclaw/pull/3308))

## [0.2.2](https://github.com/nearai/ironclaw/compare/ironclaw_safety-v0.2.1...ironclaw_safety-v0.2.2) - 2026-04-29

### Added

- *(debug-panel)* expand Activity tab coverage with CodeAct + warnings ([#2850](https://github.com/nearai/ironclaw/pull/2850))

## [0.2.1](https://github.com/nearai/ironclaw/compare/ironclaw_safety-v0.2.0...ironclaw_safety-v0.2.1) - 2026-04-11

### Added

- *(engine)* Unified Thread-Capability-CodeAct execution engine (v2 architecture) ([#1557](https://github.com/nearai/ironclaw/pull/1557))

### Fixed

- *(safety)* add credential patterns and sensitive path blocklist ([#1675](https://github.com/nearai/ironclaw/pull/1675))
- *(security)* safety layer bypass via output truncation [HIGH] ([#1851](https://github.com/nearai/ironclaw/pull/1851))

### Other

- *(e2e)* expand SSE resilience coverage ([#1897](https://github.com/nearai/ironclaw/pull/1897))
- [codex] Move safety benches into ironclaw_safety crate ([#1954](https://github.com/nearai/ironclaw/pull/1954))
