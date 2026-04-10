---
title: "Google Slides"
description: "让您的智能体创建并编辑 Google 演示文稿"
---

Google Slides 扩展允许智能体操作演示文稿，包括创建演示、管理幻灯片、插入和格式化文本、添加形状与图片，以及执行批量更新。适合自动生成汇报材料和持续更新内容。

---

## 设置

如果您还没有完成 Google OAuth，请先完成 [Google OAuth 设置](/zh/extensions/google/oauth-setup)。

<Steps>

<Step title="启用 Google Slides API">

在 Google Cloud 项目中进入 **APIs & Services → Library**，搜索 **Google Slides API** 并点击 **Enable**。

</Step>

<Step title="安装扩展">

```bash
ironclaw registry install google-slides
```

</Step>

<Step title="授权访问">

```bash
ironclaw tool auth google-slides
```

IronClaw 会提供认证链接。请确保已按 [auth setup](./oauth-setup) 完成回调配置。若环境支持，会自动打开浏览器。授权成功后，令牌会被安全保存并自动刷新。

<Tip>
即使已经授权过其他 Google 扩展，也需要对每个新增扩展单独执行一次授权。
</Tip>

</Step>

</Steps>

---

## 可用操作

- `create_presentation`: 创建演示文稿，可指定标题
- `get_presentation`: 获取元数据（标题、页数、元素 ID）
- `get_thumbnail`: 获取指定幻灯片缩略图 URL
- `create_slide`: 在指定位置新增幻灯片，可选布局
- `delete_object`: 按对象 ID 删除幻灯片或页面元素
- `insert_text`: 在文本框或形状的指定位置插入文本
- `delete_text`: 删除文本范围
- `replace_all_text`: 跨全稿查找替换文本
- `create_shape`: 在幻灯片上插入形状（矩形、椭圆、箭头等）
- `insert_image`: 从 URL 插入图片并设置尺寸与位置
- `format_text`: 设置字符样式（粗体、斜体、字号、颜色）
- `format_paragraph`: 设置段落对齐与间距
- `replace_shapes_with_image`: 将匹配标签的形状批量替换为图片
- `batch_update`: 一次 API 调用提交多条更新请求

---

## 使用示例

配置后，您可以这样对智能体说：

- _"创建一个名为 Q3 Roadmap 的新演示文稿"_
- _"新增一页标题为 Annual Review 2025 的封面页"_
- _"把整套幻灯片中的 [COMPANY] 替换成 Acme Corp"_
- _"在第 1 页右上角插入我们的 logo"_
- _"给我第 3 页缩略图预览"_
- _"删除最后两页"_

---

## 对象 ID

Google Slides 中每个元素（幻灯片、文本框、形状、图片）都有唯一对象 ID。执行更新前，可先用 `get_presentation` 获取现有对象 ID。

<Tip>
如果要全稿替换文案，优先用 `replace_all_text`，比逐个元素修改更高效。
</Tip>