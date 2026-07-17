---
title: "Telegram"
description: "通过 Telegram 与智能体交互"
icon: telegram
---

您可以创建 Telegram 机器人并将 IronClaw 智能体连接到它。配置完成后，您可以在私信中与智能体对话，也可以将其添加到群聊中参与讨论。

<Note>
如果您还没有设置智能体，请先查看我们的[快速开始指南](../quickstart)
</Note>

---

## 设置 Telegram 频道

<Steps>

<Step title="使用 BotFather 创建新机器人">

要创建新的 Telegram 机器人，您需要与 [BotFather](https://t.me/botfather) 对话，这是帮助您创建和管理机器人的官方 Telegram 机器人。

<Steps>
    <Step title="与 BotFather 开始对话">
    在 Telegram 应用中搜索"BotFather"并开始对话。您也可以使用此链接：[https://t.me/botfather](https://t.me/botfather)
    </Step>
    <Step title="创建新机器人">
    向 BotFather 发送 `/newbot` 命令，然后按照说明创建新机器人。您需要为机器人选择一个名称和用户名。用户名必须以"bot"结尾，例如"my_agent_bot"。
    </Step>
    <Step title="获取机器人令牌">
    创建机器人后，BotFather 会给您一个类似这样的令牌：`123456789:ABCdefGhIJKlmNoPQRsTUVwxyZ`。此令牌用于认证您的机器人并允许其访问 Telegram API。请妥善保管此令牌，不要与任何人分享。稍后您将需要它来在 IronClaw 中配置 Telegram 频道。
    </Step>
</Steps>

</Step>
<Step title="在 IronClaw 中配置 Telegram 频道">

    使用 `--channels-only` 标志调用 IronClaw CLI 引导向导，仅配置频道而无需再次执行整个引导过程：

    ```
    ironclaw onboard --channels-only
    ```

    <Steps>
        <Step title="配置隧道">
            如果您尚未设置`隧道`，向导会要求您选择隧道提供商并进行设置。我们推荐使用 [ngrok](https://dashboard.ngrok.com/)，因为它易于使用且可靠。

            ![ngrok setup](/images/channels/tunnel.png)
        </Step>
        <Step title="安装 Telegram 频道">
        从可用频道列表中选择 Telegram 频道进行安装。
        ![select channel](/images/channels/telegram-channel.png)
        </Step>
        <Step title="添加机器人令牌">
            输入您在上一步中从 BotFather 获取的机器人令牌。
        </Step>
    </Steps>
</Step>

<Step title="测试 Telegram 频道">
    配置完 Telegram 频道后，是时候测试一下了。如果智能体尚未运行，请先启动 `ironclaw`：

    ```
    ironclaw
    ```

    在 Telegram 中向您的机器人发送一条消息。它会回复一个命令，您需要在终端中执行该命令以完成频道设置：

    ```
    ironclaw pairing approve telegram <PAIRING_CODE>
    ```
</Step>

</Steps>

---

## Telegram 端设置

<Accordion title="隐私模式和群组可见性">
Telegram 机器人默认启用隐私模式，这限制了它们接收的群组消息。如果机器人必须查看所有群组消息，可以：

    - 通过 `/setprivacy` 禁用隐私模式，或
    - 将机器人设为群组管理员。

切换隐私模式后，在每个群组中移除并重新添加机器人，以便 Telegram 应用更改。
</Accordion>
<Accordion title="群组权限">
    管理员状态在 Telegram 群组设置中控制。管理员机器人可接收所有群组消息，适用于需要始终在线的群组行为。
</Accordion>
<Accordion title="实用的 BotFather 设置">
    - `/setjoingroups` 允许/禁止加入群组
    - `/setprivacy` 设置群组可见性行为

</Accordion>

---

## 配置选项

您可以通过 `.ironclaw/channels/telegram.capabilities.json` 文件配置 Telegram 频道的行为，该文件在首次设置频道后自动创建。

<Accordion title="选项概览">

| 选项                          | 值                         | 默认值   | 描述                                                                     |
|---------------------------------|--------------------------------|-----------|---------------------------------------------------------------------------------|
| `dm_policy`                     | `open`, `allowlist`, `pairing` | `pairing` | 控制谁可以向机器人发送私信                                |
| `allow_from`                    | 用户 ID                       | `[]`      | 当 `dm_policy` 设为 `allowlist` 时允许私信机器人的用户              |
| `owner_id`                      | Telegram 用户 ID               | —         | 如果设置，只有此用户可以与机器人交互（私信和群组消息）       |
| `respond_to_all_group_messages` | 布尔值                           | `false`   | 回复所有群组消息                                                   |
| `bot_username`                  | 用户名                       | —         | 当 `respond_to_all_group_messages` 为 `false` 时用于群组提及检测 |
| `polling_enabled`               | 布尔值                           | `false`   | 使用轮询代替 webhook                                                 |
| `poll_interval_ms`              | 数字                         | `30000`   | 轮询间隔（毫秒），仅在 `polling_enabled` 为 `true` 时使用     |
</Accordion>

<Note>

更改配置文件后请记得重启智能体以使更改生效

</Note>

### 私信策略

`dm_policy` 选项控制谁可以向机器人发送私信：

- `open`：任何人都可以无限制地私信机器人
- `allowlist`：只有 `allow_from` 列表中的用户可以私信机器人
- `pairing`**（默认）**：机器人会向联系它的任何用户回复一个配对命令，需要在终端中执行

相关选项：
- `allow_from` 选项是当 `dm_policy` 设为 `allowlist` 时允许私信机器人的 Telegram 用户 ID 列表
- `owner_id` 选项将机器人限制为仅回复特定 Telegram 用户 ID 的消息

<Tip>

**用户 ID**

向 [@userinfobot](https://t.me/userinfobot) 发消息以获取您的 Telegram 用户 ID。

</Tip>

### 回复所有群组消息
默认情况下，Telegram 频道只回复群组中提及机器人的消息。
如果您希望机器人回复所有群组消息，请设置 `respond_to_all_group_messages`

相关选项：
- 如果 `respond_to_all_group_messages` 设为 `false`，机器人只回复提及它的消息。
此时请确保在 `bot_username` 选项中设置机器人的用户名（不带 `@`）

### 轮询

如果您不想配置`隧道`，可以设置 Telegram 频道每隔一定时间轮询新消息。

为此，将 `polling_enabled` 选项设为 `true`，并将 `poll_interval_ms` 选项配置为所需的轮询间隔（毫秒），默认为 30000 毫秒（30 秒）。

### 配置示例

**私人团队助手** — 仅提及触发，私信需配对：
```json
{
  "bot_username": "TeamBot",
  "respond_to_all_group_messages": false,
  "dm_policy": "pairing"
}
```

**全天候专家** — 回复所有消息：
```json
{
  "bot_username": "DevOpsBot",
  "respond_to_all_group_messages": true,
  "allow_from": ["*"]
}
```

**仅限所有者** — 共享群组中的个人助手：
```json
{
  "bot_username": "MyBot",
  "respond_to_all_group_messages": false,
  "owner_id": "12345678"
}
```

---

## 群聊参与

IronClaw 可以配置为参与 Telegram 群聊。默认情况下，机器人只回复命令（使用 `/help` 查看可用命令列表）。如果您希望机器人回复提及或所有群组消息，需要进行配置。

### 将机器人添加到群组

1. **在 @BotFather 中启用群组隐私**：
   - 向 [@BotFather](https://t.me/BotFather) 发消息
   - 发送 `/mybots` → 选择您的机器人
   - 点击"Bot Settings" → "Group Privacy"
   - 关闭"Privacy mode"（允许机器人查看所有消息）

2. **将机器人添加到群组**：
   - 在 Telegram 中打开群组
   - 添加成员 → 搜索您的机器人用户名
   - 授予管理员权限（可选但推荐）

3. **在 IronClaw 中配置 `bot_username`**：
   ```json
   {
     "bot_username": "MyIronClawBot"
   }
   ```

### 群组触发模式

#### 命令和提及

当使用命令（如 `/skills`）或提及机器人（如 `@MyIronClawBot 天气怎么样？`）时，机器人会响应。

配置：

- 在 @BotFather 中将"Privacy mode"设为 `OFF`，或将机器人设为群组管理员
- 配置 `bot_username`：

```json
{
  "bot_username": "MyIronClawBot",
  "respond_to_all_group_messages": false
}
```

优点：
- 尊重群组对话流程
- 不会因未经请求的回复而产生垃圾信息
- 用户明确选择与智能体交互

#### 回复所有消息

机器人处理并回复群组中的每条消息。

- 在 @BotFather 中将"Privacy mode"设为 OFF，或将机器人设为群组管理员
- 同时配置 `bot_username` 和 `respond_to_all_group_messages`：

配置：
```json
{
  "bot_username": "MyIronClawBot",
  "respond_to_all_group_messages": true
}
```

使用场景：
- 智能体始终提供帮助的小型团队房间
- 自动审核或摘要
- 智能体提供专业知识的特定主题群组

---

## 消息隐私

<AccordionGroup>
  <Accordion title="机器人可以看到什么" icon="eye">
    - 禁用隐私模式的群组中的所有消息
    - 用户名和显示名称
    - 消息时间戳
    - 回复链（对话上下文）
  </Accordion>

  <Accordion title="发送给 LLM 的内容" icon="message">
    - 消息文本（已去除 @提及）
    - 发送者标识（用户名或名字）
    - 该对话中的近期对话历史
  </Accordion>

</AccordionGroup>

---

## Webhook 密钥（可选）

当 IronClaw 在 webhook 模式下运行时，Telegram 通过向您的公共 URL 发送 HTTP 请求来传递消息。由于该 URL 可从互联网访问，任何第三方都可以向其发送伪造请求。

Webhook 密钥是您在 IronClaw 中配置的共享令牌。Telegram 在每个请求中包含该令牌。IronClaw 拒绝不携带正确令牌的任何请求，因此只有真正的 Telegram 流量才能到达您的智能体。

要启用此功能，在 `.ironclaw/channels/telegram.capabilities.json` 中添加 `telegram_webhook_secret`：

```json
{
  "telegram_webhook_secret": "your-secret-here"
}
```

生成合适的值：

```bash
openssl rand -hex 16
```

<Note>
Webhook 密钥仅在 `polling_enabled` 为 `false` 时有效。如果您使用轮询，此选项无效。
</Note>

---

## 故障排除

<AccordionGroup>
  <Accordion title="消息未送达">
    **轮询：** 检查日志中的 `getUpdates` 错误，并验证机器人令牌有效。

    **Webhook：** 验证 HTTPS URL 可访问且隧道正在运行。
  </Accordion>

  <Accordion title="配对码未发送">
    - 确保 `dm_policy` 设为 `pairing` 而非 `allowlist`
    - 验证您的实例可以访问 `api.telegram.org`
  </Accordion>

  <Accordion title="群组提及不工作">
    - 确认 `bot_username` 已设置且与机器人用户名完全匹配（不带 `@`）
    - 验证机器人有读取群组消息的权限
  </Accordion>

  <Accordion title="机器人看不到群组消息">
    - 在 @BotFather 中禁用隐私模式：`/mybots` → Bot Settings → Group Privacy → 关闭
    - 更改隐私设置后在群组中移除并重新添加机器人
  </Accordion>

  <Accordion title="机器人意外回复所有群组消息">
    - 将 `respond_to_all_group_messages` 设为 `false`
    - 验证配置已保存并重启智能体
  </Accordion>

  <Accordion title="设置时所有者绑定超时">
    向导等待 120 秒接收第一条消息。如果超时，请在 Telegram 中向您的机器人发送 `/start`，然后重新运行 `ironclaw onboard --channels-only`。
  </Accordion>
</AccordionGroup>
