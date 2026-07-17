---
title: "本地"
description: "通过终端或浏览器在本地使用 IronClaw"
icon: keyboard
---

默认情况下，IronClaw 提供两种本地界面与智能体对话：

- **终端界面 (TUI)：** 直接在终端中对话
- **Web 网关：** 通过本地 HTTP 服务器在浏览器中对话

<Note>
如果您还没有设置智能体，请先查看我们的[快速开始指南](../quickstart)
</Note>

---

## 终端界面

只需运行 `ironclaw`，TUI 将在终端中启动。使用以下快捷键进行导航和对话。
| 按键 | 操作 |
|-----|--------|
| `Enter` | 发送消息 |
| `Shift+Enter` | 在编辑器中换行 |
| `Ctrl+C` | 退出 |
| `Ctrl+L` | 清屏 |
| `Tab` | 聚焦下一个元素 |
| `Esc` | 取消或返回 |
| `Up/Down` | 滚动历史记录 |

### 配置

| 选项 | 默认值 | 描述 |
|--------|---------|-------------|
| `CLI_ENABLED` | `true` | 启用或禁用终端界面 |


---

## Web 网关

| 选项 | 默认值 | 描述 |
|--------|---------|-------------|
| `GATEWAY_HOST` | `127.0.0.1` | Web 网关的主机接口 |
| `GATEWAY_PORT` | `3000` | Web 网关使用的端口 |
| `GATEWAY_ENABLED` | `true` | 启用或禁用 Web 网关 |
| `GATEWAY_AUTH_TOKEN` | 自动生成 | 打开 Web UI 所需的认证令牌 |

### 认证

默认情况下，IronClaw 在启动时生成认证令牌并在日志中打印。要在重启间使用固定令牌：

```bash
export GATEWAY_AUTH_TOKEN="your-secure-token-here"
```

生成令牌：

```bash
openssl rand -hex 32
```

### API 端点

Web 网关还暴露本地端点：

| 端点 | 描述 |
|----------|-------------|
| `GET /api/status` | 服务器状态 |
| `POST /api/chat` | 发送消息 |
| `GET /api/jobs` | 列出任务 |
| `GET /api/memory` | 搜索记忆 |

### 网络访问

使用仅本地访问（推荐）：

```bash
export GATEWAY_HOST=127.0.0.1
```

使用局域网访问：

```bash
export GATEWAY_HOST=0.0.0.0
```

<Warning>
使用 `0.0.0.0` 时，请使用强认证令牌，并在将服务暴露到本地网络之外之前，将其置于 HTTPS/反向代理后面。
</Warning>

---

## 故障排除

<AccordionGroup>
  <Accordion title="终端显示问题">
    - 确保您的终端支持 Unicode 和 256 色
    - 设置 `TERM=xterm-256color`
    - 重启终端会话
  </Accordion>

  <Accordion title="终端输入问题">
    - 检查终端焦点
    - 运行 `reset`
    - 禁用冲突的终端鼠标模式
  </Accordion>

  <Accordion title="Web UI 连接被拒绝">
    - 确认 `ironclaw run` 正在运行
    - 检查 `GATEWAY_PORT` 值
    - 确认主机和防火墙设置
  </Accordion>

  <Accordion title="Web UI 令牌被拒绝">
    - 从启动日志中精确复制令牌
    - 移除尾部空格
    - 设置持久的 `GATEWAY_AUTH_TOKEN`
  </Accordion>

  <Accordion title="WebSocket 断开连接">
    - 检查本地网络/代理稳定性
    - 确认反向代理支持 WebSocket 升级
    - 检查浏览器控制台日志
  </Accordion>
</AccordionGroup>
