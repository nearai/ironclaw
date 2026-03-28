---
name: notion
version: "1.0.0"
description: Notion API — pages, databases, blocks, search, comments
activation:
  keywords:
    - "notion"
    - "notion page"
    - "notion database"
    - "notion doc"
  exclude_keywords:
    - "confluence"
    - "google docs"
  patterns:
    - "(?i)notion.*(page|database|block|doc)"
    - "(?i)(create|update|search|query).*notion"
  tags:
    - "productivity"
    - "wiki"
    - "documentation"
  max_context_tokens: 1800
metadata:
  openclaw:
    requires:
      env: [NOTION_API_KEY]
---

# Notion API

Use the `http` tool. Credentials are automatically injected. Always include `Notion-Version` header.

## Base URL

`https://api.notion.com/v1`

Required header on every request:
```
headers=[{"name": "Notion-Version", "value": "2022-06-28"}]
```

## Actions

**Search pages/databases:**
```
http(method="POST", url="https://api.notion.com/v1/search", headers=[{"name": "Notion-Version", "value": "2022-06-28"}], body={"query": "search text", "filter": {"property": "object", "value": "page"}, "page_size": 10})
```

**Get page:**
```
http(method="GET", url="https://api.notion.com/v1/pages/<page_id>", headers=[{"name": "Notion-Version", "value": "2022-06-28"}])
```

**Create page:**
```
http(method="POST", url="https://api.notion.com/v1/pages", headers=[{"name": "Notion-Version", "value": "2022-06-28"}], body={"parent": {"database_id": "<db_id>"}, "properties": {"Name": {"title": [{"text": {"content": "Page Title"}}]}}, "children": [{"object": "block", "type": "paragraph", "paragraph": {"rich_text": [{"type": "text", "text": {"content": "Body text"}}]}}]})
```

**Query database:**
```
http(method="POST", url="https://api.notion.com/v1/databases/<db_id>/query", headers=[{"name": "Notion-Version", "value": "2022-06-28"}], body={"filter": {"property": "Status", "select": {"equals": "In Progress"}}, "page_size": 20})
```

**Update page properties:**
```
http(method="PATCH", url="https://api.notion.com/v1/pages/<page_id>", headers=[{"name": "Notion-Version", "value": "2022-06-28"}], body={"properties": {"Status": {"select": {"name": "Done"}}}})
```

**Get block children (page content):**
```
http(method="GET", url="https://api.notion.com/v1/blocks/<block_id>/children?page_size=100", headers=[{"name": "Notion-Version", "value": "2022-06-28"}])
```

**Append blocks to page:**
```
http(method="PATCH", url="https://api.notion.com/v1/blocks/<page_id>/children", headers=[{"name": "Notion-Version", "value": "2022-06-28"}], body={"children": [{"object": "block", "type": "heading_2", "heading_2": {"rich_text": [{"type": "text", "text": {"content": "New Section"}}]}}]})
```

## Notes

- Page IDs are UUIDs (with or without dashes). Extract from URL: `notion.so/Page-Title-<32hex>`.
- All text is `rich_text` arrays: `[{"type": "text", "text": {"content": "..."}}]`.
- Property types: `title`, `rich_text`, `number`, `select`, `multi_select`, `date`, `checkbox`, `url`, `email`, `phone_number`, `relation`, `people`.
- Database filter operators vary by type: `equals`, `contains`, `greater_than`, `is_not_empty`, etc.
- Pagination: use `start_cursor` from `next_cursor` in response. Check `has_more`.
