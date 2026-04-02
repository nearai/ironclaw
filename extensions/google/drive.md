---
title: "Google Drive"
description: "Let your agent manage files and folders in Google Drive"
---

The Google Drive extension allows your agent to interact with your Google Drive — listing, searching, uploading, downloading, sharing, and organizing files and folders. It supports both personal Drive and shared drives, making it ideal for file management workflows, automated uploads, and permission management.

---

## Setup

If you haven't set up Google OAuth yet, complete the [Google OAuth Setup](/extensions/google/oauth-setup) first.

<Steps>

<Step title="Enable the Google Drive API">

In your Google Cloud project, navigate to **APIs & Services → Library**, search for **Google Drive API**, and click **Enable**.

</Step>

<Step title="Install the Extension">

```bash
ironclaw registry install google-drive
```

</Step>

<Step title="Authorize Access">

```bash
ironclaw tool auth google-drive
```

IronClaw will provide a URL for you to authenticate - remember to follow the [auth setup](./oauth-setup) to enable your agent to capture the callback. If possible, it will open a browser window. Once approved, the token is stored securely and refreshed automatically.

<Tip>
If you already authenticated one Google service, you still need to authenticate each additional Google extension separately.
</Tip>

</Step>

</Steps>

---

## Available Actions

- `list_files`: List files and folders, with optional search query, MIME type filter, and folder scope
- `get_file`: Retrieve metadata for a specific file (name, type, size, owners, permissions)
- `download_file`: Download the content of a file as text or base64
- `upload_file`: Upload a new file with specified content and MIME type
- `update_file`: Update the content or name of an existing file
- `create_folder`: Create a new folder, optionally inside a parent folder
- `delete_file`: Permanently delete a file or folder
- `trash_file`: Move a file to the trash (recoverable)
- `share_file`: Share a file with a user or group with a specified role (reader/writer/owner)
- `list_permissions`: List all permissions on a file
- `remove_permission`: Remove a specific permission from a file
- `list_shared_drives`: List all shared drives accessible to the account

---

## Example Usage

Once configured, you can ask your agent things like:

- _"List all PDF files in my Drive"_
- _"Upload this report as a file named 'Q2-Report.txt'"_
- _"Download the file named 'budget.csv' from my Drive"_
- _"Create a folder called 'Project Assets' inside my 'Work' folder"_
- _"Share the contract with bob@example.com as a viewer"_
- _"Who has access to my 'Roadmap' document?"_
- _"Move the old proposal to trash"_

---

## Working with Shared Drives

If your Google account has access to shared (team) drives, the agent can target them directly:

<Tip>
Say something like: _"List all files in our Engineering shared drive."_ The agent will use `list_shared_drives` to find the right drive by name before searching for files within it.
</Tip>
