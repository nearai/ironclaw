---
title: "Google Calendar"
description: "让您的智能体管理 Google Calendar"
---

Google Calendar 扩展允许智能体与您的日历交互，包括创建事件、查看安排、更新会议等。适合自动化排程、提醒和会议管理。

---

## 设置

如果您还没有完成 Google OAuth，请先完成 [Google OAuth 设置](/zh/extensions/google/oauth-setup)。

<Steps>

<Step title="启用 Google Calendar API">

在 Google Cloud 项目中进入 **APIs & Services → Library**，搜索 [**Google Calendar API**](https://console.cloud.google.com/marketplace/product/google/calendar-json.googleapis.com?q=search&referrer=search) 并点击 **Enable**。

</Step>

<Step title="安装扩展">

```bash
ironclaw registry install google-calendar
```

</Step>

<Step title="授权访问">

```bash
ironclaw tool auth google-calendar
```

IronClaw 会提供认证链接。请确保已按 [auth setup](./oauth-setup) 完成回调配置。若环境支持，会自动打开浏览器。授权成功后，令牌会被安全保存并自动刷新。

<Tip>
即使已经授权过其他 Google 扩展，也需要对每个新增扩展单独执行一次授权。
</Tip>

</Step>

</Steps>

---

## 可用操作

- `list_calendars`: 列出账号中的所有日历
- `list_events`: 列出日历中的即将发生事件
- `get_event`: 获取指定事件详情
- `create_event`: 创建新事件
- `update_event`: 更新已有事件（标题、时间、描述、参会人）
- `delete_event`: 删除事件
- `find_free_slots`: 跨一个或多个日历查找空闲时间
- `add_attendees`: 向事件添加参会人
- `set_reminder`: 为事件设置提醒

---

## 使用示例

配置后，您可以这样对智能体说：

- _"下周二下午 3 点安排一个 1 小时团队同步会"_
- _"我这周日程是什么？"_
- _"把周五会议改到周一上午"_
- _"帮我和 john@example.com 找这周 30 分钟空档"_
- _"取消我周四下午所有会议"_

---

## 多日历场景

如果账号里有多个日历（个人、工作、共享），可以明确指定目标日历：

<Tip>
例如：_"加到我的 Work 日历，不是个人日历。"_ 智能体会先用 `list_calendars` 按名称定位日历再执行操作。
</Tip>