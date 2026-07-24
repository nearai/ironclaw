Use `web_search.search` to search the web for current information via the Brave Search API.

Pass `query` (required) plus any of `count`, `country`, `search_lang`, `ui_lang`, or `freshness` to narrow results. Results include `title`, `url`, `description`, and — when available — `published` date, `thumbnail`, and `site_name`. Follow a result's `url` with a fetch/read tool to get full page content; this capability only returns search result snippets, not page bodies.

This capability reads from the Brave Search API through host HTTP egress and requires a configured `brave_api_key` secret.
