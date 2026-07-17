---
title: "Gmail"
description: "让您的智能体读取、发送并管理 Gmail 邮件"
---

Gmail 扩展允许智能体直接操作您的 Gmail 收件箱，包括列出与搜索邮件、读取正文、发送新邮件、创建草稿、回复线程以及移动到垃圾箱。适合自动化邮件流程、监控关键会话和发送通知。

---

## 设置

如果您还没有完成 Google OAuth，请先完成 [Google OAuth 设置](/zh/extensions/google/oauth-setup)。

<Steps>

<Step title="启用 Gmail API">

在 Google Cloud 项目中进入 **APIs & Services → Library**，搜索 **Gmail API** 并点击 **Enable**。

</Step>

<Step title="安装扩展">

```bash
ironclaw registry install gmail
```

</Step>

<Step title="授权访问">

```bash
ironclaw tool auth gmail
```

IronClaw 会提供认证链接。请确保已按 [auth setup](./oauth-setup) 完成回调配置。若环境支持，会自动打开浏览器。授权成功后，令牌会被安全保存并自动刷新。

<Tip>
即使已经授权过其他 Google 扩展，也需要对每个新增扩展单独执行一次授权。
</Tip>

</Step>

</Steps>

---

## 可用操作

- `list_messages`: 列出邮件，可附带 Gmail 搜索语法、标签过滤和数量限制
- `get_message`: 按消息 ID 获取完整邮件内容（含头部、正文、标签）
- `send_message`: 发送新邮件，支持收件人、主题、正文和抄送
- `create_draft`: 创建草稿但不发送
- `reply_to_message`: 回复现有线程并保留上下文
- `trash_message`: 将邮件移入垃圾箱

---

## 使用示例

配置后，您可以这样对智能体说：

- _"这周我收到 alice@example.com 的哪些邮件？"_
- _"读取我最新的未读邮件"_
- _"给 bob@example.com 发一封主题为 'Meeting Notes' 的邮件，附上今天讨论摘要"_
- _"给项目提案线程起草一条跟进回复"_
- _"回复发票线程最后一封邮件，告知付款已完成"_
- _"把 noreply@newsletter.com 的邮件都移到垃圾箱"_

---

## Gmail 搜索语法

`list_messages` 的 `query` 字段支持标准 Gmail 查询：

| Query | 匹配内容 |
|---|---|
| `from:alice@example.com` | 来自 Alice 的邮件 |
| `subject:invoice` | 主题含 invoice 的邮件 |
| `is:unread` | 未读邮件 |
| `label:work` | 带 work 标签的邮件 |
| `after:2025/01/01` | 2025-01-01 之后收到的邮件 |
| `has:attachment` | 含附件邮件 |

<Tip>
可以组合查询：`from:alice@example.com is:unread`。
</Tip>