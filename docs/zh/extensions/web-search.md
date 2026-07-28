---
title: "网页访问"
sidebarTitle: "网页搜索"
description: "让智能体搜索网页并读取页面内容"
icon: globe
---

网页访问扩展让智能体搜索网页并获取完整页面内容，因此它可以回答时事问题、查找特定数据并引用来源。

该扩展无需任何凭据。无需申请 API 密钥，也无需注册账户——安装、激活，工具即可使用。

---

## 设置

<Steps>

<Step title="安装扩展">

```bash
ironclaw extension install web-access
```

<Note>
扩展 id 是 `web-access`，不是 `web-search`。运行 `ironclaw extension search web` 可确认当前安装所识别的 id。
</Note>

</Step>

<Step title="激活扩展">

```bash
ironclaw extension activate web-access
```

激活会将工具发布给智能体。由于不涉及任何凭据，激活后无需再提供任何信息。

</Step>

</Steps>

您也可以在 [Web 界面](/using/webui)的 **Extensions** 中完成这两个步骤。

---

## 智能体能做什么

| 能力 | 说明 |
| --- | --- |
| `web-access.search` | 搜索网页并返回带引用来源的结果 |
| `web-access.get_content` | 获取页面完整内容，或重新读取先前搜索缓存的页面 |

扩展激活后，两者默认均被允许。

<Tip>
对于 GitHub 仓库、issue、pull request、release 或工作流数据，[GitHub 扩展](/zh/extensions/github)返回的结果比网页搜索结构化程度更高。如果智能体经常处理 GitHub，建议安装该扩展。
</Tip>
