---
title: "Google Docs"
description: "让您的智能体创建并编辑 Google 文档"
---

Google Docs 扩展允许智能体操作 Google 文档，包括创建文档、读取内容、插入与格式化文本、管理表格与列表、执行批量更新。适合报告起草、内容编辑与文档流程自动化。

---

## 设置

如果您还没有完成 Google OAuth，请先完成 [Google OAuth 设置](/zh/extensions/google/oauth-setup)。

<Steps>

<Step title="启用 Google Docs API">

在 Google Cloud 项目中进入 **APIs & Services → Library**，搜索 **Google Docs API** 并点击 **Enable**。

</Step>

<Step title="安装扩展">

```bash
ironclaw registry install google-docs
```

</Step>

<Step title="授权访问">

```bash
ironclaw tool auth google-docs
```

IronClaw 会提供认证链接。请确保已按 [auth setup](./oauth-setup) 完成回调配置。若环境支持，会自动打开浏览器。授权成功后，令牌会被安全保存并自动刷新。

<Tip>
即使已经授权过其他 Google 扩展，也需要对每个新增扩展单独执行一次授权。
</Tip>

</Step>

</Steps>

---

## 可用操作

- `create_document`: 创建新文档，可指定标题
- `get_document`: 获取文档元数据（标题、修订、命名范围）
- `read_content`: 提取文档纯文本或结构化内容
- `insert_text`: 在指定索引插入文本
- `delete_content`: 按起止索引删除内容
- `replace_text`: 全文查找替换
- `format_text`: 对文本范围应用字符样式（粗体、斜体、字号、颜色）
- `format_paragraph`: 对段落应用样式（标题级别、对齐、间距、缩进）
- `insert_table`: 插入指定行列数表格
- `create_list`: 将段落范围转换为有序或无序列表
- `batch_update`: 一次 API 调用提交多条更新请求

---

## 使用示例

配置后，您可以这样对智能体说：

- _"创建一个名为 'Q2 Marketing Plan' 的文档"_
- _"读取文档 ID 1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgVE2upms 的内容"_
- _"在报告顶部插入一段摘要"_
- _"把文档里所有 TBD 替换成 Pending Review"_
- _"把标题设为 Heading 1 并加粗"_
- _"新增一个 3 列预算拆分表格"_

---

## 文档 ID

Google 文档 ID 位于 URL 中：

```
https://docs.google.com/document/d/<DOCUMENT_ID>/edit
```

<Tip>
您可以直接把完整链接发给智能体，智能体会自动提取文档 ID。
</Tip>