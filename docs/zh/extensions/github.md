---
title: "Github"
description: "让智能体访问 Github"
icon: github
---

Github 扩展允许智能体与 Github 仓库、议题、拉取请求等交互，非常适合自动化代码相关任务、管理项目或从 Github 收集信息。

---

## 设置


<Steps>

<Step title="获取 API 密钥">
要使用 Github 扩展，您需要从 Github 获取个人访问令牌。


</Step>

<Step title="安装 Github 扩展">

在终端中运行以下命令安装 Github 扩展：

```bash
ironclaw registry install github
```

</Step>

<Step title="配置 API 密钥">

安装扩展后，需要在 IronClaw 中配置您的 Github API 密钥。运行：

```bash
ironclaw tool auth github
```

然后按照提示输入您的 API 密钥。

<Warning>
请确保创建细粒度的个人访问令牌，仅授予用例所需的必要权限。如有疑问，选择最小权限选项，之后随时可以创建具有不同权限的新令牌。
</Warning>

</Step>

</Steps>

---

## 可用操作：

以下是智能体使用 Github 扩展可以执行的一些操作：

- `get_repo`：获取仓库信息
- `list_issues`：列出仓库中的所有议题
- `create_issue`：创建新议题
- `get_issue`：获取特定议题的详细信息
- `list_pull_requests`：列出拉取请求
- `get_pull_request`：获取特定拉取请求的详细信息
- `get_pull_request_files`：获取拉取请求中的文件列表
- `create_pr_review`：提交拉取请求审查
- `list_repos`：列出仓库（用户/组织）
- `get_file_content`：获取仓库中文件的内容
- `trigger_workflow`：手动触发 GitHub Actions 工作流
- `get_workflow_runs`：列出最近的工作流运行

---

## 在公共仓库上工作

让我们为智能体配置自己的 Github 账户，以便它可以在**公共仓库**中创建议题和评论拉取请求。


<Steps>

<Step title="创建新的 Github 账户">

前往 https://github.com 为智能体创建新账户。如果您已使用个人账户登录，需要暂时登出以创建新账户，之后可以立即重新登录。

</Step>

<Step title="生成个人访问令牌">

在智能体的 Github 账户上，前往 [Settings -> Developer settings -> Personal access tokens -> Tokens (classic)](https://github.com/settings/tokens) 并生成具有以下权限的新令牌（classic）：`repo` -> `public_repo`

</Step>

<Step title="认证 Github 扩展">
获取令牌后，运行以下命令认证 Github 扩展：

```bash
ironclaw tool auth github
```

然后按照提示输入刚生成的令牌。

</Step>

<Step title="测试一下！">

让智能体在您的某个公共仓库中创建一个测试议题，检查议题是否创建成功。

<Tip>
让智能体阅读 [Github Markdown 指南](https://github.com/adam-p/markdown-here/wiki/markdown-cheatsheet) 并在创建议题和评论时记住这些格式规范，可以让格式更加美观！
</Tip>

</Step>

</Steps>
