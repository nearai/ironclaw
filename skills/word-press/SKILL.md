---
name: word-press
version: "1.0.0"
description: WordPress API — WordPress is a popular open-source content management system (CMS) that enables 
activation:
  keywords:
    - "word-press"
    - "wordpress"
    - "tools"
  patterns:
    - "(?i)word.?press"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
---

# WordPress API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://{WORDPRESS_SITE}/wp-json/wp/v2`

## Actions

**List posts:**
```
http(method="GET", url="https://{WORDPRESS_SITE}/wp-json/wp/v2/posts?per_page=10")
```

**Get post:**
```
http(method="GET", url="https://{WORDPRESS_SITE}/wp-json/wp/v2/posts/{post_id}")
```

**Create post:**
```
http(method="POST", url="https://{WORDPRESS_SITE}/wp-json/wp/v2/posts", body={"title": "My Post","content": "<p>Post content</p>","status": "draft"})
```

**List pages:**
```
http(method="GET", url="https://{WORDPRESS_SITE}/wp-json/wp/v2/pages?per_page=10")
```

**List categories:**
```
http(method="GET", url="https://{WORDPRESS_SITE}/wp-json/wp/v2/categories")
```

**Upload media:**
```
http(method="POST", url="https://{WORDPRESS_SITE}/wp-json/wp/v2/media")
```

## Notes

- Post statuses: `publish`, `draft`, `pending`, `private`, `future`.
- Content uses HTML in `content` field.
- `_embed` query param includes related data (author, featured image).
- Pagination: `page` and `per_page`; check `X-WP-Total` header.
