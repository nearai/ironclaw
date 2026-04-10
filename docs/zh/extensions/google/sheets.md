---
title: "Google Sheets"
description: "让您的智能体读写 Google 表格"
---

Google Sheets 扩展允许智能体操作电子表格，包括创建表格、读写单元格区间、追加行、格式化单元格和管理工作表。使用标准 A1 表示法，适合数据录入自动化与报表生成。

---

## 设置

如果您还没有完成 Google OAuth，请先完成 [Google OAuth 设置](/zh/extensions/google/oauth-setup)。

<Steps>

<Step title="启用 Google Sheets API">

在 Google Cloud 项目中进入 **APIs & Services → Library**，搜索 **Google Sheets API** 并点击 **Enable**。

</Step>

<Step title="安装扩展">

```bash
ironclaw registry install google-sheets
```

</Step>

<Step title="授权访问">

```bash
ironclaw tool auth google-sheets
```

IronClaw 会提供认证链接。请确保已按 [auth setup](./oauth-setup) 完成回调配置。若环境支持，会自动打开浏览器。授权成功后，令牌会被安全保存并自动刷新。

<Tip>
即使已经授权过其他 Google 扩展，也需要对每个新增扩展单独执行一次授权。
</Tip>

</Step>

</Steps>

---

## 可用操作

- `create_spreadsheet`: 创建新表格，可指定标题与初始工作表名
- `get_spreadsheet`: 获取元数据（标题、工作表名、命名范围）
- `read_values`: 用 A1 表示法读取区间值（例如 `Sheet1!A1:D10`）
- `batch_read_values`: 一次读取多个区间
- `write_values`: 写入区间并覆盖原内容
- `append_values`: 在区间末尾追加新行
- `clear_values`: 清空区间值（保留格式）
- `add_sheet`: 添加新工作表
- `delete_sheet`: 按工作表 ID 删除
- `rename_sheet`: 重命名工作表
- `format_cells`: 为区间设置数值格式、文本样式或背景色

---

## 使用示例

配置后，您可以这样对智能体说：

- _"创建一个名为 Monthly Expenses 的新表格"_
- _"读取预算表 A1 到 E20"_
- _"在 Sales 工作表追加今天销售数据"_
- _"清空 Draft 工作表数据"_
- _"把第一个工作表改名为 Summary"_
- _"把支出表 B 列设置为货币格式"_

---

## A1 表示法

所有区间操作都基于 A1 表示法，可加工作表名指定目标页签：

| Notation | 含义 |
|---|---|
| `A1` | 单个单元格 |
| `A1:C10` | 行列范围 |
| `Sheet1!A1:B5` | 指定工作表范围 |
| `Sheet1!A:A` | Sheet1 的整列 A |

<Tip>
多工作表场景下，建议总是包含工作表名（例如 `Budget!B2:D50`）。
</Tip>