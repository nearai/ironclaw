---
title: "Telegram"
description: "通过 Telegram 与智能体交互"
icon: telegram
---

将 IronClaw 连接到 Telegram 机器人，即可在私信和群聊中与智能体收发消息。

## 前提条件

- 一个 Telegram 账号
- 一个可通过公网 HTTPS（含有效证书）从 Telegram 访问到的 IronClaw 实例。本地安装同样可用 —— 在实例前架设一个隧道（例如 [ngrok](https://ngrok.com) 或 [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/)），详见下文的 Webhook 步骤。

---

## 设置

<Steps>

<Step title="创建机器人">

在 Telegram 中与 [BotFather](https://t.me/botfather) 对话并发送 `/newbot`。为机器人选择名称和以 `bot` 结尾的用户名。BotFather 会返回类似 `123456789:ABCdefGhIJKlmNoPQRsTUVwxyZ` 的令牌。请同时记下令牌和用户名 —— 配置表单中两者都需要填写。

<Warning>
机器人令牌等同于该机器人的完整凭据。持有它的人可以以您的智能体身份读取和发送消息。请勿粘贴到共享频道或提交到代码仓库。
</Warning>

</Step>

<Step title="启动 IronClaw">

```bash
ironclaw serve
```

</Step>

<Step title="通过 HTTPS 暴露本地实例（仅本地安装需要）">

如果您的实例已有公网 HTTPS 地址，请跳过此步。否则需要在实例前架设隧道。最省心的方案是 ngrok —— 免费账号会自动获得一个开发域名（名称由系统分配，可在控制台和 agent 输出中看到）：

```bash
brew install ngrok                          # 或参见 ngrok.com/download
ngrok config add-authtoken <your-token>     # 令牌来自 dashboard.ngrok.com
ngrok http 3000
```

请使用 IronClaw 实际监听的端口 —— 未修改 `listen_port` 时为 `3000`。复制 ngrok 输出的 HTTPS 地址（形如 `https://<assigned-name>.ngrok-free.app`），拼接 Webhook 路径，即为下一步要填写的 Webhook URL：

```
https://<assigned-name>.ngrok-free.app/webhooks/extensions/telegram/updates
```

如果隧道的主机名发生变化（随机的 `trycloudflare.com` 快速隧道，或更换了保留域名），需要更新 **Public webhook URL** 字段并重新保存 —— 保存后会自动向 Telegram 重新注册 Webhook。ngrok 付费套餐可通过 `ngrok http --url=https://<your-domain> 3000` 固定保留域名。

<Note>
其他选择：已使用 Tailscale 的话，`tailscale funnel 3000` 可提供稳定的 `https://<machine>.<tailnet>.ts.net` 地址；`cloudflared tunnel --url http://localhost:3000` 无需注册，适合快速测试（每次启动主机名随机）。注意隧道会暴露整个 IronClaw 实例，而不仅是 Webhook 路径 —— WebUI 仍需访问令牌，Telegram 路由也会拒绝缺少 Webhook 密钥的调用，但请勿将主机名扩散给不必要的人。
</Note>

</Step>

<Step title="填写部署配置（运维人员，每个实例一次）">

在 [Web 界面](/using/webui)中打开 **Admin → Configuration**，找到 **Telegram deployment configuration** 卡片。它包含四个必填字段：

| 字段 | 填写内容 |
| --- | --- |
| **Bot token** | 第 1 步中 BotFather 签发的令牌。 |
| **Webhook secret token** | 由**您自行生成**的值 —— 任意 1–256 位的字母、数字、`_` 或 `-` 组成的随机字符串（例如 `openssl rand -hex 32` 的输出）。IronClaw 会将其注册到 Telegram，Telegram 在每次投递时原样带回，用于拒绝伪造的 Webhook 调用。 |
| **Public webhook URL** | Telegram 投递更新的完整 HTTPS 地址：`https://your-host/webhooks/extensions/telegram/updates`。本地安装请使用隧道的公网主机名。 |
| **Bot username** | 机器人的用户名，**不含开头的 @**（5–32 个字符，以 `bot` 结尾）。用于配对深链接和群聊提及。 |

保存配置。这些值会保存在加密的密钥存储中，Webhook 会在扩展激活时向 Telegram 注册。

<Note>
Telegram 只向可公网访问、证书有效、端口为 443、80、88 或 8443 的 HTTPS 端点投递 Webhook，不会向自签名端点或内网地址投递 —— 这就是本地安装需要隧道的原因。
</Note>

</Step>

<Step title="安装扩展">

打开 **Extensions**，找到 **Telegram** 并安装 —— 也可以直接在聊天中让智能体设置 Telegram。激活时会使用您保存的配置向 Telegram 注册 Webhook；若四个字段中有任何缺失或无效，激活会以明确的错误失败。

</Step>

<Step title="配对您的账号（每位用户各自完成）">

配置机器人并不等于把**您**连接上去。配对是独立的一步，用于确定哪个 Telegram 用户对应哪个 IronClaw 用户。在 Web 界面中打开配对面板，通过其链接或二维码完成配对；也可以与机器人开始对话，发送 `/start` 加上面板显示的配对码。

配对可以防止陌生人找到您的机器人后，以您的身份与智能体对话。

</Step>

<Step title="开始对话">

向机器人发送私信即可。若要在群聊中使用，请将机器人加入群组 —— 默认情况下 Telegram 机器人只能看到提及自己的消息。

</Step>

</Steps>

---

## 配置

Telegram 在 `config.toml` 中没有任何设置，也没有用于启用的 CLI 配置键。
入口路由已编译进程序并始终挂载；只有在保存部署配置并激活 Telegram 扩展
之后（见上文步骤），它才会开始正常服务，在此之前返回 `503`。

机器人令牌和 Webhook 密钥保存在加密的密钥存储中，而不是 `config.toml` 里。参见[配置](/capabilities/configuration)（暂仅提供英文版）。

<Note>
  旧版本遗留的 `[telegram]` 配置段仍可被解析，但不会被读取——`ironclaw serve`
  启动时会记录一条弃用提示。删除该配置段即可消除该提示。
</Note>

---

## 故障排查

<AccordionGroup>

<Accordion title="机器人没有任何回复">
Telegram 通过 Webhook 投递更新，因此您的实例必须可通过 HTTPS 访问、证书有效，且端口为 443、80、88 或 8443。Telegram 不会向自签名端点或内网地址投递消息。本地安装请确认隧道正在运行，且 **Public webhook URL** 字段与隧道当前的公网主机名一致 —— 免费隧道的主机名在重启后经常会变化。
</Accordion>

<Accordion title="私信可用但群聊不可用">
请直接提及机器人，或通过 BotFather 关闭隐私模式（`/setprivacy`），使其能够看到全部群消息。
</Accordion>

<Accordion title='出现 "An administrator must configure the Telegram bot first"'>
说明实例级的部署配置尚未保存，或您在保存之前进入了配对面板。请打开 **Admin → Configuration**，在 **Telegram deployment configuration** 卡片中填写全部四个字段并保存。
</Accordion>

<Accordion title="保存配置后激活失败">
激活会向 Telegram 注册 Webhook，任何取值错误都会导致激活直接失败。请检查：机器人令牌与 BotFather 签发的完全一致；Webhook 密钥只包含字母、数字、`_` 或 `-`；Public webhook URL 是完整的 `https://…/webhooks/extensions/telegram/updates` 地址；机器人用户名不含开头的 `@` 且以 `bot` 结尾。
</Accordion>

<Accordion title="我让智能体连接 Telegram，它说做不到">
运维部分 —— 填写部署配置 —— 有意不交由智能体执行，请按上文步骤在 Web 界面中自行完成。配置保存后，让智能体处理属于您个人的部分是可行的：它会安装并激活扩展，并弹出配对面板供您绑定 Telegram 账号。
</Accordion>

<Accordion title="有其他人给我的机器人发消息">
任何知道机器人用户名的人都可以与它对话。未完成配对的用户不会被视为您本人 —— 请完成配对，确保智能体只以您的身份代表您行事。
</Accordion>

</AccordionGroup>
