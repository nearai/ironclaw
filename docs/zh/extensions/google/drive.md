---
title: "Google Drive"
description: "让您的智能体管理 Google Drive 文件与文件夹"
---

Google Drive 扩展允许智能体操作云端文件，包括列出、搜索、上传、下载、共享和组织文件夹。支持个人盘与共享盘，适合自动化文件流转和权限管理。

---

## 设置

如果您还没有完成 Google OAuth，请先完成 [Google OAuth 设置](/zh/extensions/google/oauth-setup)。

<Steps>

<Step title="启用 Google Drive API">

在 Google Cloud 项目中进入 **APIs & Services → Library**，搜索 **Google Drive API** 并点击 **Enable**。

</Step>

<Step title="安装扩展">

```bash
ironclaw registry install google-drive
```

</Step>

<Step title="授权访问">

```bash
ironclaw tool auth google-drive
```

IronClaw 会提供认证链接。请确保已按 [auth setup](./oauth-setup) 完成回调配置。若环境支持，会自动打开浏览器。授权成功后，令牌会被安全保存并自动刷新。

<Tip>
即使已经授权过其他 Google 扩展，也需要对每个新增扩展单独执行一次授权。
</Tip>

</Step>

</Steps>

---

## 可用操作

- `list_files`: 列出文件与文件夹，可加搜索语句、MIME 类型过滤、目录范围
- `get_file`: 获取文件元数据（名称、类型、大小、所有者、权限）
- `download_file`: 以文本或 base64 下载文件内容
- `upload_file`: 上传新文件并指定内容与 MIME 类型
- `update_file`: 更新已有文件内容或名称
- `create_folder`: 创建文件夹，可指定父目录
- `delete_file`: 永久删除文件或文件夹
- `trash_file`: 将文件移入回收站（可恢复）
- `share_file`: 按角色（reader/writer/owner）共享给用户或群组
- `list_permissions`: 列出文件全部权限
- `remove_permission`: 删除指定权限项
- `list_shared_drives`: 列出账号可访问的共享盘

---

## 使用示例

配置后，您可以这样对智能体说：

- _"列出我 Drive 里所有 PDF"_
- _"把这份报告上传为 Q2-Report.txt"_
- _"下载我 Drive 里的 budget.csv"_
- _"在 Work 文件夹里创建 Project Assets 文件夹"_
- _"把合同以可查看权限共享给 bob@example.com"_
- _"谁可以访问我的 Roadmap 文档？"_
- _"把旧提案移到回收站"_

---

## 共享盘场景

如果账号可访问共享盘（团队盘），可以直接指定目标共享盘：

<Tip>
例如：_"列出 Engineering 共享盘里的所有文件。"_ 智能体会先用 `list_shared_drives` 按名称匹配再继续检索。
</Tip>