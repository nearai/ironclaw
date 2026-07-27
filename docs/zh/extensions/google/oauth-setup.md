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

   **回环（loopback）流程** —— 浏览器回调到固定的本地端口。适用于在运行 IronClaw 的机器上授权，或通过下文的 SSH 隧道完成授权：

   ```
   http://127.0.0.1:9876/callback
   ```

   **服务端托管流程** —— 浏览器回调到您正在运行的实例。适用于已可通过 HTTPS 访问的部署：

   ```
   https://your-host/api/reborn/product-auth/oauth/google/callback
   ```

   两个都添加也没有问题；如果不确定，这是最省事的做法。

5. 点击 **Create**，复制生成的 **Client ID** 与 **Client Secret**

<Warning>
Google 对重定向 URI 进行**完全匹配** —— 协议、主机、端口、路径都必须一致。不匹配时会在同意页出现之前直接报 `redirect_uri_mismatch`。回环地址用 `http`、托管实例用 `https` 都是正确的，请不要把回环地址改成 HTTPS。
</Warning>

若使用服务端托管流程，请同时告知 IronClaw 相同的值，使两侧保持一致：

```bash
export IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI=https://your-host/api/reborn/product-auth/oauth/google/callback
export IRONCLAW_REBORN_GOOGLE_CLIENT_ID=...
export IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET=...
```

这些也可以写在 `config.toml` 的 `[google]` 小节中（`google.redirect_uri`、`google.client_id`）。客户端密钥应存放在加密的密钥存储中，而不是配置文件里。

</Step>

<Step title="添加测试用户">

应用处于 **Testing** 模式时，仅已添加的账号可以授权。前往 [**Google Auth Platform → Audience**](https://console.cloud.google.com/auth/audience)，在 **Test users** 中点击 **+ Add users**。

添加将使用扩展的 Google 账号。应用在正式审核前最多支持 100 个测试用户。

<Info>
若出现 “access blocked” 错误，请先确认当前账号已被加入测试用户。
</Info>

</Step>

<Step title="打开 SSH 隧道">

回环回调监听在服务器**内部**的 `9876` 端口，而浏览器运行在您本机。SSH 隧道会把本机的 `9876` 端口转发到服务器的回环地址，从而把两者连接起来。

在新终端中执行：

```bash
# ssh -p <SSH-PORT> -L 9876:127.0.0.1:9876 <user>@<ironclaw-server-ip>
ssh -p 15222 -L 9876:127.0.0.1:9876 liquid-zebra@agent4.near.ai
```

在 OAuth 完成前请保持该会话开启。

<Info>
端口转发会在 SSH 会话存活期间持续有效，关闭会话后自动失效。
</Info>

<Warning>
**请勿**在服务器防火墙中开放 `9876` 端口。本地端口转发的流量走的是已建立的 SSH 连接，无需额外放行入站端口；开放该端口只会把 OAuth 回调暴露到公网，没有任何好处。该端口应始终保持仅回环可达。
</Warning>

</Step>

<Step title="将凭据写入 IronClaw">

使用 `ironclaw config` 保存客户端 ID、重定向 URI 与客户端密钥。请在运行 IronClaw 的机器上执行 —— 远程或托管实例请先通过 SSH 登录。

```bash
ironclaw config set google.client_id <your-client-id>
ironclaw config set google.redirect_uri https://<your-instance-host>/api/reborn/product-auth/oauth/google/callback
ironclaw config set google.client_secret
```

<Note>
`google.client_secret` 不接受在命令行上直接传值。它始终以隐藏输入的方式提示录入，因此密钥不会进入 shell 历史或进程列表。客户端 ID 与重定向 URI 不是密钥，可以正常传参。
</Note>

确认已写入的内容：

```bash
ironclaw config get google.client_id
ironclaw config get google.redirect_uri
```

如果您更希望在服务单元或容器中配置，也可以使用环境变量：

```bash
export IRONCLAW_REBORN_GOOGLE_CLIENT_ID=<your-client-id>
export IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET=<your-client-secret>
export IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI=https://<your-instance-host>/api/reborn/product-auth/oauth/google/callback
```

</Step>

<Step title="重启以使配置生效">

`ironclaw config set` 不会自动重启任何进程 —— 它只写入值，并提示：

```
  to apply: ironclaw service restart
```

在重启之前，正在运行的实例仍会使用旧配置，Google OAuth 会持续失败。

<Tabs>
  <Tab title="NEAR AI 托管实例">
    在 NEAR AI 托管实例上，`ironclaw service` 系列命令**无法使用** —— 实例中没有可供其调用的用户级服务管理器，`service restart` 只会报错，并不会真正重启。

    SSH 仅用于执行上面的 `ironclaw config` 命令；执行完成后，请前往 [Agent Dashboard](https://agent.near.ai/) 重启该 agent。这是重启托管实例的唯一方式。
  </Tab>

  <Tab title="自托管">
    ```bash
    ironclaw service restart
    ```

    如果您是在前台运行 `ironclaw serve`，请停止后重新启动。
  </Tab>
</Tabs>

</Step>

</Steps>

配置完成后，您可以返回任意 Google 扩展页面继续安装与授权。