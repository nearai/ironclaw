---
name: bluesky
version: "1.0.0"
description: Bluesky API — Bluesky is a decentralized social media platform built on the open-source AT Pro
activation:
  keywords:
    - "bluesky"
    - "tools"
  patterns:
    - "(?i)bluesky"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BLUESKY_ACCESS_TOKEN, BLUESKY_ENTRYWAY]
---

# Bluesky API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://bsky.social/xrpc`

## Actions

**Create session (login):**
```
http(method="POST", url="https://bsky.social/xrpc/com.atproto.server.createSession", body={"identifier": "your.handle","password": "app-password"})
```

**Create post:**
```
http(method="POST", url="https://bsky.social/xrpc/com.atproto.repo.createRecord", body={"repo": "did:plc:xxx","collection": "app.bsky.feed.post","record": {"text": "Hello Bluesky!","$type": "app.bsky.feed.post","createdAt": "2026-03-27T00:00:00Z"}})
```

**Get profile:**
```
http(method="GET", url="https://bsky.social/xrpc/app.bsky.actor.getProfile?actor=your.handle")
```

**Get timeline:**
```
http(method="GET", url="https://bsky.social/xrpc/app.bsky.feed.getTimeline?limit=20")
```

**Search posts:**
```
http(method="GET", url="https://bsky.social/xrpc/app.bsky.feed.searchPosts?q=search+term&limit=10")
```

## Notes

- First create a session to get an access token.
- Posts are AT Protocol records in collection `app.bsky.feed.post`.
- Handles look like `username.bsky.social` or custom domains.
- DIDs are persistent identifiers: `did:plc:xxxxx`.
