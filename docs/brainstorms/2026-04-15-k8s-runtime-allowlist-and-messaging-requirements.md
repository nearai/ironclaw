---
date: 2026-04-15
topic: k8s-runtime-allowlist-and-messaging
---

# K8s Runtime Allowlist And Messaging

## Problem Frame

`k8s-runtime` 当前已经具备 Stage 2 project-backed runtime 的基础能力，但
关于 Kubernetes 一次性沙箱命令的 allowlist 网络支持，以及相关用户提示，
口径仍然偏宽或互相牵制。当前实现把 allowlist 可用性与更大的 Stage 3
前提混在一起表达，导致“什么时候 Kubernetes 可以承接 allowlist 受限命令”
不够直接；同时 setup、doctor、能力说明里的文案，也仍然沿用更宽的阶段性
说法，不利于把当前实现边界说明清楚。

这次只收口 `k8s-runtime` 的现有功能实现，不扩大能力范围，不引入新的
平台承诺，也不把范围延伸到大仓库边界或更大的 Docker 体验对齐。

## Requirements

**Allowlist 判定**
- R1. Kubernetes 上一次性沙箱命令是否可承接 allowlist 受限网络访问，必须只由
  `native network controls` 是否就绪决定。
- R2. `projected runtime config` 不能再作为 allowlist 放开的前置条件。
- R3. 当 `native network controls` 未就绪时，依赖 allowlist 的一次性沙箱命令
  必须明确拒绝，并给出改用 Docker 或补齐集群条件的清楚说明。
- R4. 当 `native network controls` 已就绪时，Kubernetes 一次性沙箱命令可以承接
  allowlist 受限网络访问，不应继续因为无关前提而被拦住。

**状态与提示**
- R5. setup、doctor、运行时失败提示、能力说明中的用户可见表述，必须与当前
  allowlist 判定规则一致。
- R6. 用户可见表述必须明确区分两类条件：
  `native network controls` 决定 allowlist 网络是否可用；
  `projected runtime config` 只决定运行时配置文件交付方式。
- R7. 用户可见表述不得再把 allowlist 可用性描述成“必须等待所有 Stage 3 前提”
  才能成立。
- R8. 所有提示都必须继续保留当前范围边界：这次不新增大仓库支持范围，不扩写
  更大的 Kubernetes parity 承诺。

**范围控制**
- R9. 本次只调整 `k8s-runtime` 功能判定与用户可见说明，不扩展新的执行模式、
  新的网络机制或新的管理员开关。
- R10. 本次不处理大仓库 admission、项目级 override、或更广义的 Stage 3
  收尾工作。

## Success Criteria

- Kubernetes 一次性沙箱命令的 allowlist 放开条件可以用一句清楚的话描述：
  只看 `native network controls` 是否就绪。
- 当条件不满足时，拒绝原因和下一步动作对用户是清楚的。
- setup、doctor、能力说明、运行时错误提示不再互相矛盾，也不再把 allowlist
  能力和无关条件绑在一起。
- 文档与提示都不把本次改动表述成更大的 Kubernetes parity 扩展。

## Scope Boundaries

- 不新增大仓库边界规则。
- 不新增项目级 override 机制。
- 不重写整体三阶段路线图。
- 不要求自动探测集群网络能力；继续以显式配置为主，只补更清楚的校验与提示。
- 不把 `projected runtime config` 改成 allowlist 能力的隐含依赖。

## Key Decisions

- **只按网络条件放开 allowlist**：allowlist 网络是否可用，只由
  `native network controls` 决定。
- **继续沿用显式配置入口**：不新增完整自动探测；本次仅在现有判定入口上补清楚
  失败说明和状态呈现。
- **文案与实现同等重要**：如果代码已允许或拒绝，setup、doctor、能力说明与错误
  提示都必须同步说清楚。
- **不借题扩范围**：本次不把 allowlist 收口延伸成大仓库收口、Stage 3 完成，
  或更大的 Docker 体验对齐。

## Dependencies / Assumptions

- 现有 `IRONCLAW_K8S_NATIVE_NETWORK_CONTROLS` 继续作为 allowlist 放开条件的
  显式输入。
- 现有 `IRONCLAW_K8S_PROJECTED_RUNTIME_CONFIG` 继续只表示运行时配置文件交付能力，
  不再作为 allowlist 判定依赖。
- 当前 `k8s-runtime` 的网络能力边界仍由现有实现承接，本次只收口判定和说明，
  不引入新的底层网络机制。

## Outstanding Questions

### Deferred to Planning
- [Affects R5-R7][Technical] 哪些用户可见入口必须一并对齐，才能避免同一能力在
  setup、doctor、运行时报错、能力文档之间仍然出现口径差异？
- [Affects R3-R7][Technical] 哪些现有提示应改成“allowlist 只看网络条件”，哪些仍应
  保留更宽的阶段性背景说明但不能影响判定？

## Next Steps

→ /prompts:ce-plan for structured implementation planning
