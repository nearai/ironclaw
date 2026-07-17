---
title: "Google Calendar"
description: "让您的智能体管理 Google Calendar"
---

Google Calendar 扩展允许您的智能体与 Google Calendar 交互，包括创建事件、查看日程、更新预约等。它非常适合自动化排程、设置提醒，或者直接通过智能体管理会议。

---

## 设置

<Steps>

<Step title="创建 Google Cloud Project">

前往 [Google Cloud Console](https://console.cloud.google.com) 创建一个新项目，或者选择一个已有项目。

1. 点击 **Select a project** → **New Project**
2. 为项目命名（例如 `ironclaw-calendar`），然后点击 **Create**

</Step>

<Step title="启用 Google Calendar API">

选择项目后，进入 **APIs & Services → Library**，搜索 **Google Calendar API**，然后点击 **Enable**。

</Step>

<Step title="创建 OAuth 2.0 凭证">

进入 **Google Auth Platform → Clients** 并创建一个新客户端：

1. 点击 **Create client**
2. 将 **Application type** 设为 **Web application**
3. 为客户端命名（例如 `ironclaw-calendar`）
4. 在 **Authorized redirect URIs** 下点击 **+ Add URI**，填入：

   ```
   http://127.0.0.1:9876/callback
   ```

5. 点击 **Create**，然后复制展示出来的 **Client ID** 和 **Client Secret**


</Step>

<Step title="添加测试用户">

由于应用处于 **Testing** 模式，只有被明确添加的用户才能完成授权。前往 **APIs & Services → OAuth consent screen**，向下滚动到 **Test users**，然后点击 **+ Add users**。

把将要使用这个扩展的 Google 账号添加进去（例如 `yourname@gmail.com`）。在应用需要验证之前，最多可以添加 100 个测试用户。

<Info>
当应用仍处于 Testing 模式时，只有测试用户可以完成 OAuth 流程。如果您看到 “access blocked” 错误，请确认当前账号已经被列在这里。
</Info>

</Step>

<Step title="连接到开发服务器">

Google OAuth 回调会在远程服务器的 `9876` 端口上运行。由于该端口并未公开暴露，您需要创建一个 **SSH 隧道**，把本机上的 `localhost:9876` 转发到服务器上的 `127.0.0.1:9876`。这样，当 Google 在授权完成后重定向到 `http://127.0.0.1:9876/callback` 时，请求才能正确到达服务器。

运行以下命令建立隧道：

```bash
ssh -p 15222 -L 9876:127.0.0.1:9876 solid-wolf@agent4.near.ai
```

在使用扩展期间，请保持这个终端会话处于打开状态。

<Info>
`-L 9876:127.0.0.1:9876` 参数就是用来建立隧道的。没有它，OAuth 回调会失败，因为 9876 端口只能从服务器内部访问。
</Info>

</Step>

<Step title="设置环境变量">

使用前一步拿到的 **Client ID** 和 **Client Secret**，在服务器上将它们导出为环境变量：

```bash
export GOOGLE_OAUTH_CLIENT_ID=<your-client-id>
export GOOGLE_OAUTH_CLIENT_SECRET=<your-client-secret>
```

</Step>

<Step title="安装 Google Calendar 扩展">

运行以下命令安装扩展：

```bash
ironclaw registry install google-calendar
```

</Step>

<Step title="配置您的凭证">

向 IronClaw 提供您的 OAuth 凭证：

```bash
ironclaw tool auth google-calendar
```

按照提示粘贴 `credentials.json` 文件内容，或者提供该文件的路径。IronClaw 会为您打开一个浏览器窗口来授权访问日历。授权完成后，token 会被安全存储。

<Info>
授权流程只需要运行一次。之后 IronClaw 会在需要时自动刷新访问 token。
</Info>

</Step>

</Steps>

---

## 可用操作

以下是您的智能体可以通过 Google Calendar 扩展执行的一些操作：

- `list_calendars`：列出您 Google 账号中的所有日历
- `list_events`：列出某个日历中的即将发生事件
- `get_event`：获取某个事件的详细信息
- `create_event`：创建新的日历事件
- `update_event`：更新已有事件（标题、时间、描述、参会人）
- `delete_event`：删除日历事件
- `find_free_slots`：在一个或多个日历中查找空闲时间段
- `add_attendees`：为现有事件添加参会人
- `set_reminder`：为事件设置提醒

---

## 使用示例

配置完成后，您可以对智能体说：

- _“帮我安排一个下周二下午 3 点的一小时团队同步会。”_
- _“我这周的日程是什么？”_
- _“把我周五的会议改到周一上午。”_
- _“帮我和 john@example.com 找一个这周 30 分钟的空闲时间。”_
- _“取消我周四下午的所有会议。”_

---

## 使用多个日历

如果您的 Google 账号下有多个日历（个人、工作、共享等），您可以明确告诉智能体要使用哪一个：

<Tip>
您可以这样说：_“把这件事加到我的 Work 日历，而不是个人日历。”_ 智能体会先用 `list_calendars` 按名称找到对应日历，再去创建事件。
</Tip>