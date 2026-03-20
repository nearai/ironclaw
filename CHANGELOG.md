# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.21.0](https://github.com/nearai/ironclaw/compare/v0.20.0...v0.21.0) - 2026-03-20

### Added

- structured fallback deliverables for failed/stuck jobs ([#236](https://github.com/nearai/ironclaw/pull/236))
- LRU embedding cache for workspace search ([#1423](https://github.com/nearai/ironclaw/pull/1423))
- receive relay events via webhook callbacks ([#1254](https://github.com/nearai/ironclaw/pull/1254))

### Fixed

- bump Feishu channel version for promotion
- *(approval)* make "always" auto-approve work for credentialed HTTP requests ([#1257](https://github.com/nearai/ironclaw/pull/1257))
- skip NEAR AI session check when backend is not nearai ([#1413](https://github.com/nearai/ironclaw/pull/1413))

### Other

- Make hosted OAuth and MCP auth generic ([#1375](https://github.com/nearai/ironclaw/pull/1375))

## [0.20.0](https://github.com/nearai/ironclaw/compare/v0.19.0...v0.20.0) - 2026-03-19

### Added

- *(self-repair)* wire stuck_threshold, store, and builder ([#712](https://github.com/nearai/ironclaw/pull/712))
- *(testing)* add FaultInjector framework for StubLlm ([#1233](https://github.com/nearai/ironclaw/pull/1233))
- *(gateway)* unified settings page with subtabs ([#1191](https://github.com/nearai/ironclaw/pull/1191))
- upgrade MiniMax default model to M2.7 ([#1357](https://github.com/nearai/ironclaw/pull/1357))

### Fixed

- navigate telegram E2E tests to channels subtab ([#1408](https://github.com/nearai/ironclaw/pull/1408))
- add missing `builder` field and update E2E extensions tab navigation ([#1400](https://github.com/nearai/ironclaw/pull/1400))
- remove debug_assert guards that panic on valid error paths ([#1385](https://github.com/nearai/ironclaw/pull/1385))
- address valid review comments from PR #1359 ([#1380](https://github.com/nearai/ironclaw/pull/1380))
- full_job routine runs stay running until linked job completion ([#1374](https://github.com/nearai/ironclaw/pull/1374))
- full_job routine concurrency tracks linked job lifetime ([#1372](https://github.com/nearai/ironclaw/pull/1372))
- remove -x from coverage pytest to prevent suite-blocking failures ([#1360](https://github.com/nearai/ironclaw/pull/1360))
- add debug_assert invariant guards to critical code paths ([#1312](https://github.com/nearai/ironclaw/pull/1312))
- *(mcp)* retry after missing session id errors ([#1355](https://github.com/nearai/ironclaw/pull/1355))
- *(telegram)* preserve polling after secret-blocked updates ([#1353](https://github.com/nearai/ironclaw/pull/1353))
- *(llm)* cap retry-after delays ([#1351](https://github.com/nearai/ironclaw/pull/1351))
- *(setup)* remove nonexistent webhook secret command hint ([#1349](https://github.com/nearai/ironclaw/pull/1349))
- Rate limiter returns retry after None instead of a duration ([#1269](https://github.com/nearai/ironclaw/pull/1269))

### Other

- bump telegram channel version to 0.2.5 ([#1410](https://github.com/nearai/ironclaw/pull/1410))
- *(ci)* enforce test requirement for state machine and resilience changes ([#1230](https://github.com/nearai/ironclaw/pull/1230)) ([#1304](https://github.com/nearai/ironclaw/pull/1304))
- Fix duplicate LLM responses for matched event routines ([#1275](https://github.com/nearai/ironclaw/pull/1275))
- add Japanese README ([#1306](https://github.com/nearai/ironclaw/pull/1306))
- *(ci)* add coverage gates via codecov.yml ([#1228](https://github.com/nearai/ironclaw/pull/1228)) ([#1291](https://github.com/nearai/ironclaw/pull/1291))
- Redesign routine create requests for LLMs ([#1147](https://github.com/nearai/ironclaw/pull/1147))

## [0.19.0](https://github.com/nearai/ironclaw/compare/v0.18.0...v0.19.0) - 2026-03-17

### Added

- verify telegram owner during hot activation ([#1157](https://github.com/nearai/ironclaw/pull/1157))
- *(config)* unify config resolution with Settings fallback (Phase 2, #1119) ([#1203](https://github.com/nearai/ironclaw/pull/1203))
- *(sandbox)* add retry logic for transient container failures ([#1232](https://github.com/nearai/ironclaw/pull/1232))
- *(heartbeat)* fire_at time-of-day scheduling with IANA timezone ([#1029](https://github.com/nearai/ironclaw/pull/1029))
- Reuse Codex CLI OAuth tokens for ChatGPT backend LLM calls ([#693](https://github.com/nearai/ironclaw/pull/693))
- add pre-push git hook with delta lint mode ([#833](https://github.com/nearai/ironclaw/pull/833))
- *(cli)* add `logs` command for gateway log access ([#1105](https://github.com/nearai/ironclaw/pull/1105))
- add Feishu/Lark WASM channel plugin ([#1110](https://github.com/nearai/ironclaw/pull/1110))
- add Criterion benchmarks for safety layer hot paths ([#836](https://github.com/nearai/ironclaw/pull/836))
- *(routines)* human-readable cron schedule summaries in web UI ([#1154](https://github.com/nearai/ironclaw/pull/1154))
- *(web)* add follow-up suggestion chips and ghost text ([#1156](https://github.com/nearai/ironclaw/pull/1156))
- *(ci)* include commit history in staging promotion PRs ([#952](https://github.com/nearai/ironclaw/pull/952))
- *(tools)* add reusable sensitive JSON redaction helper ([#457](https://github.com/nearai/ironclaw/pull/457))
- configurable hybrid search fusion strategy ([#234](https://github.com/nearai/ironclaw/pull/234))
- *(cli)* add cron subcommand for managing scheduled routines ([#1017](https://github.com/nearai/ironclaw/pull/1017))
- adds context-llm tool support ([#616](https://github.com/nearai/ironclaw/pull/616))
- *(web-chat)* add hover copy button for user/assistant messages ([#948](https://github.com/nearai/ironclaw/pull/948))
- add Slack approval buttons for tool execution in DMs ([#796](https://github.com/nearai/ironclaw/pull/796))
- enhance HTTP tool parameter parsing ([#911](https://github.com/nearai/ironclaw/pull/911))
- *(routines)* enable tool access in lightweight routine execution ([#257](https://github.com/nearai/ironclaw/pull/257)) ([#730](https://github.com/nearai/ironclaw/pull/730))
- add MiniMax as a built-in LLM provider ([#940](https://github.com/nearai/ironclaw/pull/940))
- *(cli)* add `ironclaw channels list` subcommand ([#933](https://github.com/nearai/ironclaw/pull/933))
- *(cli)* add `ironclaw skills list/search/info` subcommands ([#918](https://github.com/nearai/ironclaw/pull/918))
- add cargo-deny for supply chain safety ([#834](https://github.com/nearai/ironclaw/pull/834))
- *(setup)* display ASCII art banner during onboarding ([#851](https://github.com/nearai/ironclaw/pull/851))
- *(extensions)* unify auth and configure into single entrypoint ([#677](https://github.com/nearai/ironclaw/pull/677))
- *(i18n)* Add internationalization support with Chinese and English translations ([#929](https://github.com/nearai/ironclaw/pull/929))
- Import OpenClaw memory, history and settings ([#903](https://github.com/nearai/ironclaw/pull/903))

### Fixed

- jobs limit ([#1274](https://github.com/nearai/ironclaw/pull/1274))
- misleading UI message ([#1265](https://github.com/nearai/ironclaw/pull/1265))
- bump channel registry versions for promotion ([#1264](https://github.com/nearai/ironclaw/pull/1264))
- cover staging CI all-features and routine batch regressions ([#1256](https://github.com/nearai/ironclaw/pull/1256))
- resolve merge conflict fallout and missing config fields
- web/CLI routine mutations do not refresh live event trigger cache ([#1255](https://github.com/nearai/ironclaw/pull/1255))
- *(jobs)* make completed->completed transition idempotent to prevent race errors ([#1068](https://github.com/nearai/ironclaw/pull/1068))
- *(llm)* persist refreshed Anthropic OAuth token after Keychain re-read ([#1213](https://github.com/nearai/ironclaw/pull/1213))
- *(worker)* prevent orphaned tool_results and fix parallel merging ([#1069](https://github.com/nearai/ironclaw/pull/1069))
- Telegram bot token validation fails intermittently (HTTP 404) ([#1166](https://github.com/nearai/ironclaw/pull/1166))
- *(security)* prevent metadata spoofing of internal job monitor flag ([#1195](https://github.com/nearai/ironclaw/pull/1195))
- *(security)* default webhook server to loopback when tunnel is configured ([#1194](https://github.com/nearai/ironclaw/pull/1194))
- *(auth)* avoid false success and block chat during pending auth ([#1111](https://github.com/nearai/ironclaw/pull/1111))
- *(config)* unify ChannelsConfig resolution to env > settings > default ([#1124](https://github.com/nearai/ironclaw/pull/1124))
- *(web-chat)* normalize chat copy to plain text ([#1114](https://github.com/nearai/ironclaw/pull/1114))
- *(skill)* treat empty url param as absent when installing skills ([#1128](https://github.com/nearai/ironclaw/pull/1128))
- preserve AuthError type in oauth_http_client cache ([#1152](https://github.com/nearai/ironclaw/pull/1152))
- *(web)* prevent Safari IME composition Enter from sending message ([#1140](https://github.com/nearai/ironclaw/pull/1140))
- *(mcp)* handle 400 auth errors, clear auth mode after OAuth, trim tokens ([#1158](https://github.com/nearai/ironclaw/pull/1158))
- eliminate panic paths in production code ([#1184](https://github.com/nearai/ironclaw/pull/1184))
- N+1 query pattern in event trigger loop (routine_engine) ([#1163](https://github.com/nearai/ironclaw/pull/1163))
- *(llm)* add stop_sequences parity for tool completions ([#1170](https://github.com/nearai/ironclaw/pull/1170))
- *(channels)* use live owner binding during wasm hot activation ([#1171](https://github.com/nearai/ironclaw/pull/1171))
- Non-transactional multi-step context updates between metadata/to… ([#1161](https://github.com/nearai/ironclaw/pull/1161))
- *(webhook)* avoid lock-held awaits in server lifecycle paths ([#1168](https://github.com/nearai/ironclaw/pull/1168))
- Google Sheets returns 403 PERMISSION_DENIED after completing OAuth ([#1164](https://github.com/nearai/ironclaw/pull/1164))
- HTTP webhook secret transmitted in request body rather than via header, docs inconsistency and security concern ([#1162](https://github.com/nearai/ironclaw/pull/1162))
- *(ci)* exclude ironclaw_safety from release automation ([#1146](https://github.com/nearai/ironclaw/pull/1146))
- *(registry)* bump versions for github, web-search, and discord extensions ([#1106](https://github.com/nearai/ironclaw/pull/1106))
- *(mcp)* address 14 audit findings across MCP module ([#1094](https://github.com/nearai/ironclaw/pull/1094))
- *(http)* replace .expect() with match in webhook handler ([#1133](https://github.com/nearai/ironclaw/pull/1133))
- *(time)* treat empty timezone string as absent ([#1127](https://github.com/nearai/ironclaw/pull/1127))
- 5 critical/high-priority bugs (auth bypass, relay failures, unbounded recursion, context growth) ([#1083](https://github.com/nearai/ironclaw/pull/1083))
- *(ci)* checkout promotion PR head for metadata refresh ([#1097](https://github.com/nearai/ironclaw/pull/1097))
- *(ci)* add missing attachments field and crates/ dir to Dockerfiles ([#1100](https://github.com/nearai/ironclaw/pull/1100))
- *(registry)* bump telegram channel version for capabilities change ([#1064](https://github.com/nearai/ironclaw/pull/1064))
- *(ci)* repair staging promotion workflow behavior ([#1091](https://github.com/nearai/ironclaw/pull/1091))
- *(wasm)* address #1086 review followups -- description hint and coercion safety ([#1092](https://github.com/nearai/ironclaw/pull/1092))
- *(ci)* repair staging-ci workflow parsing ([#1090](https://github.com/nearai/ironclaw/pull/1090))
- *(extensions)* fix lifecycle bugs + comprehensive E2E tests ([#1070](https://github.com/nearai/ironclaw/pull/1070))
- add tool_info schema discovery for WASM tools ([#1086](https://github.com/nearai/ironclaw/pull/1086))
- resolve bug_bash UX/logging issues (#1054 #1055 #1058) ([#1072](https://github.com/nearai/ironclaw/pull/1072))
- *(http)* fail closed when webhook secret is missing at runtime ([#1075](https://github.com/nearai/ironclaw/pull/1075))
- *(service)* set CLI_ENABLED=false in macOS launchd plist ([#1079](https://github.com/nearai/ironclaw/pull/1079))
- relax approval requirements for low-risk tools ([#922](https://github.com/nearai/ironclaw/pull/922))
- *(web)* make approval requests appear without page reload ([#996](https://github.com/nearai/ironclaw/pull/996)) ([#1073](https://github.com/nearai/ironclaw/pull/1073))
- *(routines)* run cron checks immediately on ticker startup ([#1066](https://github.com/nearai/ironclaw/pull/1066))
- *(web)* recompute cron next_fire_at when re-enabling routines ([#1080](https://github.com/nearai/ironclaw/pull/1080))
- *(memory)* reject absolute filesystem paths with corrective routing ([#934](https://github.com/nearai/ironclaw/pull/934))
- remove all inline event handlers for CSP script-src compliance ([#1063](https://github.com/nearai/ironclaw/pull/1063))
- *(mcp)* include OAuth state parameter in authorization URLs ([#1049](https://github.com/nearai/ironclaw/pull/1049))
- *(mcp)* open MCP OAuth in same browser as gateway ([#951](https://github.com/nearai/ironclaw/pull/951))
- *(deploy)* harden production container and bootstrap security ([#1014](https://github.com/nearai/ironclaw/pull/1014))
- release lock guards before awaiting channel send ([#869](https://github.com/nearai/ironclaw/pull/869)) ([#1003](https://github.com/nearai/ironclaw/pull/1003))
- *(registry)* use versioned artifact URLs and checksums for all WASM manifests ([#1007](https://github.com/nearai/ironclaw/pull/1007))
- *(setup)* preserve model selection on provider re-run ([#679](https://github.com/nearai/ironclaw/pull/679)) ([#987](https://github.com/nearai/ironclaw/pull/987))
- *(mcp)* attach session manager for non-OAuth HTTP clients ([#793](https://github.com/nearai/ironclaw/pull/793)) ([#986](https://github.com/nearai/ironclaw/pull/986))
- *(security)* migrate webhook auth to HMAC-SHA256 signature header ([#970](https://github.com/nearai/ironclaw/pull/970))
- *(security)* make unsafe env::set_var calls safe with explicit invariants ([#968](https://github.com/nearai/ironclaw/pull/968))
- *(security)* require explicit SANDBOX_ALLOW_FULL_ACCESS to enable FullAccess policy ([#967](https://github.com/nearai/ironclaw/pull/967))
- *(security)* add Content-Security-Policy header to web gateway ([#966](https://github.com/nearai/ironclaw/pull/966))
- *(test)* stabilize openai compat oversized-body regression ([#839](https://github.com/nearai/ironclaw/pull/839))
- *(ci)* disambiguate WASM bundle filenames to prevent tool/channel collision ([#964](https://github.com/nearai/ironclaw/pull/964))
- *(setup)* validate channel credentials during setup ([#684](https://github.com/nearai/ironclaw/pull/684))
- drain tunnel pipes to prevent zombie process ([#735](https://github.com/nearai/ironclaw/pull/735))
- *(mcp)* header safety validation and Authorization conflict bug from #704 ([#752](https://github.com/nearai/ironclaw/pull/752))
- *(agent)* block thread_id-based context pollution across users ([#760](https://github.com/nearai/ironclaw/pull/760))
- *(mcp)* stdio/unix transports skip initialize handshake ([#890](https://github.com/nearai/ironclaw/pull/890)) ([#935](https://github.com/nearai/ironclaw/pull/935))
- *(setup)* drain residual events and filter key kind in onboard prompts ([#937](https://github.com/nearai/ironclaw/pull/937)) ([#949](https://github.com/nearai/ironclaw/pull/949))
- *(security)* load WASM tool description and schema from capabilities.json ([#520](https://github.com/nearai/ironclaw/pull/520))
- *(security)* resolve DNS once and reuse for SSRF validation to prevent rebinding ([#518](https://github.com/nearai/ironclaw/pull/518))
- *(security)* replace regex HTML sanitizer with DOMPurify to prevent XSS ([#510](https://github.com/nearai/ironclaw/pull/510))
- *(ci)* improve Claude Code review reliability ([#955](https://github.com/nearai/ironclaw/pull/955))
- *(ci)* run gated test jobs during staging CI ([#956](https://github.com/nearai/ironclaw/pull/956))
- *(ci)* prevent staging-ci tag failure and chained PR auto-close ([#900](https://github.com/nearai/ironclaw/pull/900))
- *(ci)* WASM WIT compat sqlite3 duplicate symbol conflict ([#953](https://github.com/nearai/ironclaw/pull/953))
- resolve deferred review items from PRs #883, #848, #788 ([#915](https://github.com/nearai/ironclaw/pull/915))
- *(web)* improve UX readability and accessibility in chat UI ([#910](https://github.com/nearai/ironclaw/pull/910))

### Other

- Fix Telegram auto-verify flow and routing ([#1273](https://github.com/nearai/ironclaw/pull/1273))
- *(e2e)* fix approval waiting regression coverage ([#1270](https://github.com/nearai/ironclaw/pull/1270))
- isolate heavy integration tests ([#1266](https://github.com/nearai/ironclaw/pull/1266))
- Merge branch 'main' into fix/resolve-conflicts
- Refactor owner scope across channels and fix default routing fallback ([#1151](https://github.com/nearai/ironclaw/pull/1151))
- *(extensions)* document relay manager init order ([#928](https://github.com/nearai/ironclaw/pull/928))
- *(setup)* extract init logic from wizard into owning modules ([#1210](https://github.com/nearai/ironclaw/pull/1210))
- mention MiniMax as built-in provider in all READMEs ([#1209](https://github.com/nearai/ironclaw/pull/1209))
- Fix schema-guided tool parameter coercion ([#1143](https://github.com/nearai/ironclaw/pull/1143))
- Make no-panics CI check test-aware ([#1160](https://github.com/nearai/ironclaw/pull/1160))
- *(mcp)* avoid reallocating SSE buffer on each chunk ([#1153](https://github.com/nearai/ironclaw/pull/1153))
- *(routines)* avoid full message history clone each tool iteration ([#1172](https://github.com/nearai/ironclaw/pull/1172))
- *(registry)* align manifest versions with published artifacts ([#1169](https://github.com/nearai/ironclaw/pull/1169))
- remove __pycache__ from repo and add to .gitignore ([#1177](https://github.com/nearai/ironclaw/pull/1177))
- *(registry)* move MCP servers from code to JSON manifests ([#1144](https://github.com/nearai/ironclaw/pull/1144))
- improve routine schema guidance ([#1089](https://github.com/nearai/ironclaw/pull/1089))
- add event-trigger routine e2e coverage ([#1088](https://github.com/nearai/ironclaw/pull/1088))
- enforce no .unwrap(), .expect(), or assert!() in production code ([#1087](https://github.com/nearai/ironclaw/pull/1087))
- periodic sync main into staging (resolved conflicts) ([#1098](https://github.com/nearai/ironclaw/pull/1098))
- fix formatting in cli/mod.rs and mcp/auth.rs ([#1071](https://github.com/nearai/ironclaw/pull/1071))
- Expose the shared agent session manager via AppComponents ([#532](https://github.com/nearai/ironclaw/pull/532))
- *(agent)* remove unnecessary Worker re-export ([#923](https://github.com/nearai/ironclaw/pull/923))
- Fix UTF-8 unsafe truncation in WASM emit_message ([#1015](https://github.com/nearai/ironclaw/pull/1015))
- extract safety module into ironclaw_safety crate ([#1024](https://github.com/nearai/ironclaw/pull/1024))
- Add Z.AI provider support for GLM-5 ([#938](https://github.com/nearai/ironclaw/pull/938))
- *(html_to_markdown)* refresh golden files after renderer bump ([#1016](https://github.com/nearai/ironclaw/pull/1016))
- Migrate GitHub webhook normalization into github tool ([#758](https://github.com/nearai/ironclaw/pull/758))
- Fix systemctl unit ([#472](https://github.com/nearai/ironclaw/pull/472))
- add Russian localization (README.ru.md) ([#850](https://github.com/nearai/ironclaw/pull/850))
- Add generic host-verified /webhook/tools/{tool} ingress ([#757](https://github.com/nearai/ironclaw/pull/757))

## [0.18.0](https://github.com/nearai/ironclaw/compare/v0.17.0...v0.18.0) - 2026-03-11

### Other

- Merge pull request #907 from nearai/staging-promote/b0214fef-22930316561
- promote staging to main (2026-03-10 15:19 UTC) ([#865](https://github.com/nearai/ironclaw/pull/865))
- Merge pull request #830 from nearai/staging-promote/3a2989d0-22888378864
- update WASM artifact SHA256 checksums [skip ci] ([#876](https://github.com/nearai/ironclaw/pull/876))

## [0.17.0](https://github.com/nearai/ironclaw/compare/v0.16.1...v0.17.0) - 2026-03-10

### Added

- *(llm)* per-provider unsupported parameter filtering (#749, #728) ([#809](https://github.com/nearai/ironclaw/pull/809))
- persist user_id in save_job and expose job_id on routine runs ([#709](https://github.com/nearai/ironclaw/pull/709))
- *(ci)* chained promotion PRs with multi-agent Claude review ([#776](https://github.com/nearai/ironclaw/pull/776))
- add background sandbox reaper for orphaned Docker containers ([#634](https://github.com/nearai/ironclaw/pull/634))
- *(wasm)* lazy schema injection on WASM tool errors ([#638](https://github.com/nearai/ironclaw/pull/638))
- add AWS Bedrock LLM provider via native Converse API ([#713](https://github.com/nearai/ironclaw/pull/713))
- full image support across all channels ([#725](https://github.com/nearai/ironclaw/pull/725))
- *(skills)* exclude_keywords veto in skill activation scoring ([#688](https://github.com/nearai/ironclaw/pull/688))
- *(mcp)* transport abstraction, stdio/UDS transports, and OAuth fixes ([#721](https://github.com/nearai/ironclaw/pull/721))
- add PID-based gateway lock to prevent multiple instances ([#717](https://github.com/nearai/ironclaw/pull/717))
- configurable LLM request timeout via LLM_REQUEST_TIMEOUT_SECS ([#615](https://github.com/nearai/ironclaw/pull/615)) ([#630](https://github.com/nearai/ironclaw/pull/630))
- *(timezone)* add timezone-aware session context ([#671](https://github.com/nearai/ironclaw/pull/671))
- *(setup)* Anthropic OAuth onboarding with setup-token support ([#384](https://github.com/nearai/ironclaw/pull/384))
- *(llm)* add Google Gemini, AWS Bedrock, io.net, Mistral, Yandex, and Cloudflare WS AI providers ([#676](https://github.com/nearai/ironclaw/pull/676))
- unified thread model for web gateway ([#607](https://github.com/nearai/ironclaw/pull/607))
- WASM channel attachments with LLM pipeline integration ([#596](https://github.com/nearai/ironclaw/pull/596))
- enable Anthropic prompt caching via automatic cache_control injection ([#660](https://github.com/nearai/ironclaw/pull/660))
- *(routines)* approval context for autonomous job execution ([#577](https://github.com/nearai/ironclaw/pull/577))
- *(llm)* declarative provider registry ([#618](https://github.com/nearai/ironclaw/pull/618))
- *(gateway)* show IronClaw version in status popover [skip-regression-check] ([#636](https://github.com/nearai/ironclaw/pull/636))
- Wire memory hygiene retention policy into heartbeat loop ([#629](https://github.com/nearai/ironclaw/pull/629))

### Fixed

- *(ci)* run fmt + clippy on staging PRs, skip Windows clippy [skip-regression-check] ([#802](https://github.com/nearai/ironclaw/pull/802))
- *(ci)* clean up staging pipeline — remove hacks, skip redundant checks [skip-regression-check] ([#794](https://github.com/nearai/ironclaw/pull/794))
- *(ci)* secrets can't be used in step if conditions [skip-regression-check] ([#787](https://github.com/nearai/ironclaw/pull/787))
- prevent irreversible context loss when compaction archive write fails ([#754](https://github.com/nearai/ironclaw/pull/754))
- button styles ([#637](https://github.com/nearai/ironclaw/pull/637))
- *(mcp)* JSON-RPC spec compliance — flexible id, correct notification format ([#685](https://github.com/nearai/ironclaw/pull/685))
- preserve tool-call history across thread hydration ([#568](https://github.com/nearai/ironclaw/pull/568)) ([#670](https://github.com/nearai/ironclaw/pull/670))
- CLI commands ignore runtime DATABASE_BACKEND when both features compiled ([#740](https://github.com/nearai/ironclaw/pull/740))
- *(web)* prevent fetch error when hostname is an IP address in TEE check ([#672](https://github.com/nearai/ironclaw/pull/672))
- add timezone conversion support to time tool ([#687](https://github.com/nearai/ironclaw/pull/687))
- standardize libSQL timestamps as RFC 3339 UTC ([#683](https://github.com/nearai/ironclaw/pull/683))
- *(docker)* bind postgres to localhost only ([#686](https://github.com/nearai/ironclaw/pull/686))
- *(repl)* skip /quit on EOF when stdin is not a TTY ([#724](https://github.com/nearai/ironclaw/pull/724))
- *(web)* prevent Enter key from sending message during IME composition ([#715](https://github.com/nearai/ironclaw/pull/715))
- *(config)* init_secrets no longer overwrites entire config ([#726](https://github.com/nearai/ironclaw/pull/726))
- *(cli)* status command ignores config.toml and settings.json ([#354](https://github.com/nearai/ironclaw/pull/354)) ([#734](https://github.com/nearai/ironclaw/pull/734))
- *(setup)* preserve model name when re-running onboarding with same provider ([#600](https://github.com/nearai/ironclaw/pull/600)) ([#694](https://github.com/nearai/ironclaw/pull/694))
- *(setup)* initialize secrets crypto for env-var security option ([#666](https://github.com/nearai/ironclaw/pull/666)) ([#706](https://github.com/nearai/ironclaw/pull/706))
- persist /model selection across restarts ([#707](https://github.com/nearai/ironclaw/pull/707))
- *(routines)* resolve message tool channel/target from per-job metadata ([#708](https://github.com/nearai/ironclaw/pull/708))
- sanitize HTML error bodies from MCP servers to prevent web UI white screen ([#263](https://github.com/nearai/ironclaw/pull/263)) ([#656](https://github.com/nearai/ironclaw/pull/656))
- prevent Instant duration overflow on Windows ([#657](https://github.com/nearai/ironclaw/pull/657)) ([#664](https://github.com/nearai/ironclaw/pull/664))
- enable libsql remote + tls features for Turso cloud sync ([#587](https://github.com/nearai/ironclaw/pull/587))
- *(tests)* replace hardcoded /tmp paths with tempdir + add 300 unit tests ([#659](https://github.com/nearai/ironclaw/pull/659))
- *(llm)* nudge LLM when it expresses tool intent without calling tools ([#653](https://github.com/nearai/ironclaw/pull/653))
- *(llm)* report zero cost for OpenRouter free-tier models ([#463](https://github.com/nearai/ironclaw/pull/463)) ([#613](https://github.com/nearai/ironclaw/pull/613))
- reliable network tests and improved tool error messages ([#626](https://github.com/nearai/ironclaw/pull/626))
- *(wasm)* use per-engine cache dirs on Windows to avoid file lock error ([#624](https://github.com/nearai/ironclaw/pull/624))
- *(libsql)* support flexible embedding dimensions ([#534](https://github.com/nearai/ironclaw/pull/534))

### Other

- Restructure CLAUDE.md into modular rules + add pr-shepherd command ([#750](https://github.com/nearai/ironclaw/pull/750))
- make src/llm/ self-contained for crate extraction ([#767](https://github.com/nearai/ironclaw/pull/767))
- add simplified Chinese (zh-CN) README translation ([#488](https://github.com/nearai/ironclaw/pull/488))
- *(job)* cover job tool validation and state transitions ([#681](https://github.com/nearai/ironclaw/pull/681))
- *(agent)* wire TestRig job tools through the scheduler ([#716](https://github.com/nearai/ironclaw/pull/716))
- Fix single-message mode to exit after one turn when background channels are enabled ([#719](https://github.com/nearai/ironclaw/pull/719))
- remove dead code ([#648](https://github.com/nearai/ironclaw/pull/648)) ([#703](https://github.com/nearai/ironclaw/pull/703))
- add reviewer-feedback guardrails (CLAUDE.md, pre-commit hook, skill) ([#665](https://github.com/nearai/ironclaw/pull/665))
- update WASM artifact SHA256 checksums [skip ci] ([#631](https://github.com/nearai/ironclaw/pull/631))
- add explanatory comments to coverage workflow ([#610](https://github.com/nearai/ironclaw/pull/610))
- build system prompt once per turn, skip tools on force-text ([#583](https://github.com/nearai/ironclaw/pull/583))
- add comprehensive subdirectory CLAUDE.md files and update root ([#589](https://github.com/nearai/ironclaw/pull/589))
- Improve test infrastructure: StubChannel, gateway helpers, security tests, search edge cases ([#623](https://github.com/nearai/ironclaw/pull/623))
- *(workspace)* regression test for document_path in search results ([#509](https://github.com/nearai/ironclaw/pull/509))

### Added

- AWS Bedrock LLM provider via native Converse API with IAM and SSO auth support (feature-gated: `--features bedrock`)

## [0.16.1](https://github.com/nearai/ironclaw/compare/v0.16.0...v0.16.1) - 2026-03-06

### Fixed

- revert WASM artifact SHA256 checksums to null ([#627](https://github.com/nearai/ironclaw/pull/627))

## [0.16.0](https://github.com/nearai/ironclaw/compare/v0.15.0...v0.16.0) - 2026-03-06

### Added

- *(e2e)* extensions tab tests, CI parallelization, and 3 production bug fixes ([#584](https://github.com/nearai/ironclaw/pull/584))
- WASM extension versioning with WIT compat checks ([#592](https://github.com/nearai/ironclaw/pull/592))
- Add HMAC-SHA256 webhook signature validation for Slack ([#588](https://github.com/nearai/ironclaw/pull/588))
- restart ([#531](https://github.com/nearai/ironclaw/pull/531))
- merge http/web_fetch tools, add tool output stash for large responses ([#578](https://github.com/nearai/ironclaw/pull/578))
- integrate 13-dimension complexity scorer into smart routing ([#529](https://github.com/nearai/ironclaw/pull/529))

### Fixed

- *(llm)* fix reasoning model response parsing bugs ([#564](https://github.com/nearai/ironclaw/pull/564)) ([#580](https://github.com/nearai/ironclaw/pull/580))
- *(ci)* fix three coverage workflow failures ([#597](https://github.com/nearai/ironclaw/pull/597))
- Telegram channel accepts group messages from all users if owner_… ([#590](https://github.com/nearai/ironclaw/pull/590))
- *(ci)* anchor coverage/ gitignore rule to repo root ([#591](https://github.com/nearai/ironclaw/pull/591))
- *(security)* use OsRng for all security-critical key and token generation ([#519](https://github.com/nearai/ironclaw/pull/519))
- prevent concurrent memory hygiene passes and Windows file lock errors ([#535](https://github.com/nearai/ironclaw/pull/535))
- sort tool_definitions() for deterministic LLM tool ordering ([#582](https://github.com/nearai/ironclaw/pull/582))
- *(ci)* persist all cargo-llvm-cov env vars for E2E coverage ([#559](https://github.com/nearai/ironclaw/pull/559))

### Other

- *(llm)* complete response cache — set_model invalidation, stats logging, sync mutex ([#290](https://github.com/nearai/ironclaw/pull/290))
- add 29 E2E trace tests for issues #571-575 ([#593](https://github.com/nearai/ironclaw/pull/593))
- add 26 tests for multi-thread safety, db CRUD, concurrency, errors ([#442](https://github.com/nearai/ironclaw/pull/442))
- update WASM artifact SHA256 checksums [skip ci] ([#560](https://github.com/nearai/ironclaw/pull/560))
- add WIT compatibility tests for WASM extensions ([#586](https://github.com/nearai/ironclaw/pull/586))
- Trajectory benchmarks and e2e trace test rig ([#553](https://github.com/nearai/ironclaw/pull/553))

## [0.15.0](https://github.com/nearai/ironclaw/compare/v0.14.0...v0.15.0) - 2026-03-04

### Added

- *(oauth)* route callbacks through web gateway for hosted instances ([#555](https://github.com/nearai/ironclaw/pull/555))
- *(web)* show error details for failed tool calls ([#490](https://github.com/nearai/ironclaw/pull/490))
- *(extensions)* improve auth UX and add load-time validation ([#536](https://github.com/nearai/ironclaw/pull/536))
- add local-test skill and Dockerfile.test for web gateway testing ([#524](https://github.com/nearai/ironclaw/pull/524))

### Fixed

- *(security)* restrict query-token auth to SSE endpoints only ([#528](https://github.com/nearai/ironclaw/pull/528))
- *(ci)* flush profraw coverage data in E2E teardown ([#550](https://github.com/nearai/ironclaw/pull/550))
- *(wasm)* coerce string parameters to schema-declared types ([#498](https://github.com/nearai/ironclaw/pull/498))
- *(agent)* strip leaked [Called tool ...] text from responses ([#497](https://github.com/nearai/ironclaw/pull/497))
- *(web)* reset job list UI on restart failure ([#499](https://github.com/nearai/ironclaw/pull/499))
- *(security)* replace .unwrap() panics in pairing store with proper error handling ([#515](https://github.com/nearai/ironclaw/pull/515))

### Other

- Fix UTF-8 unsafe truncation in sandbox log capture ([#359](https://github.com/nearai/ironclaw/pull/359))
- enhance coverage with feature matrix, postgres, and E2E ([#523](https://github.com/nearai/ironclaw/pull/523))

## [0.14.0](https://github.com/nearai/ironclaw/compare/v0.13.1...v0.14.0) - 2026-03-04

### Added

- remove the okta tool ([#506](https://github.com/nearai/ironclaw/pull/506))
- add OAuth support for WASM tools in web gateway ([#489](https://github.com/nearai/ironclaw/pull/489))
- *(web)* fix jobs UI parity for non-sandbox mode ([#491](https://github.com/nearai/ironclaw/pull/491))
- *(workspace)* add TOOLS.md, BOOTSTRAP.md, and disk-to-DB import ([#477](https://github.com/nearai/ironclaw/pull/477))

### Fixed

- *(web)* mobile browser bar obscures chat input ([#508](https://github.com/nearai/ironclaw/pull/508))
- *(web)* assign unique thread_id to manual routine triggers ([#500](https://github.com/nearai/ironclaw/pull/500))
- *(web)* refresh routine UI after Run Now trigger ([#501](https://github.com/nearai/ironclaw/pull/501))
- *(skills)* use slug for skill download URL from ClawHub ([#502](https://github.com/nearai/ironclaw/pull/502))
- *(workspace)* thread document path through search results ([#503](https://github.com/nearai/ironclaw/pull/503))
- *(workspace)* import custom templates before seeding defaults ([#505](https://github.com/nearai/ironclaw/pull/505))
- use std::sync::RwLock in MessageTool to avoid runtime panic ([#411](https://github.com/nearai/ironclaw/pull/411))
- wire secrets store into all WASM runtime activation paths ([#479](https://github.com/nearai/ironclaw/pull/479))

### Other

- enforce regression tests for fix commits ([#517](https://github.com/nearai/ironclaw/pull/517))
- add code coverage with cargo-llvm-cov and Codecov ([#511](https://github.com/nearai/ironclaw/pull/511))
- Remove restart infrastructure, generalize WASM channel setup ([#493](https://github.com/nearai/ironclaw/pull/493))

## [0.13.1](https://github.com/nearai/ironclaw/compare/v0.13.0...v0.13.1) - 2026-03-02

### Added

- add Brave Web Search WASM tool ([#474](https://github.com/nearai/ironclaw/pull/474))

### Fixed

- *(web)* auto-scroll and Enter key completion for slash command autocomplete ([#475](https://github.com/nearai/ironclaw/pull/475))
- correct download URLs for telegram-mtproto and slack-tool extensions ([#470](https://github.com/nearai/ironclaw/pull/470))

## [0.13.0](https://github.com/nearai/ironclaw/compare/v0.12.0...v0.13.0) - 2026-03-02

### Added

- *(cli)* add tool setup command + GitHub setup schema ([#438](https://github.com/nearai/ironclaw/pull/438))
- add web_fetch built-in tool ([#435](https://github.com/nearai/ironclaw/pull/435))
- *(web)* DB-backed Jobs tab + scheduler-dispatched local jobs ([#436](https://github.com/nearai/ironclaw/pull/436))
- *(extensions)* add OAuth setup UI for WASM tools + display name labels ([#437](https://github.com/nearai/ironclaw/pull/437))
- *(bootstrap)* auto-detect libsql when ironclaw.db exists ([#399](https://github.com/nearai/ironclaw/pull/399))
- *(web)* slash command autocomplete + /status /list + fix chat input locking ([#404](https://github.com/nearai/ironclaw/pull/404))
- *(routines)* deliver notifications to all installed channels ([#398](https://github.com/nearai/ironclaw/pull/398))
- *(web)* persist tool calls, restore approvals on thread switch, and UI fixes ([#382](https://github.com/nearai/ironclaw/pull/382))
- add IRONCLAW_BASE_DIR env var with LazyLock caching ([#397](https://github.com/nearai/ironclaw/pull/397))
- feat(signal) attachment upload  + message tool ([#375](https://github.com/nearai/ironclaw/pull/375))

### Fixed

- *(channels)* add host-based credential injection to WASM channel wrapper ([#421](https://github.com/nearai/ironclaw/pull/421))
- pre-validate Cloudflare tunnel token by spawning cloudflared ([#446](https://github.com/nearai/ironclaw/pull/446))
- batch of quick fixes (#417, #338, #330, #358, #419, #344) ([#428](https://github.com/nearai/ironclaw/pull/428))
- persist channel activation state across restarts ([#432](https://github.com/nearai/ironclaw/pull/432))
- init WASM runtime eagerly regardless of tools directory existence ([#401](https://github.com/nearai/ironclaw/pull/401))
- add TLS support for PostgreSQL connections ([#363](https://github.com/nearai/ironclaw/pull/363)) ([#427](https://github.com/nearai/ironclaw/pull/427))
- scan inbound messages for leaked secrets ([#433](https://github.com/nearai/ironclaw/pull/433))
- use tailscale funnel --bg for proper tunnel setup ([#430](https://github.com/nearai/ironclaw/pull/430))
- normalize secret names to lowercase for case-insensitive matching ([#413](https://github.com/nearai/ironclaw/pull/413)) ([#431](https://github.com/nearai/ironclaw/pull/431))
- persist model name to .env so dotted names survive restart ([#426](https://github.com/nearai/ironclaw/pull/426))
- *(setup)* check cloudflared binary and validate tunnel token ([#424](https://github.com/nearai/ironclaw/pull/424))
- *(setup)* validate PostgreSQL version and pgvector availability before migrations ([#423](https://github.com/nearai/ironclaw/pull/423))
- guard zsh compdef call to prevent error before compinit ([#422](https://github.com/nearai/ironclaw/pull/422))
- *(telegram)* remove restart button, validate token on setup ([#434](https://github.com/nearai/ironclaw/pull/434))
- web UI routines tab shows all routines regardless of creating channel ([#391](https://github.com/nearai/ironclaw/pull/391))
- Discord Ed25519 signature verification and capabilities header alias ([#148](https://github.com/nearai/ironclaw/pull/148)) ([#372](https://github.com/nearai/ironclaw/pull/372))
- prevent duplicate WASM channel activation on startup ([#390](https://github.com/nearai/ironclaw/pull/390))

### Other

- rename WasmBuildable::repo_url to source_dir ([#445](https://github.com/nearai/ironclaw/pull/445))
- Improve --help: add detailed about/examples/color, snapshot test (clo… ([#371](https://github.com/nearai/ironclaw/pull/371))
- Add automated QA: schema validator, CI matrix, Docker build, and P1 test coverage ([#353](https://github.com/nearai/ironclaw/pull/353))

## [0.12.0](https://github.com/nearai/ironclaw/compare/v0.11.1...v0.12.0) - 2026-02-26

### Added

- *(web)* improve WASM channel setup flow ([#380](https://github.com/nearai/ironclaw/pull/380))
- *(web)* inline tool activity cards with auto-collapsing ([#376](https://github.com/nearai/ironclaw/pull/376))
- *(web)* display logs newest-first in web gateway UI ([#369](https://github.com/nearai/ironclaw/pull/369))
- *(signal)* tool approval workflow and status updates ([#350](https://github.com/nearai/ironclaw/pull/350))
- add OpenRouter preset to setup wizard ([#270](https://github.com/nearai/ironclaw/pull/270))
- *(channels)* add native Signal channel via signal-cli HTTP daemon ([#271](https://github.com/nearai/ironclaw/pull/271))

### Fixed

- correct MCP registry URLs and remove non-existent Google endpoints ([#370](https://github.com/nearai/ironclaw/pull/370))
- resolve_thread adopts existing session threads by UUID ([#377](https://github.com/nearai/ironclaw/pull/377))
- resolve telegram/slack name collision between tool and channel registries ([#346](https://github.com/nearai/ironclaw/pull/346))
- make onboarding installs prefer release artifacts with source fallback ([#323](https://github.com/nearai/ironclaw/pull/323))
- copy missing files in Dockerfile to fix build ([#322](https://github.com/nearai/ironclaw/pull/322))
- fall back to build-from-source when extension download fails ([#312](https://github.com/nearai/ironclaw/pull/312))

### Other

- Add --version flag with clap built-in support and test ([#342](https://github.com/nearai/ironclaw/pull/342))
- Update FEATURE_PARITY.md ([#337](https://github.com/nearai/ironclaw/pull/337))
- add brew install ironclaw instructions ([#310](https://github.com/nearai/ironclaw/pull/310))
- Fix skills system: enable by default, fix registry and install ([#300](https://github.com/nearai/ironclaw/pull/300))

## [0.11.1](https://github.com/nearai/ironclaw/compare/v0.11.0...v0.11.1) - 2026-02-23

### Other

- Ignore out-of-date generated CI so custom release.yml jobs are allowed

## [0.11.0](https://github.com/nearai/ironclaw/compare/v0.10.0...v0.11.0) - 2026-02-23

### Fixed

- auto-compact and retry on ContextLengthExceeded ([#315](https://github.com/nearai/ironclaw/pull/315))

### Other

- *(README)* Adding badges to readme ([#316](https://github.com/nearai/ironclaw/pull/316))
- Feat/completion ([#240](https://github.com/nearai/ironclaw/pull/240))

## [0.10.0](https://github.com/nearai/ironclaw/compare/v0.9.0...v0.10.0) - 2026-02-22

### Added

- update dashboard favicon ([#309](https://github.com/nearai/ironclaw/pull/309))
- add web UI test skill for Chrome extension ([#302](https://github.com/nearai/ironclaw/pull/302))
- implement FullJob routine mode with scheduler dispatch ([#288](https://github.com/nearai/ironclaw/pull/288))
- hot-activate WASM channels, channel-first prompts, unified artifact resolution ([#297](https://github.com/nearai/ironclaw/pull/297))
- add pairing/permission system to all WASM channels and fix extension registry ([#286](https://github.com/nearai/ironclaw/pull/286))
- group chat privacy, channel-aware prompts, and safety hardening ([#285](https://github.com/nearai/ironclaw/pull/285))
- embedded registry catalog and WASM bundle install pipeline ([#283](https://github.com/nearai/ironclaw/pull/283))
- show token usage and cost tracker in gateway status popover ([#284](https://github.com/nearai/ironclaw/pull/284))
- support custom HTTP headers for OpenAI-compatible provider ([#269](https://github.com/nearai/ironclaw/pull/269))
- add smart routing provider for cost-optimized model selection ([#281](https://github.com/nearai/ironclaw/pull/281))

### Fixed

- persist user message at turn start before agentic loop ([#305](https://github.com/nearai/ironclaw/pull/305))
- block send until thread is selected ([#306](https://github.com/nearai/ironclaw/pull/306))
- reload chat history on SSE reconnect ([#307](https://github.com/nearai/ironclaw/pull/307))
- map Esc to interrupt and Ctrl+C to graceful quit ([#267](https://github.com/nearai/ironclaw/pull/267))

### Other

- Fix tool schema OpenAI compatibility ([#301](https://github.com/nearai/ironclaw/pull/301))
- simplify config resolution and consolidate main.rs init ([#287](https://github.com/nearai/ironclaw/pull/287))
- Update image source in README.md
- Add files via upload
- remove ExtensionSource::Bundled, use download-only install for WASM channels ([#293](https://github.com/nearai/ironclaw/pull/293))
- allow OAuth callback to work on remote servers (fixes #186) ([#212](https://github.com/nearai/ironclaw/pull/212))
- add rate limiting for built-in tools (closes #171) ([#276](https://github.com/nearai/ironclaw/pull/276))
- add LLM providers guide (OpenRouter, Together AI, Fireworks, Ollama, vLLM) ([#193](https://github.com/nearai/ironclaw/pull/193))
- Feat/html to markdown #106  ([#115](https://github.com/nearai/ironclaw/pull/115))
- adopt agent-market design language for web UI ([#282](https://github.com/nearai/ironclaw/pull/282))
- speed up startup from ~15s to ~2s ([#280](https://github.com/nearai/ironclaw/pull/280))
- consolidate tool approval into single param-aware method ([#274](https://github.com/nearai/ironclaw/pull/274))

## [0.9.0](https://github.com/nearai/ironclaw/compare/v0.8.0...v0.9.0) - 2026-02-21

### Added

- add TEE attestation shield to web gateway UI ([#275](https://github.com/nearai/ironclaw/pull/275))
- configurable tool iterations, auto-approve, and policy fix ([#251](https://github.com/nearai/ironclaw/pull/251))

### Fixed

- add X-Accel-Buffering header to SSE endpoints ([#277](https://github.com/nearai/ironclaw/pull/277))

## [0.8.0](https://github.com/nearai/ironclaw/compare/ironclaw-v0.7.0...ironclaw-v0.8.0) - 2026-02-20

### Added

- extension registry with metadata catalog and onboarding integration ([#238](https://github.com/nearai/ironclaw/pull/238))
- *(models)* add GPT-5.3 Codex, full GPT-5.x family, Claude 4.x series, o4-mini ([#197](https://github.com/nearai/ironclaw/pull/197))
- wire memory hygiene into the heartbeat loop ([#195](https://github.com/nearai/ironclaw/pull/195))

### Fixed

- persist WASM channel workspace writes across callbacks ([#264](https://github.com/nearai/ironclaw/pull/264))
- consolidate per-module ENV_MUTEX into crate-wide test lock ([#246](https://github.com/nearai/ironclaw/pull/246))
- remove auto-proceed fake user message injection from agent loop ([#255](https://github.com/nearai/ironclaw/pull/255))
- onboarding errors reset flow and remote server auth (#185, #186) ([#248](https://github.com/nearai/ironclaw/pull/248))
- parallelize tool call execution via JoinSet ([#219](https://github.com/nearai/ironclaw/pull/219)) ([#252](https://github.com/nearai/ironclaw/pull/252))
- prevent pipe deadlock in shell command execution ([#140](https://github.com/nearai/ironclaw/pull/140))
- persist turns after approval and add agent-level tests ([#250](https://github.com/nearai/ironclaw/pull/250))

### Other

- add automated PR labeling system ([#253](https://github.com/nearai/ironclaw/pull/253))
- update CLAUDE.md for recently merged features ([#183](https://github.com/nearai/ironclaw/pull/183))

## [0.7.0](https://github.com/nearai/ironclaw/compare/ironclaw-v0.6.0...ironclaw-v0.7.0) - 2026-02-19

### Added

- extend lifecycle hooks with declarative bundles ([#176](https://github.com/nearai/ironclaw/pull/176))
- support per-request model override in /v1/chat/completions ([#103](https://github.com/nearai/ironclaw/pull/103))

### Fixed

- harden openai-compatible provider, approval replay, and embeddings defaults ([#237](https://github.com/nearai/ironclaw/pull/237))
- Network Security Findings ([#201](https://github.com/nearai/ironclaw/pull/201))

### Added

- Refactored OpenAI-compatible chat completion routing to use the rig adapter and `RetryProvider` composition for custom base URL usage.
- Added Ollama embeddings provider support (`EMBEDDING_PROVIDER=ollama`, `OLLAMA_BASE_URL`) in workspace embeddings.
- Added migration `V9__flexible_embedding_dimension.sql` for flexible embedding vector dimensions.

### Changed

- Changed default sandbox image to `ironclaw-worker:latest` in config/settings/sandbox defaults.
- Improved tool-message sanitization and provider compatibility handling across NEAR AI, rig adapter, and shared LLM provider code.

### Fixed

- Fixed approval-input aliases (`a`, `/approve`, `/always`, `/deny`, etc.) in submission parsing.
- Fixed multi-tool approval resume flow by preserving and replaying deferred tool calls so all prior `tool_use` IDs receive matching `tool_result` messages.
- Fixed REPL quit/exit handling to route shutdown through the agent loop for graceful termination.

## [0.6.0](https://github.com/nearai/ironclaw/compare/ironclaw-v0.5.0...ironclaw-v0.6.0) - 2026-02-19

### Added

- add issue triage skill ([#200](https://github.com/nearai/ironclaw/pull/200))
- add PR triage dashboard skill ([#196](https://github.com/nearai/ironclaw/pull/196))
- add OpenRouter usage examples ([#189](https://github.com/nearai/ironclaw/pull/189))
- add Tinfoil private inference provider ([#62](https://github.com/nearai/ironclaw/pull/62))
- shell env scrubbing and command injection detection ([#164](https://github.com/nearai/ironclaw/pull/164))
- Add PR review tools, job monitor, and channel injection for E2E sandbox workflows ([#57](https://github.com/nearai/ironclaw/pull/57))
- Secure prompt-based skills system (Phases 1-4) ([#51](https://github.com/nearai/ironclaw/pull/51))
- Add benchmarking harness with spot suite ([#10](https://github.com/nearai/ironclaw/pull/10))
- 10 infrastructure improvements from zeroclaw ([#126](https://github.com/nearai/ironclaw/pull/126))

### Fixed

- *(rig)* prevent OpenAI Responses API panic on tool call IDs ([#182](https://github.com/nearai/ironclaw/pull/182))
- *(docs)* correct settings storage path in README ([#194](https://github.com/nearai/ironclaw/pull/194))
- OpenAI tool calling — schema normalization, missing types, and Responses API panic ([#132](https://github.com/nearai/ironclaw/pull/132))
- *(security)* prevent path traversal bypass in WASM HTTP allowlist ([#137](https://github.com/nearai/ironclaw/pull/137))
- persist OpenAI-compatible provider and respect embeddings disable ([#177](https://github.com/nearai/ironclaw/pull/177))
- remove .expect() calls in FailoverProvider::try_providers ([#156](https://github.com/nearai/ironclaw/pull/156))
- sentinel value collision in FailoverProvider cooldown ([#125](https://github.com/nearai/ironclaw/pull/125)) ([#154](https://github.com/nearai/ironclaw/pull/154))
- skills module audit cleanup ([#173](https://github.com/nearai/ironclaw/pull/173))

### Other

- Fix division by zero panic in ValueEstimator::is_profitable ([#139](https://github.com/nearai/ironclaw/pull/139))
- audit feature parity matrix against codebase and recent commits ([#202](https://github.com/nearai/ironclaw/pull/202))
- architecture improvements for contributor velocity ([#198](https://github.com/nearai/ironclaw/pull/198))
- fix rustfmt formatting from PR #137
- add .env.example examples for Ollama and OpenAI-compatible ([#110](https://github.com/nearai/ironclaw/pull/110))

## [0.5.0](https://github.com/nearai/ironclaw/compare/v0.4.0...v0.5.0) - 2026-02-17

### Added

- add cooldown management to FailoverProvider ([#114](https://github.com/nearai/ironclaw/pull/114))

## [0.4.0](https://github.com/nearai/ironclaw/compare/v0.3.0...v0.4.0) - 2026-02-17

### Added

- move per-invocation approval check into Tool trait ([#119](https://github.com/nearai/ironclaw/pull/119))
- add polished boot screen on CLI startup ([#118](https://github.com/nearai/ironclaw/pull/118))
- Add lifecycle hooks system with 6 interception points ([#18](https://github.com/nearai/ironclaw/pull/18))

### Other

- remove accidentally committed .sidecar and .todos directories ([#123](https://github.com/nearai/ironclaw/pull/123))

## [0.3.0](https://github.com/nearai/ironclaw/compare/v0.2.0...v0.3.0) - 2026-02-17

### Added

- direct api key and cheap model ([#116](https://github.com/nearai/ironclaw/pull/116))

## [0.2.0](https://github.com/nearai/ironclaw/compare/v0.1.3...v0.2.0) - 2026-02-16

### Added

- mark Ollama + OpenAI-compatible as implemented ([#102](https://github.com/nearai/ironclaw/pull/102))
- multi-provider inference + libSQL onboarding selection ([#92](https://github.com/nearai/ironclaw/pull/92))
- add multi-provider LLM failover with retry backoff ([#28](https://github.com/nearai/ironclaw/pull/28))
- add libSQL/Turso embedded database backend ([#47](https://github.com/nearai/ironclaw/pull/47))
- Move debug log truncation from agent loop to REPL channel ([#65](https://github.com/nearai/ironclaw/pull/65))

### Fixed

- shell destructive-command check bypassed by Value::Object arguments ([#72](https://github.com/nearai/ironclaw/pull/72))
- propagate real tool_call_id instead of hardcoded placeholder ([#73](https://github.com/nearai/ironclaw/pull/73))
- Fix wasm tool schemas and runtime ([#42](https://github.com/nearai/ironclaw/pull/42))
- flatten tool messages for NEAR AI cloud-api compatibility ([#41](https://github.com/nearai/ironclaw/pull/41))
- security hardening across all layers ([#35](https://github.com/nearai/ironclaw/pull/35))

### Other

- Explicitly enable cargo-dist caching for binary artifacts building
- Skip building binary artifacts on every PR
- add module specification rules to CLAUDE.md
- add setup/onboarding specification (src/setup/README.md)
- deduplicate tool code and remove dead stubs ([#98](https://github.com/nearai/ironclaw/pull/98))
- Reformat architecture diagram in README ([#64](https://github.com/nearai/ironclaw/pull/64))
- Add review discipline guidelines to CLAUDE.md ([#68](https://github.com/nearai/ironclaw/pull/68))
- Bump MSRV to 1.92, add GCP deployment files ([#40](https://github.com/nearai/ironclaw/pull/40))
- Add OpenAI-compatible HTTP API (/v1/chat/completions, /v1/models)   ([#31](https://github.com/nearai/ironclaw/pull/31))


## [0.1.3](https://github.com/nearai/ironclaw/compare/v0.1.2...v0.1.3) - 2026-02-12

### Other

- Enabled builds caching during CI/CD
- Disabled npm publishing as the name is already taken

## [0.1.2](https://github.com/nearai/ironclaw/compare/v0.1.1...v0.1.2) - 2026-02-12

### Other

- Added Installation instructions for the pre-built binaries
- Disabled Windows ARM64 builds as auto-updater [provided by cargo-dist] does not support this platform yet and it is not a common platform for us to support

## [0.1.1](https://github.com/nearai/ironclaw/compare/v0.1.0...v0.1.1) - 2026-02-12

### Other

- Renamed the secrets in release-plz.yml to match the configuration
- Make sure that the binaries release CD it kicking in after release-plz

## [0.1.0](https://github.com/nearai/ironclaw/releases/tag/v0.1.0) - 2026-02-12

### Added

- Add multi-provider LLM support via rig-core adapter ([#36](https://github.com/nearai/ironclaw/pull/36))
- Sandbox jobs ([#4](https://github.com/nearai/ironclaw/pull/4))
- Add Google Suite & Telegram WASM tools ([#9](https://github.com/nearai/ironclaw/pull/9))
- Improve CLI ([#5](https://github.com/nearai/ironclaw/pull/5))

### Fixed

- resolve runtime panic in Linux keychain integration ([#32](https://github.com/nearai/ironclaw/pull/32))

### Other

- Skip release-plz on forks
- Upgraded release-plz CD pipeline
- Added CI/CD and release pipelines ([#45](https://github.com/nearai/ironclaw/pull/45))
- DM pairing + Telegram channel improvements ([#17](https://github.com/nearai/ironclaw/pull/17))
- Fixes build, adds missing sse event and correct command ([#11](https://github.com/nearai/ironclaw/pull/11))
- Codex/feature parity pr hook ([#6](https://github.com/nearai/ironclaw/pull/6))
- Add WebSocket gateway and control plane ([#8](https://github.com/nearai/ironclaw/pull/8))
- select bundled Telegram channel and auto-install ([#3](https://github.com/nearai/ironclaw/pull/3))
- Adding skills for reusable work
- Fix MCP tool calls, approval loop, shutdown, and improve web UI
- Add auth mode, fix MCP token handling, and parallelize startup loading
- Merge remote-tracking branch 'origin/main' into ui
- Adding web UI
- Rename `setup` CLI command to `onboard` for compatibility
- Add in-chat extension discovery, auth, and activation system
- Add Telegram typing indicator via WIT on-status callback
- Add proactivity features: memory CLI, session pruning, self-repair notifications, slash commands, status diagnostics, context warnings
- Add hosted MCP server support with OAuth 2.1 and token refresh
- Add interactive setup wizard and persistent settings
- Rebrand to IronClaw with security-first mission
- Fix build_software tool stuck in planning mode loop
- Enable sandbox by default
- Fix Telegram Markdown formatting and clarify tool/memory distinctions
- Simplify Telegram channel config with host-injected tunnel/webhook settings
- Apply Telegram channel learnings to WhatsApp implementation
- Merge remote-tracking branch 'origin/main'
- Docker file for sandbox
- Replace hardcoded intent patterns with job tools
- Fix router test to match intentional job creation patterns
- Add Docker execution sandbox for secure shell command isolation
- Move setup wizard credentials to database storage
- Add interactive setup wizard for first-run configuration
- Add Telegram Bot API channel as WASM module
- Add OpenClaw feature parity tracking matrix
- Add Chat Completions API support and expand REPL debugging
- Implementing channels to be handled in wasm
- Support non interactive mode and model selection
- Implement tool approval, fix tool definition refresh, and wire embeddings
- Tool use
- Wiring more
- Add heartbeat integration, planning phase, and auto-repair
- Login flow
- Extend support for session management
- Adding builder capability
- Load tools at launch
- Fix multiline message rendering in TUI
- Parse NEAR AI alternative response format with output field
- Handle NEAR AI plain text responses
- Disable mouse capture to allow text selection in TUI
- Add verbose logging to debug empty NEAR AI responses
- Improve NEAR AI response parsing for varying response formats
- Show status/thinking messages in chat window, debug empty responses
- Add timeout and logging to NEAR AI provider
- Add status updates to show agent thinking/processing state
- Add CLI subcommands for WASM tool management
- Fix TUI shutdown: send /shutdown message and handle in agent loop
- Remove SimpleCliChannel, add Ctrl+D twice quit, redirect logs to TUI
- Fix TuiChannel integration and enable in main.rs
- Integrate Codex patterns: task scheduler, TUI, sessions, compaction
- Adding LICENSE
- Add README with IronClaw branding
- Add WASM sandbox secure API extension
- Wire database Store into agent loop
- Implementing WASM runtime
- Add workspace integration tests
- Compact memory_tree output format
- Replace memory_list with memory_tree tool
- Simplify workspace to path-based storage, remove legacy code
- Add NEAR AI chat-api as default LLM provider
- Add CLAUDE.md project documentation
- Add workspace and memory system (OpenClaw-inspired)
- Initial implementation of the agent framework
