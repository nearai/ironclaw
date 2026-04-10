---
title: "网页搜索"
description: "让智能体搜索网页"
icon: globe
---

网页搜索工具允许智能体使用 [Brave Search API]() 搜索网页获取最新信息，非常适合回答时事问题、查找特定数据或收集一般信息。

---

## 设置


<Steps>

<Step title="获取 Brave Search API 密钥">
要使用网页搜索工具，您需要从 Brave Search 获取 API 密钥。可以在 https://api-dashboard.search.brave.com 注册获取。

<Info>

截至撰写时，Brave Search API 基础计划每月提供 5 美元免费额度，对于测试和小规模使用完全足够。

</Info>


</Step>

<Step title="安装网页搜索扩展">

在终端中运行以下命令安装网页搜索扩展：

```bash
ironclaw registry install web-search
```

</Step>

<Step title="配置 API 密钥">

安装扩展后，需要在 IronClaw 中配置您的 Brave Search API 密钥。运行：

```bash
ironclaw tool auth web-search
```

然后按照提示输入您的 API 密钥。

</Step>

</Steps>
