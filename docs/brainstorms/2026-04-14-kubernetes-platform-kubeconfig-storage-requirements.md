---
date: 2026-04-14
topic: kubernetes-platform-kubeconfig-storage
---

# Kubernetes Platform Kubeconfig Storage

## Problem Frame

IronClaw 现有的 Kubernetes runtime 已能连接 cluster、创建 worker Pod，并通过
orchestrator 向 job 下发项目快照与运行时配置。但 Kubernetes 连接凭证目前仍
依赖运行环境默认提供的 kubeconfig 或 in-cluster config，产品层没有清楚定义：
哪些场景应直接使用 ServiceAccount，哪些场景需要平台托管 kubeconfig，以及这类
凭证应存放在哪一层。

如果这个边界不明确，后续实现很容易把 Kubernetes 平台凭证与 job 运行时凭证混
在一起，导致安全模型、审计范围与失败行为都变得含糊。

## Requirements

**Credential Ownership**
- R1. Kubernetes 连接凭证必须被定义为 IronClaw 平台级凭证，用于主进程连接
  Kubernetes API，而不是 sandbox job 内部凭证。
- R2. Kubernetes 连接凭证不得进入 worker Pod、sandbox 容器或 job 的临时凭证
  授权链路。
- R3. Job 运行时凭证与 Kubernetes 平台连接凭证必须在产品概念、存储位置与访问
  路径上明确分离。

**Resolution Order**
- R4. 当 IronClaw 运行在 Kubernetes 集群内且具备可用的 in-cluster 身份时，
  系统必须优先使用 in-cluster config，不要求额外保存 kubeconfig。
- R5. 当 in-cluster config 不可用且部署目标仍需连接 Kubernetes 时，系统必须
  支持使用平台级加密 kubeconfig 作为后备来源。
- R6. 本地开发与临时运维场景可以继续使用显式 `KUBECONFIG` 或系统默认
  kubeconfig 作为兜底路径，但这类路径应被表述为开发/运维兼容机制，而不是主推
  产品配置方式。

**Storage and Access Boundaries**
- R7. 持久化的 kubeconfig 必须存入现有加密 secrets 体系或等价的加密秘密存储，
  不得以明文形式存入普通 settings、项目目录或 job 配置。
- R8. 普通 settings 只应保存非敏感引用信息，例如是否启用 Kubernetes、目标
  namespace、以及平台级 kubeconfig 的引用标识；不得保存 kubeconfig 明文。
- R9. 读取平台级 kubeconfig 的行为必须限制在主进程侧完成，并遵循现有 secrets
  的访问控制与审计能力。

**Operational Behavior**
- R10. Setup、doctor 与相关管理界面必须能清楚说明当前 Kubernetes 凭证来源：
  in-cluster、平台级加密 kubeconfig，或本地/环境变量兜底。
- R11. 当系统缺少可用的 Kubernetes 凭证时，失败信息必须明确指出缺的是平台连
  接配置，而不是 job 凭证、namespace 或项目内容。
- R12. 当平台使用加密 kubeconfig 时，产品必须让操作者理解这是实例级或平台级
  配置，而不是每个用户、每个 job 单独上传的凭证。

## Success Criteria

- 用户与维护者能明确区分“连接 Kubernetes 的平台凭证”与“job 运行时凭证”。
- 集群内部署默认走 in-cluster config，不需要额外配置 kubeconfig。
- 集群外部署可以通过平台级加密 kubeconfig 正常连接 Kubernetes，而不会把凭证
  暴露给 worker Pod。
- settings、setup、doctor 与错误提示对凭证来源与缺失原因的表达一致，不产生误导。

## Scope Boundaries

- 不在这份 requirements 中定义 kubeconfig 的具体字段映射、解析方式或代码结构。
- 不把 kubeconfig 设计为每个用户各自维护的凭证。
- 不把 kubeconfig 纳入 job 的临时 credential grant、bootstrap artifact 或环境
  变量下发流程。
- 不在本次范围内重新设计 Kubernetes RBAC、namespace 自动创建或 cluster 多租户
  策略。

## Key Decisions

- **In-cluster 优先**：当 IronClaw 已运行在 Kubernetes 内部时，优先依赖平台原生
  身份，而不是额外保存 kubeconfig。
- **平台级加密存储是集群外部署的主路径**：需要持久化 kubeconfig 时，进入加密
  secrets 体系，而不是普通 settings。
- **本地 kubeconfig 仅作为兼容兜底**：保留开发便利性，但不把它作为正式产品模
  型的中心。
- **Kubernetes 凭证不进入 job 边界**：这类凭证只服务于主进程访问 Kubernetes API，
  不属于 sandbox job 的可见资源。

## Dependencies / Assumptions

- 现有 secrets 存储可承担加密保存平台级 kubeconfig 的职责。
- 现有 setup / doctor / runtime 诊断路径可以扩展为展示凭证来源与缺失原因。
- 运行环境允许可靠地区分 in-cluster config 与外部 kubeconfig 场景。

## Outstanding Questions

### Deferred to Planning
- [Affects R5][Technical] 平台级加密 kubeconfig 的引用应采用单一固定 secret 名称，
  还是允许多份命名配置以支持多 cluster 切换？
- [Affects R8][Technical] 普通 settings 中应保留哪些最小引用字段，才能让 setup、
  doctor 与运行时诊断表达清楚而不泄露敏感信息？
- [Affects R10][Needs research] 哪些产品入口最需要展示“当前凭证来源”，才能让操
  作者快速定位连接问题而不过度增加配置复杂度？

## Next Steps

→ /prompts:ce-plan for structured implementation planning
