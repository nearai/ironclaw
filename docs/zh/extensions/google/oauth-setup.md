---
title: "Google OAuth 设置"
description: "IronClaw 中所有 Google 扩展的一次性 OAuth 配置"
---

所有 Google 扩展共用同一套 OAuth 2.0 配置。完成一次后，您可以复用同一个 Google Cloud 项目和凭证。

---

<Steps>

<Step title="创建 Google Cloud 项目">

前往 [Google Cloud Console](https://console.cloud.google.com)，新建项目或选择已有项目。

1. 点击 **Select a project** → **New Project**
2. 输入项目名（例如 `ironclaw`），点击 **Create**

</Step>

<Step title="创建 OAuth 2.0 凭证">

前往 [**Google Auth Platform → Clients**](https://console.cloud.google.com/auth/clients)，创建客户端：

1. 点击 **Create client**
2. 将 **Application type** 设置为 **Web application**
3. 设置名称（例如 `ironclaw`）
4. 在 **Authorized redirect URIs** 中点击 **+ Add URI**，填写：

   ```
   http://127.0.0.1:9876/callback
   ```

5. 点击 **Create**，复制生成的 **Client ID** 与 **Client Secret**

</Step>

<Step title="添加测试用户">

应用处于 **Testing** 模式时，仅已添加的账号可以授权。前往 [**Google Auth Platform → Audience**](https://console.cloud.google.com/auth/audience)，在 **Test users** 中点击 **+ Add users**。

添加将使用扩展的 Google 账号。应用在正式审核前最多支持 100 个测试用户。

<Info>
若出现 “access blocked” 错误，请先确认当前账号已被加入测试用户。
</Info>

</Step>

<Step title="打开 SSH 隧道">

为完成 OAuth 回调，需要让 Google 访问 IronClaw 服务。由于 `9876` 端口仅在服务器内部可访问，您需要将本地端口转发到服务器。

在新终端中执行：

```bash
# ssh -p <SSH-PORT> -L 9876:127.0.0.1:9876 <user>@<ironclaw-server-ip>
ssh -p 15222 -L 9876:127.0.0.1:9876 liquid-zebra@agent4.near.ai
```

在 OAuth 完成前请保持该会话开启。

<Info>
端口转发会在 SSH 会话存活期间持续有效，关闭会话后自动失效。
</Info>

<Tip>
请确保服务器防火墙允许相关端口转发规则。
</Tip>

</Step>

<Step title="设置环境变量">

连接服务器后，导出 OAuth 凭证：

```bash
export GOOGLE_OAUTH_CLIENT_ID=<your-client-id>
export GOOGLE_OAUTH_CLIENT_SECRET=<your-client-secret>
```

</Step>

</Steps>

配置完成后，您可以返回任意 Google 扩展页面继续安装与授权。