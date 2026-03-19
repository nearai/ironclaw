# PR 草案: `feat(anp): add ANP identity foundation to ironclaw`

## Summary

- 为 `ironclaw` 增加实例级、可持久化的身份层，先为每个实例生成一个稳定的 `did:key`。
- 增加受保护的身份查询接口和 CLI 子命令，便于用户与集成方查看本机 DID 和 DID Document。
- 基于 `ironclaw` 当前能力生成 ANP Agent Description 预览，为后续 Discovery 和 Messaging 做准备。
- 将第一次 PR 严格控制在低风险范围内: 不引入公网发现、不实现 ANP 消息传输、不直接上 `did:wba`，也不改变现有 chat、tool、channel 行为。

## Change Type

- [ ] Bug fix
- [x] New feature
- [ ] Refactor
- [x] Documentation
- [ ] CI/Infrastructure
- [x] Security
- [ ] Dependencies

## Linked Issue

暂无。这个 PR 旨在成为 `ironclaw` 集成 ANP 的第一个增量步骤。

## 术语澄清

本文提到的“身份”如果没有特别说明，均指 **实例级密码学身份**，也就是 DID、私钥、公钥、DID Document 这一层。

这和 `ironclaw` 现有 workspace 里的 `IDENTITY.md`、`SOUL.md`、`AGENTS.md` 不是一回事。后者属于 prompt / memory / persona 文档，用于构造系统提示词；前者则是 agent-to-agent 互联时使用的可验证身份。

为避免 maintainer 混淆，建议在实现时优先使用更明确的命名，例如:

- `src/did/`
- `src/instance_identity/`

本文后续统一使用“实例身份层”来指代这套新增的密码学身份基础设施。

## 为什么是 ANP

### ANP 是什么

ANP, Agent Network Protocol, 是一个面向智能体互联互通的开源协议项目。它的定位非常明确: 成为 “Agentic Web 时代的 HTTP”。

ANP 当前公开的协议设计主要分为三层:

- 身份与安全通信层
- Meta-Protocol 协商层
- 应用协议层, 包括 Agent Description、Agent Discovery、Messaging 等

如果 `ironclaw` maintainer 之前没有接触过 ANP，最值得先看的材料是:

- [ANP 仓库](https://github.com/agent-network-protocol/AgentNetworkProtocol)
- [ANP README](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/README.md)
- [ANP 技术白皮书](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/01-agentnetworkprotocol-technical-white-paper.md)
- [ANP Agent Description 规范](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/07-anp-agent-description-protocol-specification.md)
- [ANP Agent Discovery 规范](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/08-ANP-Agent-Discovery-Protocol-Specification.md)
- [ANP Messaging 规范](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/09-ANP-end-to-end-instant-messaging-protocol-specification.md)

### ANP 社区是什么

ANP 不是某个封闭产品的私有接口，而是一个以开源社区方式推进的协议项目。公开材料里已经明确给出了:

- 协议规范仓库
- 开源贡献入口
- 社区沟通方式
- 与协议配套的实现方向 `AgentConnect`

可供 maintainer 了解社区背景的入口包括:

- [ANP CONTRIBUTING 指南](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/CONTRIBUTING.md)
- [AgentConnect 实现仓库](https://github.com/agent-network-protocol/AgentConnect)
- [ANP 官方网站](https://agent-network-protocol.com/)

对 `ironclaw` 来说，ANP 的价值不在于“替换现有能力”，而在于提供一个开放、标准化、可演进的 agent interoperability 方向，让 `ironclaw` 未来具备与外部 agent 网络协作的清晰路径。

## 为什么这种接法适合 `ironclaw`

这个方案不是把整套 ANP 一次性硬塞进 `ironclaw`，而是沿着 `ironclaw` 已有的稳定边界做增量集成。

- `ironclaw` 已经有统一的本地基目录抽象，这正适合存放实例级身份文件: [src/bootstrap.rs](https://github.com/nearai/ironclaw/blob/main/src/bootstrap.rs)
- Web gateway 已经支持共享状态注入和受保护 API，这使它天然适合承载身份查询接口: [src/channels/web/mod.rs](https://github.com/nearai/ironclaw/blob/main/src/channels/web/mod.rs), [src/channels/web/server.rs](https://github.com/nearai/ironclaw/blob/main/src/channels/web/server.rs)
- Session 路由已经会把外部标识映射到内部 thread，后续 ANP conversation 可以直接复用这个机制: [src/agent/session_manager.rs](https://github.com/nearai/ironclaw/blob/main/src/agent/session_manager.rs)
- `ironclaw` 已经有 pending request 和 allowlist 的 pairing 模型，后续可以自然演进到 DID trust store: [src/pairing/store.rs](https://github.com/nearai/ironclaw/blob/main/src/pairing/store.rs)
- Tool registry 已经能导出结构化工具定义和 JSON schema，并且已有 `tool_info` 发现能力，后续可以增强 Agent Description: [src/tools/registry.rs](https://github.com/nearai/ironclaw/blob/main/src/tools/registry.rs)
- 配置链路已经清晰分成 `Settings` 持久化层和 `Config` 运行时解析层，后续如果增加公开 DID 或发布开关，有明确接入位置: [src/settings.rs](https://github.com/nearai/ironclaw/blob/main/src/settings.rs), [src/config/mod.rs](https://github.com/nearai/ironclaw/blob/main/src/config/mod.rs)

这意味着第一次 PR 不需要改写 `ironclaw` 的核心 agent loop，也不需要提前引入高风险公网行为。

## 为什么第一次 PR 先用 `did:key`

ANP 的长期公网身份方向与 `did:wba` 是兼容的，相关规范见:

- [did:wba 方法规范](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/03-did-wba-method-design-specification.md)

但 `ironclaw` 当前明显是 local-first 的:

- 很多实例没有稳定域名
- 很多实例没有长期可用的 HTTPS 地址
- 现有 tunnel URL 适合临时入站，不适合作为持久身份

因此，第一次 PR 采用一个非常保守但稳妥的规则:

- 每个 `ironclaw` 实例先拥有一个稳定的本地根身份 `did:key`
- 后续身份框架允许支持多个 DID method 的别名
- 但前几次 PR 中，唯一正式支持并对外宣称兼容的公网别名应是 `did:wba`

这样做的好处是，PR1 既能立刻给 `ironclaw` 带来真实价值，又不会把第一个 ANP PR 变成“大规模公网身份系统改造”。

这里还应额外说明一件事:

- `did:key` 适合作为本地根身份、自举身份、恢复身份
- 后续公网身份连续性与密钥轮换的主路径，不应建立在“多个 `did:key` 别名等价”之上
- 更合理的长期路径是: 本地保留根 `did:key`，对外公开身份使用可更新的 `did:wba`，并在其 `verificationMethod` 层处理后续轮换

## 本 PR 的范围

这个 PR 只做 ANP 集成的基础层。

它会做:

- 为实例创建并持久化稳定 DID
- 通过 CLI 和受保护 gateway API 暴露 DID 与 DID Document
- 生成 ANP Agent Description 预览
- 为后续 Discovery、Trust、Messaging 留出干净的扩展位

它不会做:

- 公网发现
- ANP 消息传输
- `did:wba`
- 公网 challenge / nonce / replay protection
- 现有 channel、tool、chat 语义变更

## 本 PR 实现的内容

### 1. 实例级持久化身份

新增一个独立的实例身份模块，命名建议优先使用 `src/did/`，以避免与 workspace `IDENTITY.md` 产生概念冲突。该模块负责:

- 首次启动生成密钥
- 将身份持久化到 `~/.ironclaw/identity/instance.json`
- 重启后加载已有身份
- 导出当前实例的 DID Document

PR1 使用 `did:key` 作为实例身份。

建议模块结构:

- `src/did/mod.rs`
- `src/did/store.rs`
- `src/did/did_key.rs`
- `src/did/document.rs`

### 2. ANP 元数据层

新增 `src/anp/` 模块，负责 ANP 相关元数据生成，首期只做 Agent Description。

建议模块结构:

- `src/anp/mod.rs`
- `src/anp/agent_description.rs`

PR1 生成的 Agent Description 应该是一个“诚实的预览”，即:

- 反映 `ironclaw` 当前真实具备的能力
- 不夸大尚未实现的 ANP 子协议
- 为后续公开发布和 richer interface export 做铺垫

首版至少应描述:

- 协议家族与版本
- 实例名称或展示名称
- 当前实例 DID
- 自然语言入口
- PR1 阶段确实可以对外声明的结构化接口

### 3. 启动期集成

在正常启动流程中加载 identity，并将其注入 gateway shared state。

主要接入点:

- [src/main.rs](https://github.com/nearai/ironclaw/blob/main/src/main.rs)
- [src/channels/web/mod.rs](https://github.com/nearai/ironclaw/blob/main/src/channels/web/mod.rs)
- [src/channels/web/server.rs](https://github.com/nearai/ironclaw/blob/main/src/channels/web/server.rs)

这样可以保证实例身份层是“实例级基础设施”，而不是某个 channel 或数据库生命周期的一部分。

结合最新版启动流程，建议的挂载时机是:

- 在 `AppBuilder::build_all()` 之后加载或解析实例身份
- 在 gateway 构造前把身份对象准备好
- 如果后续 PR 需要把公开 service endpoint 写入对外元数据，则应在 tunnel 启动后再补齐公开部分

另外，最新版 gateway 的接入约束比文档初稿里更明确。若 PR1 给 `GatewayState` 新增 `identity` 字段，提交实现时必须同步更新:

- `GatewayChannel::new()`
- `GatewayChannel::rebuild_state()`
- `GatewayState` 结构体本身
- `src/channels/web/test_helpers.rs` 里的 `TestGatewayBuilder`
- 以及仓库中所有手写 `GatewayState { ... }` 的测试构造代码

### 4. CLI 支持

新增 CLI 子命令:

- `ironclaw did show`
- `ironclaw did document`

同时扩展 `ironclaw status`，展示:

- 当前 DID
- identity 文件路径

主要接入点:

- [src/cli/mod.rs](https://github.com/nearai/ironclaw/blob/main/src/cli/mod.rs)
- [src/cli/status.rs](https://github.com/nearai/ironclaw/blob/main/src/cli/status.rs)

这里还有一个实现边界需要提前写清楚:

- `status` 目前是一个轻量诊断命令，优先读取本地 settings / config 文件，不应因为增加 DID 而变成一个需要 DB、keychain 或完整 async config 解析的重命令
- 因此 `status` 中展示 DID 时，建议直接读取本地实例身份文件，而不是走完整运行时依赖链

### 5. 受保护的 Gateway API

在现有 gateway auth 保护下新增身份查询接口:

- `GET /api/identity`
- `GET /api/identity/did-document`
- `GET /api/identity/agent-description`

这些接口是给本地 UI、CLI 和集成方检查本机身份用的，不是公网 discovery endpoint。

### 6. 配置入口

PR1 本身不强制引入新的用户可见配置项；但如果后续 PR 增加下面这类开关:

- `identity.public_base_url`
- `identity.publish_enabled`
- `identity.public_alias_method`

则应明确同时进入:

- `Settings` 持久化层
- `Config` 运行时解析层

这样才能与 `ironclaw` 当前的 JSON/TOML/env 配置链路保持一致。

## 用户可见效果

这个 PR 合并后:

- 每个 `ironclaw` 实例都会自动拥有稳定 DID
- 重启不会改变 DID
- 用户可以通过 CLI 和本地 gateway 查看 DID
- 用户可以导出本机 DID Document
- maintainer 与集成方可以直接检查 `ironclaw` 生成的 ANP Agent Description 预览
- 现有 chat、tool、channel、pairing 和 OpenAI-compatible 行为保持不变

## PR1 明确不做的内容

为了让第一次 ANP PR 足够小、足够容易 review，以下内容明确不在 PR1 范围内:

- `did:wba` 发布
- `/.well-known/did.json`
- `/.well-known/agent-descriptions`
- 公开 `ad.json`
- DID-WBA challenge flow
- nonce / replay protection
- DID trust store
- ANP HTTP JSON-RPC 入口，例如 `/api/v1/messages/rpc`
- WebSocket 消息传输
- E2EE、HPKE、群消息
- Meta-Protocol 协商

这些内容应在后续 PR 中继续推进，而不是在第一次 PR 中混在一起提交。

## Validation

第一次 PR 应该自带一组清晰的自动化测试，让 review scope 一眼可见。下面的表格可以直接作为提交 PR 时的验证清单。

| PR1 状态 | 测试用例 | 建议位置 | 目的 |
| --- | --- | --- | --- |
| PASS | 首次启动会生成 `did:key` 身份 | `src/did/*` 单元测试 | 确保实例身份自动创建 |
| PASS | 重新加载后 DID 保持不变 | `src/did/*` 单元测试 | 确保身份跨重启稳定 |
| PASS | Unix 下 identity 文件权限被收紧 | `src/did/store.rs` 单元测试 | 防止私钥材料可被任意读取 |
| PASS | DID Document 包含可用于 authentication 的 verification method | `src/did/document.rs` 单元测试 | 确保导出的文档可用 |
| PASS | `ironclaw did show` 返回期望 DID | 集成测试，例如 `tests/anp_identity_integration.rs` | 验证 CLI 集成 |
| PASS | `ironclaw did document` 返回合法 JSON | 集成测试，例如 `tests/anp_identity_integration.rs` | 验证 CLI 文档导出 |
| PASS | `ironclaw status` 显示 DID 和 identity 路径 | 集成测试，例如 `tests/anp_identity_integration.rs` | 验证用户可见状态输出 |
| PASS | `/api/identity` 受 gateway auth 保护 | gateway 集成测试 | 确保没有意外公网暴露 |
| PASS | `/api/identity/did-document` 返回当前 DID Document | gateway 集成测试 | 验证 API 文档导出 |
| PASS | `/api/identity/agent-description` 返回 ANP Agent Description 预览 | gateway 集成测试 | 验证 ANP 元数据导出 |
| PASS | 默认情况下没有新增公开 ANP 路由 | gateway 集成测试 | 确保 PR1 仍然保持 local-first |

最新版仓库里，gateway 测试已经有现成的 `TestGatewayBuilder` 可复用。PR1 的 gateway 集成测试应优先基于它构建，而不是每个测试都重新拼装一份状态对象。

另外，提交实现时别遗漏那些手写 `GatewayState { ... }` 的测试文件；如果新增 `GatewayState.identity` 字段，它们都必须同步更新，否则第一次 PR 很容易在 unrelated test 上翻车。

本次实现实际通过的 Docker 验证命令:

```bash
docker-tests/run_pr1_tdd.sh
docker-tests/run_pr1_tdd.sh cargo test did --lib --no-default-features --features libsql
docker-tests/run_pr1_tdd.sh cargo test anp --lib --no-default-features --features libsql
docker-tests/run_pr1_tdd.sh cargo test --test ws_gateway_integration --no-default-features --features libsql
docker-tests/run_pr1_tdd.sh cargo test --test openai_compat_integration --no-default-features --features libsql
docker-tests/run_pr1_tdd.sh cargo fmt --all -- --check
```

这些命令全部在 PR1 开发过程中执行并通过。

建议在提交 PR 时附上的手工验证说明:

- 用一个全新的 base dir 启动 `ironclaw`，确认 identity 文件被自动创建
- 运行 `ironclaw did show`，确认重启前后 DID 保持不变
- 带 auth 和不带 auth 分别调用身份接口，确认行为符合预期
- 确认没有因为启用身份层而默认暴露 discovery endpoint

## Security Impact

这个 PR 会首次为 `ironclaw` 引入持久化私钥材料，因此主要安全点集中在:

- 本地文件权限
- 受保护接口与公开接口的边界
- 身份层与现有 bearer token / pairing 的职责分离

PR1 的安全影响应保持可控:

- 私钥只保存在本地文件，不落数据库
- 身份查询接口沿用现有 gateway auth 保护
- 默认不新增公网入口
- 默认不自动信任任何远端 agent

## Database Impact

None.

PR1 不应引入 migration，也不应改变现有数据库 schema。

## Blast Radius

这个 PR 触及的主要子系统:

- 启动流程
- CLI status 与 DID 子命令
- gateway state 和受保护身份路由
- gateway 测试辅助层与手工 `GatewayState` 构造测试
- 新增本地 identity 持久化
- 新增 ANP 元数据序列化

潜在风险点:

- identity 初始化如果不够健壮，可能影响启动
- `status` 输出格式可能变化
- gateway route 注册如果不完整，可能导致状态注入缺失
- 如果 `GatewayState` 新字段没有同步更新测试辅助层，集成测试会大面积失败

由于 PR1 不改动现有 message loop 和 channel 行为，整体 blast radius 是可控的。

## Rollback Plan

如果这个 PR 产生问题，回滚路径很简单:

- 回滚代码
- 去掉新增的身份路由注册
- 保留已生成的 `~/.ironclaw/identity/instance.json` 文件但不再使用

由于 PR1 不会迁移数据库，也不会改变现有消息处理逻辑，因此回滚时不需要额外做数据修复。

## 后续 PR 路线图

### PR2: 公网 DID 与 Discovery

增加可选的公网身份发布能力:

- 引入 method-extensible 的别名框架
- 但正式支持的第一个公网别名方法应是 `did:wba`
- 本地根 `did:key` 继续保留
- 要求显式配置稳定公网 HTTPS base URL
- 公开发布 DID Document
- 公开发布 ANP Agent Description
- 增加 `/.well-known/agent-descriptions`
- 公网身份的连续性和后续轮换优先通过 `did:wba` 的 `verificationMethod` 处理，而不是通过不断生成新的 `did:key` 别名来模拟

### PR3: DID Trust Store

增加基于 DID 的信任管理:

- pending peers
- trusted peers
- blocked peers
- review / approve workflow

这一层应借鉴当前 pairing 的产品模式，但不能直接复用 channel sender ID 的语义和存储格式。

### PR4: ANP Messaging Foundation

增加 ANP 消息传输基础层:

- ANP HTTP JSON-RPC 入口
- message envelope 校验
- 将 ANP conversation 映射到 `SessionManager`
- 先做文本消息流，再扩展更复杂 transport

相关 ANP 参考:

- [ANP Messaging 规范](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/09-ANP-end-to-end-instant-messaging-protocol-specification.md)

### PR5: 认证增强与更丰富的 Agent Description

增加:

- DID-WBA challenge 处理
- nonce / timestamp 校验
- replay protection
- 更丰富的 structured interface export
- 在合适边界下考虑 OpenRPC 风格能力导出

### PR6: Meta-Protocol 与高级 Messaging

只有在前面几层稳定之后再考虑:

- Meta-Protocol 协商
- 可选 WebSocket transport
- E2EE 和高级消息能力, 前提是 maintainer 认为产品上确实有必要

相关 ANP 参考:

- [ANP Meta-Protocol 规范](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/06-anp-agent-communication-meta-protocol-specification.md)

## 为什么这个 review 策略对 maintainer 友好

这条路线的核心优点是:

- PR1 足够小, 易于 review
- PR1 本身就有独立价值, 即使后续 ANP 工作暂时不继续也不会浪费
- PR1 不会把 `ironclaw` 突然变成一个公网消息服务器
- PR1 避开了 ANP 里最复杂、最容易争议的部分
- 后续 PR 可以建立在一个干净的 identity foundation 之上，而不是返工

这也是为什么第一次 PR 应该先做 identity 和 metadata，而不是直接上 messaging。

## Repo Policy Note

提交实现版 PR 时，还应检查仓库的 feature parity 规则:

- 如果这次改动会影响 `FEATURE_PARITY.md` 中已跟踪特性的实现状态，需要在同一个分支里一并更新
- 如果 maintainer 希望把 DID / ANP 集成也纳入 parity 跟踪，可以在首次实现 PR 中补充对应条目

## References

### `ironclaw`

- [Repository](https://github.com/nearai/ironclaw)
- [README](https://github.com/nearai/ironclaw/blob/main/README.md)
- [PR 模板](https://github.com/nearai/ironclaw/blob/main/.github/pull_request_template.md)
- [Bootstrap 与 base dir 处理](https://github.com/nearai/ironclaw/blob/main/src/bootstrap.rs)
- [Gateway state 注入](https://github.com/nearai/ironclaw/blob/main/src/channels/web/mod.rs)
- [Gateway 路由处理](https://github.com/nearai/ironclaw/blob/main/src/channels/web/server.rs)
- [Gateway test helpers](https://github.com/nearai/ironclaw/blob/main/src/channels/web/test_helpers.rs)
- [Session manager](https://github.com/nearai/ironclaw/blob/main/src/agent/session_manager.rs)
- [Pairing store](https://github.com/nearai/ironclaw/blob/main/src/pairing/store.rs)
- [Tool registry](https://github.com/nearai/ironclaw/blob/main/src/tools/registry.rs)
- [Workspace identity document definitions](https://github.com/nearai/ironclaw/blob/main/src/workspace/document.rs)
- [Settings persistence](https://github.com/nearai/ironclaw/blob/main/src/settings.rs)
- [Runtime config assembly](https://github.com/nearai/ironclaw/blob/main/src/config/mod.rs)
- [FEATURE_PARITY policy entry point](https://github.com/nearai/ironclaw/blob/main/FEATURE_PARITY.md)

### ANP

- [Repository](https://github.com/agent-network-protocol/AgentNetworkProtocol)
- [README](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/README.md)
- [技术白皮书](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/01-agentnetworkprotocol-technical-white-paper.md)
- [did:wba 方法规范](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/03-did-wba-method-design-specification.md)
- [Agent Description 规范](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/07-anp-agent-description-protocol-specification.md)
- [Agent Discovery 规范](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/08-ANP-Agent-Discovery-Protocol-Specification.md)
- [Messaging 规范](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/09-ANP-end-to-end-instant-messaging-protocol-specification.md)
- [CONTRIBUTING](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/CONTRIBUTING.md)
- [AgentConnect 实现仓库](https://github.com/agent-network-protocol/AgentConnect)

---

**Review track**: `B`
