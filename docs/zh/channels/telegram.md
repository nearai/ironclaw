---
title: "Telegram"
description: "通过 Telegram 与智能体交互"
icon: telegram
---

将 IronClaw 连接到 Telegram 机器人，即可在私信和群聊中与智能体收发消息。

## 前提条件

- 一个 Telegram 账号
- 一个可通过 HTTPS 从 Telegram 访问到的 IronClaw 实例

---

## 设置

<Steps>

<Step title="创建机器人">

在 Telegram 中与 [BotFather](https://t.me/botfather) 对话并发送 `/newbot`。为机器人选择名称和以 `bot` 结尾的用户名。BotFather 会返回类似 `123456789:ABCdefGhIJKlmNoPQRsTUVwxyZ` 的令牌。

<Warning>
机器人令牌等同于该机器人的完整凭据。持有它的人可以以您的智能体身份读取和发送消息。请勿粘贴到共享频道或提交到代码仓库。
</Warning>

</Step>

<Step title="启动 IronClaw">

```bash
ironclaw serve
```

</Step>

<Step title="连接频道">

在 [Web 界面](/using/webui)中打开 **Extensions**，切换到 **Channels** 标签页，并**向下滚动到 Built-in 面板底部**，在 Telegram 卡片上点击 **Configure**，然后粘贴机器人令牌。

<Warning>
不要在 **Registry** 标签页中对 Telegram 点击 **Configure**。该按钮打开的是配对面板，在机器人尚未配置时只会报错 *"An administrator must configure the Telegram bot first."* —— 这并非权限问题，请改用 Channels 标签页。
</Warning>

令牌会被加密存储，Webhook 也会自动向 Telegram 注册。

Telegram 会将更新投递到：

```
https://your-host/webhooks/extensions/telegram/updates
```

</Step>

<Step title="配对您的账号（每位用户各自完成）">

配置机器人并不等于把**您**连接上去。配对是独立的一步，用于确定哪个 Telegram 用户对应哪个 IronClaw 用户。与您的机器人开始对话，并按照配对提示操作。

配对可以防止陌生人找到您的机器人后，以您的身份与智能体对话。

</Step>

<Step title="开始对话">

向机器人发送私信即可。若要在群聊中使用，请将机器人加入群组 —— 默认情况下 Telegram 机器人只能看到提及自己的消息。

</Step>

</Steps>

---

## 配置

Telegram 在 `config.toml` 中没有任何设置，也没有用于启用的 CLI 配置键。
入口路由已编译进程序并始终挂载；只有在按上述步骤安装 Telegram 扩展并完成
机器人设置之后，它才会开始正常服务，在此之前返回 `503`。

机器人令牌保存在加密的密钥存储中，而不是 `config.toml` 里。参见[配置](/capabilities/configuration)（暂仅提供英文版）。

<Note>
  旧版本遗留的 `[telegram]` 配置段仍可被解析，但不会被读取——`ironclaw serve`
  启动时会记录一条弃用提示。删除该配置段即可消除该提示。
</Note>

---

## 故障排查

<AccordionGroup>

<Accordion title="机器人没有任何回复">
Telegram 通过 Webhook 投递更新，因此您的实例必须可通过 HTTPS 访问并具备有效证书。Telegram 不会向自签名端点或内网地址投递消息。
</Accordion>

<Accordion title="私信可用但群聊不可用">
请直接提及机器人，或通过 BotFather 关闭隐私模式（`/setprivacy`），使其能够看到全部群消息。
</Accordion>

<Accordion title='出现 "An administrator must configure the Telegram bot first"'>
说明实例级的机器人令牌尚未设置，或您是从 Registry 标签页进入了配对面板。请打开 **Extensions → Channels**，滚动到 Built-in 面板底部，在 Telegram 卡片上点击 **Configure**。
</Accordion>

<Accordion title="我让智能体连接 Telegram，它说做不到">
连接频道属于运维操作，有意不交由智能体执行。请按上文步骤在 Web 界面中自行完成。
</Accordion>

<Accordion title="有其他人给我的机器人发消息">
任何知道机器人用户名的人都可以与它对话。未完成配对的用户不会被视为您本人 —— 请完成配对，确保智能体只以您的身份代表您行事。
</Accordion>

</AccordionGroup>
